//! Hardware-free layer draft and stable-ID operations. Transport belongs to the host.
use super::{setup_inspector, setup_targets};
use eframe::egui;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use underglow::setup::{Activation, Layer, SetupDocument, Target};
use underglow::setup_edits::{LayerEdit, LayerPatch, apply_edits};

pub enum Intent {
    Apply {
        revision: u64,
        document: SetupDocument,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document() -> SetupDocument {
        let mut doc =
            SetupDocument::from_toml(include_str!("../../../examples/layer-setup.toml")).unwrap();
        doc.revision = 8;
        doc.layers[0].variants = vec![serde_json::from_value(serde_json::json!({
            "id":"working", "name":"Working", "activation":{"kind":"while-state","source":"test","state":"working"},
            "renderer":{"kind":"solid","color":[1,2,3]}, "brightness":81
        })).unwrap()];
        doc
    }

    #[test]
    fn scoped_draft_edits_preserve_all_unrelated_data_and_variants() {
        let original = document();
        let id = original.layers[0].id.clone();
        let mut editor = Editor::default();
        editor.load(Some(original.clone()));
        editor
            .apply(LayerEdit::Update {
                id: id.clone(),
                patch: LayerPatch {
                    name: Some("Renamed".into()),
                    renderer: setup_inspector::default_renderer("matrix"),
                    ..Default::default()
                },
            })
            .unwrap();
        editor
            .apply(LayerEdit::Enable {
                id: id.clone(),
                enabled: false,
            })
            .unwrap();
        let edited = editor.draft.as_ref().unwrap().layers[0].clone();
        assert_eq!(edited.variants, original.layers[0].variants);
        assert_eq!(edited.target, original.layers[0].target);
        assert_eq!(edited.activation, original.layers[0].activation);
        assert_eq!(edited.brightness, original.layers[0].brightness);
        assert_eq!(
            &editor.draft.as_ref().unwrap().layers[1..],
            &original.layers[1..]
        );
        editor
            .apply(LayerEdit::Duplicate {
                id: id.clone(),
                new_id: "copy".into(),
                index: 1,
            })
            .unwrap();
        let mut copy = editor.draft.as_ref().unwrap().layers[1].clone();
        copy.id = id.clone();
        assert_eq!(copy, edited);
        editor
            .apply(LayerEdit::Remove { id: "copy".into() })
            .unwrap();
        let mut expected = original.clone();
        expected.layers[0] = edited;
        assert_eq!(editor.draft.as_ref().unwrap(), &expected);
        assert_eq!(editor.base_revision, 8);
        assert!(editor.dirty);
        editor.load(Some(original.clone()));
        assert_eq!(
            editor.draft.as_ref(),
            Some(&expected),
            "refresh cannot overwrite a draft"
        );
        assert!(!editor.accept_saved(&original, original.clone()));
        let mut saved = expected.clone();
        saved.revision = 9;
        assert!(editor.accept_saved(&expected, saved.clone()));
        assert_eq!(editor.draft, Some(saved));
        assert!(!editor.dirty);
        assert_eq!(editor.base_revision, 9);
        editor.blank();
        assert_ne!(editor.draft.as_ref().unwrap().id, original.id);
        assert!(editor.draft.as_ref().unwrap().layers.is_empty());
        assert!(editor.draft.as_ref().unwrap().groups.is_empty());
        assert_eq!(editor.base_revision, 9);
    }

    fn run(
        ctx: &egui::Context,
        editor: &mut Editor,
        events: Vec<egui::Event>,
    ) -> (egui::FullOutput, Option<Intent>) {
        let mut intent = None;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(940.0, 1800.0),
                )),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    intent = editor.show(ui);
                });
            },
        );
        (output, intent)
    }

    fn text_position(output: &egui::FullOutput, label: &str, nth: usize) -> egui::Pos2 {
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text() == label => {
                    Some(text.pos + text.galley.size() * 0.5)
                }
                _ => None,
            })
            .nth(nth)
            .unwrap_or_else(|| panic!("missing {label} #{nth}"))
    }

    fn click_at(ctx: &egui::Context, editor: &mut Editor, pos: egui::Pos2) {
        for pressed in [true, false] {
            run(
                ctx,
                editor,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            );
        }
    }

    fn click(ctx: &egui::Context, editor: &mut Editor, label: &str, nth: usize) {
        run(ctx, editor, vec![]);
        let (output, _) = run(ctx, editor, vec![]);
        click_at(ctx, editor, text_position(&output, label, nth));
    }

    #[test]
    fn headless_build_select_rename_mute_duplicate_reorder_and_delete_stack() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        click(&ctx, &mut editor, "New blank composition", 0);
        click(&ctx, &mut editor, "Create blank", 0);
        assert!(editor.draft.as_ref().unwrap().layers.is_empty());
        click(&ctx, &mut editor, "Add layer", 0);
        let bottom = editor.selected.clone().unwrap();
        click(&ctx, &mut editor, "Add layer", 0);
        let top = editor.selected.clone().unwrap();
        assert_ne!(bottom, top);
        let unrelated = editor.draft.as_ref().unwrap().layers[0].clone();
        // Actual mounted appearance inspector and physical-key canvas, not mocks.
        click(&ctx, &mut editor, "Aurora", 0);
        click(&ctx, &mut editor, "Matrix", 0);
        click(&ctx, &mut editor, "F1", 0);
        let selected = &editor.draft.as_ref().unwrap().layers[1];
        assert_eq!(selected.renderer.catalog_id(), Some("matrix"));
        assert_eq!(
            selected.target,
            Target::Selection {
                zones: vec![],
                keys: vec!["F1".into()],
                groups: vec![]
            }
        );
        click(&ctx, &mut editor, "New layer", 0); // TextEdit, list entries include index.
        run(
            &ctx,
            &mut editor,
            vec![
                egui::Event::Key {
                    key: egui::Key::A,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers {
                        command: true,
                        ctrl: true,
                        ..Default::default()
                    },
                },
                egui::Event::Text("Top renamed".into()),
            ],
        );
        assert_eq!(editor.draft.as_ref().unwrap().layers[1].name, "Top renamed");
        click(&ctx, &mut editor, "Enabled", 1);
        assert!(!editor.draft.as_ref().unwrap().layers[1].enabled);
        assert_eq!(editor.draft.as_ref().unwrap().layers[0], unrelated);
        click(&ctx, &mut editor, "Duplicate layer", 0);
        let copy_id = editor.selected.clone().unwrap();
        let before = editor.draft.as_ref().unwrap().clone();
        assert_eq!(before.layers.len(), 3);
        let mut copy = before.layers[2].clone();
        copy.id = top.clone();
        assert_eq!(copy, before.layers[1]);
        click(&ctx, &mut editor, "Toward bottom", 2);
        assert_eq!(editor.draft.as_ref().unwrap().layers[1].id, copy_id);
        assert_eq!(editor.selected.as_deref(), Some(copy_id.as_str()));
        click(&ctx, &mut editor, "Delete layer", 0);
        assert_eq!(editor.draft.as_ref().unwrap().layers, before.layers[..2]);
        click(&ctx, &mut editor, "1 · New layer", 0);
        assert_eq!(editor.selected.as_deref(), Some(bottom.as_str()));
        assert_eq!(editor.draft.as_ref().unwrap().layers[0], unrelated);
        assert!(editor.error.is_none(), "{:?}", editor.error);
    }
}

