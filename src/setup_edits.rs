//! Pure optimistic setup edits. No persistence, hardware, or producer lifecycle.
use crate::setup::{Activation, Layer, MAX_DOCUMENT_BYTES, Renderer, SetupDocument, Target};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LayerPatch {
    pub name: Option<String>,
    pub renderer: Option<Renderer>,
    pub brightness: Option<u8>,
    pub target: Option<Target>,
    pub activation: Option<Activation>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "kebab-case", deny_unknown_fields)]
pub enum LayerEdit {
    /// Insert at a zero-based bottom-to-top index (len appends on top).
    Create {
        layer: Layer,
        index: usize,
    },
    Update {
        id: String,
        patch: LayerPatch,
    },
    Remove {
        id: String,
    },
    /// The caller supplies a fresh stable ID; no implicit random identity.
    Duplicate {
        id: String,
        new_id: String,
        index: usize,
    },
    Enable {
        id: String,
        enabled: bool,
    },
    /// A complete permutation of stable IDs, bottom to top.
    Reorder {
        ids: Vec<String>,
    },
}

fn next_revision(current: Option<&SetupDocument>, expected: u64) -> Result<u64, String> {
    let actual = current.map_or(0, |doc| doc.revision);
    if actual != expected {
        return Err(format!(
            "stale setup revision: expected {expected}, current {actual}"
        ));
    }
    actual
        .checked_add(1)
        .ok_or_else(|| "setup revision exhausted".into())
}

fn bounded(doc: &SetupDocument) -> Result<(), String> {
    doc.validate()?;
    if serde_json::to_vec(doc).map_err(|e| e.to_string())?.len() > MAX_DOCUMENT_BYTES {
        return Err("setup exceeds 256 KiB".into());
    }
    Ok(())
}

/// Build and validate a candidate without mutating the input. Compilation and
/// actual-device checks must also succeed before the engine persists/adopts it.
pub fn apply_edits(
    current: &SetupDocument,
    expected_revision: u64,
    edits: &[LayerEdit],
) -> Result<SetupDocument, String> {
    let revision = next_revision(Some(current), expected_revision)?;
    if edits.is_empty() || edits.len() > 128 {
        return Err("an edit transaction requires 1..=128 operations".into());
    }
    let mut candidate = current.clone();
    for edit in edits {
        let position = |id: &str, doc: &SetupDocument| {
            doc.layers
                .iter()
                .position(|layer| layer.id == id)
                .ok_or_else(|| format!("unknown layer ID {id:?}"))
        };
        match edit {
            LayerEdit::Create { layer, index } => {
                insert(&mut candidate, layer.clone(), *index)?;
            }
            LayerEdit::Update { id, patch } => {
                let index = position(id, &candidate)?;
                let layer = &mut candidate.layers[index];
                if let Some(value) = &patch.name {
                    layer.name = value.clone();
                }
                if let Some(value) = &patch.renderer {
                    layer.renderer = value.clone();
                }
                if let Some(value) = patch.brightness {
                    layer.brightness = value;
                }
                if let Some(value) = &patch.target {
                    layer.target = value.clone();
                }
                if let Some(value) = &patch.activation {
                    layer.activation = value.clone();
                }
            }
            LayerEdit::Remove { id } => {
                let index = position(id, &candidate)?;
                candidate.layers.remove(index);
            }
            LayerEdit::Duplicate { id, new_id, index } => {
                let mut layer = candidate.layers[position(id, &candidate)?].clone();
                layer.id = new_id.clone();
                insert(&mut candidate, layer, *index)?;
            }
            LayerEdit::Enable { id, enabled } => {
                let index = position(id, &candidate)?;
                candidate.layers[index].enabled = *enabled;
            }
            LayerEdit::Reorder { ids } => {
                let unique: BTreeSet<_> = ids.iter().collect();
                if ids.len() != candidate.layers.len() || unique.len() != ids.len() {
                    return Err(
                        "reorder requires every layer ID exactly once, bottom to top".into(),
                    );
                }
                candidate.layers = ids
                    .iter()
                    .map(|id| position(id, &candidate).map(|index| candidate.layers[index].clone()))
                    .collect::<Result<_, _>>()?;
            }
        }
    }
    candidate.revision = revision;
    bounded(&candidate)?;
    Ok(candidate)
}

