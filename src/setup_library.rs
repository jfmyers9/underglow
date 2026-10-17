//! Bounded, declarative saved compositions. IDs and names are data, never paths.
//! Starters are copied only on an explicit create operation; reopening does not
//! consult templates, restore deleted layers, or change connection configuration.
use crate::effects::EffectKind;
use crate::setup::{Activation, Layer, Renderer, SetupDocument, Target, TargetZone};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

pub const MAX_SETUPS: usize = 64;
pub const MAX_LIBRARY_BYTES: usize = 768 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupLibrary {
    pub schema_version: u32,
    pub revision: u64,
    pub documents: Vec<SetupDocument>,
}

impl Default for SetupLibrary {
    fn default() -> Self {
        Self {
            schema_version: 1,
            revision: 0,
            documents: vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetupSummary {
    pub id: String,
    pub name: String,
    pub layers: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum Starter {
    Blank,
    Aurora,
    MatrixPi,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LibraryMutation {
    Store {
        document: SetupDocument,
    },
    Create {
        starter: Starter,
        id: String,
        name: String,
    },
    Duplicate {
        id: String,
        new_id: String,
        name: String,
    },
    Rename {
        id: String,
        name: String,
    },
    Delete {
        id: String,
    },
    Switch {
        id: String,
        expected_setup_revision: u64,
    },
}

/// Engine revision is an optimistic concurrency token, not user-authored content.
pub fn same_content(left: &SetupDocument, right: &SetupDocument) -> bool {
    let mut left = left.clone();
    left.revision = right.revision;
    &left == right
}

pub fn starter(starter: Starter, id: String, name: String) -> Result<SetupDocument, String> {
    let mut document = SetupDocument {
        schema_version: crate::setup::SCHEMA_VERSION,
        revision: 0,
        id,
        name,
        layers: vec![],
        groups: Default::default(),
    };
    if starter != Starter::Blank {
        document.layers.push(Layer {
            opacity: 255,
            blend: Default::default(),
            id: "background".into(),
            name: "Background".into(),
            enabled: true,
            renderer: Renderer::Effect {
                effect: if starter == Starter::MatrixPi {
                    EffectKind::Matrix
                } else {
                    EffectKind::Aurora
                },
                palette: None,
                speed: crate::animation::DEFAULT_SPEED,
            },
            brightness: crate::DEFAULT_BRIGHTNESS,
            target: Target::Whole {},
            activation: Activation::Always {},
            variants: vec![],
        });
    }
    if starter == Starter::MatrixPi {
        document.layers.push(crate::templates::pi_status_layer(
            "pi-status".into(),
            Target::Selection {
                zones: vec![TargetZone::Function],
                keys: vec![],
                groups: vec![],
            },
            None,
        ));
    }
    document.validate().map_err(|error| error.to_string())?;
    Ok(document)
}

impl SetupLibrary {
    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 1 {
            return Err("unsupported setup library version".into());
        }
        if self.documents.len() > MAX_SETUPS {
            return Err("setup library exceeds 64 documents".into());
        }
        let mut ids = BTreeSet::new();
        for document in &self.documents {
            document.validate().map_err(|error| error.to_string())?;
            if serde_json::to_vec(document)
                .map_err(|error| error.to_string())?
                .len()
                > crate::setup::MAX_DOCUMENT_BYTES
            {
                return Err("saved setup exceeds 256 KiB".into());
            }
            if !ids.insert(&document.id) {
                return Err("duplicate setup document ID".into());
            }
        }
        if serde_json::to_vec(self)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_LIBRARY_BYTES
        {
            return Err("setup library exceeds 768 KiB".into());
        }
        Ok(())
    }

    pub fn get(&self, id: &str) -> Result<&SetupDocument, String> {
        self.documents
            .iter()
            .find(|document| document.id == id)
            .ok_or_else(|| "unknown saved setup".into())
    }

    pub fn summaries(&self) -> Vec<SetupSummary> {
        self.documents
            .iter()
            .map(|document| SetupSummary {
                id: document.id.clone(),
                name: document.name.clone(),
                layers: document.layers.len(),
            })
            .collect()
    }

    /// Returns a validated candidate; a rejected edit never changes the original.
    /// Active-document safety and physical activation belong to the engine host.
    pub fn update(
        &self,
        expected_revision: u64,
        mutation: &LibraryMutation,
    ) -> Result<Self, String> {
        if self.revision != expected_revision {
            return Err("setup library revision conflict; refresh and retry".into());
        }
        let mut next = self.clone();
        match mutation {
            LibraryMutation::Store { document } => next.upsert(document.clone()),
            LibraryMutation::Create {
                starter: template,
                id,
                name,
            } => {
                if next.get(id).is_ok() {
                    return Err("setup ID already exists".into());
                }
                next.documents
                    .push(starter(*template, id.clone(), name.clone())?);
            }
            LibraryMutation::Duplicate { id, new_id, name } => {
                if next.get(new_id).is_ok() {
                    return Err("duplicate requires a new setup ID".into());
                }
                let mut document = next.get(id)?.clone();
                document.id = new_id.clone();
                document.name = name.clone();
                document.revision = 0;
                next.documents.push(document);
            }
            LibraryMutation::Rename { id, name } => {
                let mut document = next.get(id)?.clone();
                document.name = name.clone();
                next.upsert(document);
            }
            LibraryMutation::Delete { id } => {
                next.get(id)?;
                next.documents.retain(|document| &document.id != id);
            }
            LibraryMutation::Switch { id, .. } => {
                next.get(id)?;
            }
        }
        next.revision = next
            .revision
            .checked_add(1)
            .ok_or("setup library revision exhausted")?;
        next.validate()?;
        Ok(next)
    }

    fn upsert(&mut self, document: SetupDocument) {
        if let Some(existing) = self
            .documents
            .iter_mut()
            .find(|existing| existing.id == document.id)
        {
            *existing = document;
        } else {
            self.documents.push(document);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_preserves_layer_ids_references_and_deliberate_edits() {
        let mut original = starter(Starter::MatrixPi, "daily".into(), "Everyday".into()).unwrap();
        if let Activation::WhileState { instance, .. } = &mut original.layers[1].activation {
            *instance = Some("work/session".into());
        }
        let library = SetupLibrary::default()
            .update(
                0,
                &LibraryMutation::Store {
                    document: original.clone(),
                },
            )
            .unwrap();
        let mut duplicated = library
            .update(
                1,
                &LibraryMutation::Duplicate {
                    id: "daily".into(),
                    new_id: "custom/one".into(),
                    name: "Mine".into(),
                },
            )
            .unwrap();
        let copy = duplicated.get("custom/one").unwrap();
        assert_eq!(copy.layers, original.layers);
        let mut edited = copy.clone();
        edited.layers.clear();
        duplicated = duplicated
            .update(
                2,
                &LibraryMutation::Store {
                    document: edited.clone(),
                },
            )
            .unwrap();
        let reopened: SetupLibrary =
            serde_json::from_slice(&serde_json::to_vec(&duplicated).unwrap()).unwrap();
        assert_eq!(reopened.get("daily").unwrap(), &original);
        assert_eq!(reopened.get("custom/one").unwrap(), &edited);
        assert!(reopened.get("custom/one").unwrap().layers.is_empty());
        assert_eq!(
            starter(Starter::MatrixPi, "another".into(), "New".into())
                .unwrap()
                .layers
                .len(),
            2
        );
        assert!(
            reopened
                .update(0, &LibraryMutation::Delete { id: "daily".into() })
                .is_err()
        );
    }

    #[test]
    fn starters_and_validation_are_bounded_declarative_data() {
        for template in [Starter::Blank, Starter::Aurora, Starter::MatrixPi] {
            let document = starter(template, "safe/../id".into(), "Editable".into()).unwrap();
            crate::layer_runtime::CompiledSetup::compile(&document).unwrap();
            let mut revised = document.clone();
            revised.revision = 123;
            assert!(same_content(&document, &revised));
        }
        let doc = starter(Starter::Blank, "id".into(), "Blank".into()).unwrap();
        assert!(
            SetupLibrary {
                documents: vec![doc.clone(), doc.clone()],
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            SetupLibrary::default()
                .update(
                    0,
                    &LibraryMutation::Duplicate {
                        id: "missing".into(),
                        new_id: "copy".into(),
                        name: "Copy".into()
                    }
                )
                .is_err()
        );
        let mut too_many = SetupLibrary::default();
        for i in 0..=MAX_SETUPS {
            let mut entry = doc.clone();
            entry.id = format!("id-{i}");
            too_many.documents.push(entry);
        }
        assert!(too_many.validate().is_err());
        assert!(
            SetupLibrary {
                schema_version: 2,
                ..Default::default()
            }
            .validate()
            .is_err()
        );
        assert!(
            serde_json::from_value::<LibraryMutation>(
                serde_json::json!({"op":"store", "document":doc, "command":["never-run"]})
            )
            .is_err()
        );
    }

    #[test]
    fn rename_delete_and_byte_budget_leave_other_entries_intact() {
        let library = SetupLibrary::default()
            .update(
                0,
                &LibraryMutation::Create {
                    starter: Starter::Aurora,
                    id: "daily".into(),
                    name: "Everyday".into(),
                },
            )
            .unwrap();
        let renamed = library
            .update(
                1,
                &LibraryMutation::Rename {
                    id: "daily".into(),
                    name: "Named by user".into(),
                },
            )
            .unwrap();
        assert_eq!(
            renamed.get("daily").unwrap().layers,
            library.get("daily").unwrap().layers
        );
        assert_eq!(renamed.get("daily").unwrap().name, "Named by user");
        let empty = renamed
            .update(2, &LibraryMutation::Delete { id: "daily".into() })
            .unwrap();
        assert!(empty.documents.is_empty());
        let mut document = library.get("daily").unwrap().clone();
        let layer = document.layers[0].clone();
        document.layers = (0..crate::setup::MAX_LAYERS)
            .map(|index| {
                let mut layer = layer.clone();
                layer.id = format!("layer-{index}");
                layer.name = "x".repeat(128);
                layer
            })
            .collect();
        document.validate().unwrap();
        let oversized = SetupLibrary {
            documents: (0..MAX_SETUPS)
                .map(|index| {
                    let mut doc = document.clone();
                    doc.id = format!("doc-{index}");
                    doc
                })
                .collect(),
            ..Default::default()
        };
        assert!(serde_json::to_vec(&oversized).unwrap().len() > MAX_LIBRARY_BYTES);
        assert!(oversized.validate().unwrap_err().contains("768 KiB"));
    }
}