#[derive(Default)]
pub struct Editor {
    pub draft: Option<SetupDocument>,
    /// Last engine revision, never a count of local keystrokes.
    pub base_revision: u64,
    pub dirty: bool,
    pub selected: Option<String>,
    targets: BTreeMap<String, setup_targets::TargetSelection>,
    error: Option<String>,
    confirm_blank: bool,
}

fn fresh_id(prefix: &str) -> String {
    static SERIAL: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{prefix}-{now:x}-{:x}-{:x}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    )
}

impl Editor {
    /// Only acknowledge the exact draft submitted, never erase subsequent edits.
    pub fn accept_saved(&mut self, submitted: &SetupDocument, saved: SetupDocument) -> bool {
        if self.draft.as_ref() != Some(submitted) {
            return false;
        }
        self.dirty = false;
        self.load(Some(saved));
        true
    }

    /// Background status/setup refreshes must never replace an unsaved draft.
    pub fn load(&mut self, document: Option<SetupDocument>) {
        if self.dirty {
            return;
        }
        self.base_revision = document.as_ref().map_or(0, |doc| doc.revision);
        self.draft = document;
        self.reconcile_selection();
    }

    fn reconcile_selection(&mut self) {
        let layers = self
            .draft
            .as_ref()
            .map(|doc| doc.layers.as_slice())
            .unwrap_or(&[]);
        self.targets
            .retain(|id, _| layers.iter().any(|layer| &layer.id == id));
        if !layers
            .iter()
            .any(|layer| Some(&layer.id) == self.selected.as_ref())
        {
            self.selected = layers.last().map(|layer| layer.id.clone());
        }
    }