fn insert(doc: &mut SetupDocument, layer: Layer, index: usize) -> Result<(), String> {
    if index > doc.layers.len() {
        return Err("layer index exceeds stack length".into());
    }
    if doc.layers.iter().any(|existing| existing.id == layer.id) {
        return Err(format!("layer ID {:?} already exists", layer.id));
    }
    if doc.layers.len() >= crate::setup::MAX_LAYERS {
        return Err("at most 64 layers are supported".into());
    }
    doc.layers.insert(index, layer);
    Ok(())
}

/// Complete declarative draft replacement, never an outer configuration or
/// executable trust replacement. Its revision is engine-assigned, not imported.
/// Legacy configurations have no setup and use expected_revision zero.
pub fn replace_setup(
    current: Option<&SetupDocument>,
    expected_revision: u64,
    draft: &SetupDocument,
) -> Result<SetupDocument, String> {
    let mut candidate = draft.clone();
    candidate.revision = next_revision(current, expected_revision)?;
    bounded(&candidate)?;
    Ok(candidate)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::effects::EffectKind;

    fn doc() -> SetupDocument {
        SetupDocument::from_toml(include_str!("../examples/layer-setup.toml")).unwrap()
    }

    #[test]
    fn renderer_patch_preserves_every_other_field_and_layer() {
        let original = doc();
        let mut expected = original.clone();
        let renderer = Renderer::Effect {
            effect: EffectKind::Aurora,
            palette: None,
            speed: 120,
        };
        expected.layers[0].renderer = renderer.clone();
        expected.revision += 1;
        let candidate = apply_edits(
            &original,
            0,
            &[LayerEdit::Update {
                id: original.layers[0].id.clone(),
                patch: LayerPatch {
                    renderer: Some(renderer),
                    ..Default::default()
                },
            }],
        )
        .unwrap();
        assert_eq!(candidate, expected);
        assert_eq!(original, doc());
    }

    #[test]
    fn stable_id_operations_and_bottom_to_top_order() {
        let original = doc();
        let id = original.layers[0].id.clone();
        let mut added = original.layers[0].clone();
        added.id = "new".into();
        let candidate = apply_edits(
            &original,
            0,
            &[
                LayerEdit::Create {
                    layer: added,
                    index: 0,
                },
                LayerEdit::Duplicate {
                    id: id.clone(),
                    new_id: "copy".into(),
                    index: 1,
                },
                LayerEdit::Enable {
                    id: "copy".into(),
                    enabled: false,
                },
                LayerEdit::Remove { id: "new".into() },
                LayerEdit::Reorder {
                    ids: vec![original.layers[1].id.clone(), id, "copy".into()],
                },
            ],
        )
        .unwrap();
        assert_eq!(candidate.layers[0], original.layers[1]);
        assert_eq!(candidate.layers[1], original.layers[0]);
        assert_eq!(candidate.layers[2].id, "copy");
        assert!(!candidate.layers[2].enabled);
        assert_eq!(candidate.revision, 1);
    }

    #[test]
    fn rejects_stale_unknown_invalid_and_identity_changes_without_mutation() {
        let original = doc();
        assert!(
            apply_edits(
                &original,
                1,
                &[LayerEdit::Remove {
                    id: "missing".into()
                }]
            )
            .unwrap_err()
            .contains("stale")
        );
        for edit in [
            LayerEdit::Remove {
                id: "missing".into(),
            },
            LayerEdit::Duplicate {
                id: original.layers[0].id.clone(),
                new_id: original.layers[1].id.clone(),
                index: 0,
            },
            LayerEdit::Reorder {
                ids: vec!["missing".into(); original.layers.len()],
            },
            LayerEdit::Update {
                id: original.layers[0].id.clone(),
                patch: LayerPatch {
                    name: Some("".into()),
                    ..Default::default()
                },
            },
        ] {
            assert!(apply_edits(&original, 0, &[edit]).is_err());
        }
        assert!(
            serde_json::from_str::<LayerEdit>(r#"{"op":"update","id":"a","patch":{"id":"b"}}"#)
                .is_err()
        );
        assert!(serde_json::from_str::<LayerEdit>(r#"{"op":"unknown"}"#).is_err());
        assert_eq!(original, doc());
    }

    #[test]
    fn replacement_revision_is_local_and_overflow_is_rejected() {
        let mut original = doc();
        let mut draft = original.clone();
        draft.revision = 123;
        assert_eq!(replace_setup(None, 0, &draft).unwrap().revision, 1);
        assert!(replace_setup(None, 1, &draft).is_err());
        original.revision = u64::MAX;
        assert!(replace_setup(Some(&original), u64::MAX, &draft).is_err());
    }
}