    pub fn blank(&mut self) {
        self.draft = Some(SetupDocument {
            schema_version: underglow::setup::SCHEMA_VERSION,
            revision: self.base_revision,
            id: fresh_id("setup"),
            name: "Blank composition".into(),
            layers: vec![],
            groups: BTreeMap::new(),
        });
        self.dirty = true;
        self.selected = None;
        self.targets.clear();
        self.error = None;
    }

    /// Typed local edits share the engine's scoped validation/preservation rules.
    pub fn apply(&mut self, edit: LayerEdit) -> Result<(), String> {
        let current = self.draft.as_ref().ok_or("No composition loaded")?;
        let mut candidate = apply_edits(current, current.revision, &[edit])?;
        candidate.revision = self.base_revision;
        self.draft = Some(candidate);
        self.dirty = true;
        self.reconcile_selection();
        Ok(())
    }

    fn add(&mut self) -> Result<(), String> {
        let id = fresh_id("layer");
        let index = self
            .draft
            .as_ref()
            .ok_or("No composition loaded")?
            .layers
            .len();
        self.apply(LayerEdit::Create {
            index,
            layer: Layer {
                id: id.clone(),
                name: "New layer".into(),
                enabled: true,
                renderer: setup_inspector::default_renderer("aurora").expect("catalog default"),
                brightness: underglow::DEFAULT_BRIGHTNESS,
                target: Target::Whole {},
                activation: Activation::Always {},
                variants: vec![],
            },
        })?;
        self.selected = Some(id);
        Ok(())
    }

    /// No source polling, engine requests, SDK ownership or preview producers.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Intent> {
        let mut intent = None;
        ui.heading("Layer composition");
        ui.small("Draft only until Apply & persist. Applying never resumes paused lighting.");
        ui.horizontal_wrapped(|ui| {
            if ui.button("New blank composition").clicked() {
                self.confirm_blank = true;
            }
            if self.draft.is_some() && ui.button("Add layer").clicked() {
                self.error = self.add().err();
            }
            if ui
                .add_enabled(self.dirty, egui::Button::new("Apply & persist composition"))
                .clicked()
                && let Some(doc) = &self.draft
            {
                match doc.validate() {
                    Ok(()) => {
                        intent = Some(Intent::Apply {
                            revision: self.base_revision,
                            document: doc.clone(),
                        })
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        });
        if self.confirm_blank {
            ui.horizontal_wrapped(|ui| {
                ui.label("Replace this draft with an empty stack?");
                if ui.button("Create blank").clicked() {
                    self.blank();
                    self.confirm_blank = false;
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_blank = false;
                }
            });
        }
        if let Some(error) = &self.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        let Some(doc) = self.draft.clone() else {
            ui.label(
                "No canonical setup loaded. Legacy compatibility controls are separate below.",
            );
            return intent;
        };
        ui.label(format!(
            "{} · engine revision {} · {}",
            doc.name,
            self.base_revision,
            if self.dirty {
                "Unapplied draft"
            } else {
                "Persisted"
            }
        ));
        ui.small("Stack order: bottom → top. Later layers cover earlier layers on their targets.");
        if doc.layers.is_empty() {
            ui.label("Empty composition · no hidden layers or Pi behavior. Add a layer to begin.");
        }
        let mut operation = None;
        egui::ScrollArea::vertical()
            .id_salt("layer-list")
            .max_height(210.0)
            .show(ui, |ui| {
                for (index, layer) in doc.layers.iter().enumerate() {
                    ui.push_id(&layer.id, |ui| {
                        ui.horizontal_wrapped(|ui| {
                            let mut enabled = layer.enabled;
                            if ui.checkbox(&mut enabled, "Enabled").changed() {
                                operation = Some(LayerEdit::Enable {
                                    id: layer.id.clone(),
                                    enabled,
                                });
                            }
                            if ui
                                .selectable_label(
                                    self.selected.as_ref() == Some(&layer.id),
                                    format!("{} · {}", index + 1, layer.name),
                                )
                                .clicked()
                            {
                                self.selected = Some(layer.id.clone());
                            }
                            for (label, offset) in [("Toward bottom", -1_isize), ("Toward top", 1)]
                            {
                                let next = index
                                    .checked_add_signed(offset)
                                    .filter(|next| *next < doc.layers.len());
                                if ui
                                    .add_enabled(next.is_some(), egui::Button::new(label))
                                    .clicked()
                                {
                                    let mut ids: Vec<_> =
                                        doc.layers.iter().map(|layer| layer.id.clone()).collect();
                                    ids.swap(index, next.unwrap());
                                    operation = Some(LayerEdit::Reorder { ids });
                                }
                            }
                        })
                    });
                }
            });
        if let Some(edit) = operation {
            self.error = self.apply(edit).err();
        }
        let Some(id) = self.selected.clone() else {
            return intent;
        };
        // Resolve again after reorder/enable, always by stable ID, never list index.
        let Some(doc) = self.draft.clone() else {
            return intent;
        };
        let Some(layer) = doc.layers.iter().find(|layer| layer.id == id) else {
            return intent;
        };
        let mut edited = layer.clone();
        ui.separator();
        ui.push_id(&id, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label("Layer name");
                ui.add(egui::TextEdit::singleline(&mut edited.name).char_limit(256));
                if ui.button("Duplicate layer").clicked() {
                    let new_id = fresh_id("layer");
                    let index = doc.layers.iter().position(|layer| layer.id == id).unwrap() + 1;
                    self.error = self
                        .apply(LayerEdit::Duplicate {
                            id: id.clone(),
                            new_id: new_id.clone(),
                            index,
                        })
                        .err();
                    if self.error.is_none() {
                        self.selected = Some(new_id);
                    }
                }
                if ui.button("Delete layer").clicked() {
                    self.error = self.apply(LayerEdit::Remove { id: id.clone() }).err();
                }
            });
            ui.small(format!("Stable ID: {id}"));
            ui.small(format!(
                "Condition (read-only): {}",
                serde_json::to_string(&layer.activation).unwrap_or_default()
            ));
            if !layer.variants.is_empty() {
                ui.collapsing(
                    format!("{} behavior appearances (read-only)", layer.variants.len()),
                    |ui| {
                        for variant in &layer.variants {
                            ui.label(format!(
                                "{} · {}",
                                variant.name,
                                serde_json::to_string(&variant.activation).unwrap_or_default()
                            ));
                        }
                    },
                );
            }
            setup_inspector::show(ui, &mut edited);
            ui.separator();
            let others: Vec<_> = doc
                .layers
                .iter()
                .filter(|layer| layer.id != id)
                .map(|layer| layer.target.clone())
                .collect();
            let mut groups = doc.groups.clone();
            self.targets.entry(id.clone()).or_default().show(
                ui,
                &mut edited.target,
                &mut groups,
                &others,
                &underglow::device::DeviceInfo::synthetic_80he(),
            );
            if edited != *layer || groups != doc.groups {
                // Group edits are a separate declarative map; validate atomically with
                // the selected layer patch, retaining every other layer byte-for-byte.
                let mut current = self.draft.clone().unwrap();
                current.groups = groups;
                let patch = LayerPatch {
                    name: (edited.name != layer.name).then_some(edited.name),
                    renderer: (edited.renderer != layer.renderer).then_some(edited.renderer),
                    brightness: (edited.brightness != layer.brightness)
                        .then_some(edited.brightness),
                    target: (edited.target != layer.target).then_some(edited.target),
                    ..Default::default()
                };
                match apply_edits(
                    &current,
                    current.revision,
                    &[LayerEdit::Update {
                        id: id.clone(),
                        patch,
                    }],
                ) {
                    Ok(mut candidate) => {
                        candidate.revision = self.base_revision;
                        self.draft = Some(candidate);
                        self.dirty = true;
                        self.error = None;
                    }
                    Err(error) => self.error = Some(error),
                }
            }
        });
        intent
    }
}
