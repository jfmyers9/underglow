//! Hardware-free layer draft and stable-ID operations. Transport belongs to the host.
use super::{setup_behavior, setup_explanation, setup_inspector, setup_preview, setup_targets};
use eframe::egui;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use underglow::setup::{Activation, Layer, SetupDocument, Target, TargetZone};
use underglow::setup_edits::{LayerEdit, LayerPatch, apply_edits};
use underglow::source_capabilities::CapabilityRegistry;

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
    fn composed_key_click_inspects_f3_without_editing_or_retargeting() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        let original = document();
        editor.load(Some(original.clone()));
        // First F3 belongs to composed output, second to the target editor.
        click(&ctx, &mut editor, "F3", 0);
        let coord = underglow::key_targets::canonical_coord("F3").unwrap();
        assert_eq!(editor.explained_key, Some(coord));
        assert_eq!(editor.draft.as_ref(), Some(&original));
        assert!(!editor.dirty);
        let selected = editor.selected.clone();
        let report = setup_explanation::report(&editor.preview, coord).unwrap();
        assert_eq!(report.color, editor.preview.frame().get_coord(coord));
        let (output, intent) = run(&ctx, &mut editor, vec![]);
        assert!(
            intent.is_none(),
            "inspection has no physical activation intent"
        );
        assert_eq!(editor.selected, selected);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text().starts_with("Why F3?"))));
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
    fn headless_build_matrix_cyan_arrows_function_row_save_and_discard() {
        let ctx = egui::Context::default();
        let mut editor = Editor {
            can_save: true,
            ..Default::default()
        };
        click(&ctx, &mut editor, "New blank composition", 0);
        click(&ctx, &mut editor, "Create blank", 0);
        click(&ctx, &mut editor, "Add layer", 0);
        click(&ctx, &mut editor, "Aurora", 0);
        click(&ctx, &mut editor, "Matrix", 0);
        click(&ctx, &mut editor, "Add layer", 0);
        click(&ctx, &mut editor, "Aurora", 0);
        click(&ctx, &mut editor, "Solid", 0);
        let (output, _) = run(&ctx, &mut editor, vec![]);
        let swatch = text_position(&output, "Color", 0) - egui::vec2(38.0, 0.0);
        click_at(&ctx, &mut editor, swatch);
        click(&ctx, &mut editor, "R 255", 0);
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
                egui::Event::Text("0".into()),
                egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: Default::default(),
                },
            ],
        );
        assert_eq!(
            editor.draft.as_ref().unwrap().layers[1].renderer,
            underglow::setup::Renderer::Solid {
                color: [0, 255, 255]
            }
        );
        click(&ctx, &mut editor, "Layer composition", 0); // close color popup
        click(&ctx, &mut editor, "Arrow keys (4 physical)", 0);
        click(&ctx, &mut editor, "Add layer", 0);
        click(&ctx, &mut editor, "Aurora", 0);
        click(&ctx, &mut editor, "Pulse", 0);
        click(&ctx, &mut editor, "Named zones (legacy engine map)", 0);
        click(&ctx, &mut editor, "Function", 0);
        let draft = editor.draft.clone().unwrap();
        assert_eq!(draft.layers[0].renderer.catalog_id(), Some("matrix"));
        assert_eq!(draft.layers[0].target, Target::Whole {});
        assert_eq!(
            draft.layers[1].target,
            Target::Selection {
                zones: vec![],
                keys: ["ArrowUp", "ArrowLeft", "ArrowDown", "ArrowRight"]
                    .map(str::to_owned)
                    .to_vec(),
                groups: vec![]
            }
        );
        assert!(matches!(
            draft.layers[2].renderer,
            underglow::setup::Renderer::Pulse { .. }
        ));
        assert_eq!(
            draft.layers[2].target,
            Target::Selection {
                zones: vec![underglow::setup::TargetZone::Function],
                keys: vec![],
                groups: vec![]
            }
        );
        let (output, _) = run(&ctx, &mut editor, vec![]);
        let pos = text_position(&output, "Save composition", 0);
        let mut intent = None;
        for pressed in [true, false] {
            intent = run(
                &ctx,
                &mut editor,
                vec![
                    egui::Event::PointerMoved(pos),
                    egui::Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .1
            .or(intent);
        }
        let Some(Intent::Apply { revision, document }) = intent else {
            panic!("Save must emit one explicit CAS intent");
        };
        assert_eq!(revision, 0);
        assert_eq!(document, draft);
        editor.save_error("stale setup revision".into());
        assert_eq!(editor.draft.as_ref(), Some(&draft));
        assert!(editor.applied.is_none());
        assert!(editor.dirty);
        assert!(editor.error.is_some());
        let mut saved = draft.clone();
        saved.revision = 1;
        assert!(editor.accept_saved(&draft, saved.clone()));
        assert!(editor.error.is_none());
        let restored = SetupDocument::from_toml(&saved.to_toml().unwrap()).unwrap();
        let mut restarted = Editor::default();
        restarted.load(Some(restored));
        assert_eq!(restarted.draft, Some(saved.clone()));
        click(&ctx, &mut editor, "Add layer", 0);
        editor.load(Some(draft)); // refresh cannot overwrite dirty state or baseline
        assert_eq!(editor.draft.as_ref().unwrap().layers.len(), 4);
        click(&ctx, &mut editor, "Discard draft", 0);
        assert_eq!(editor.draft, Some(saved.clone()));
        assert_eq!(editor.applied, Some(saved));
        assert!(!editor.dirty);
        assert_eq!(editor.base_revision, 1);
    }

    #[test]
    fn mounted_behavior_forms_use_custom_metadata_preserve_content_and_discard() {
        use underglow::source_capabilities::{FactDescriptor, pi_descriptor};
        let ctx = egui::Context::default();
        let original = document();
        let id = original.layers[0].id.clone();
        let mut editor = Editor::default();
        editor.load(Some(original.clone()));
        editor.selected = Some(id.clone());
        editor.can_save = true;
        let mut descriptor = pi_descriptor();
        descriptor.source = "build".into();
        descriptor.display_name = "Build server".into();
        descriptor.events = vec![FactDescriptor {
            id: "artifact".into(),
            label: "Artifact ready".into(),
        }];
        let mut registry = CapabilityRegistry::new();
        registry.insert(descriptor).unwrap();
        editor.set_capabilities(registry);
        click(&ctx, &mut editor, "Always", 0);
        click(&ctx, &mut editor, "When an event occurs", 0);
        let changed = editor.draft.as_ref().unwrap();
        assert!(
            matches!(&changed.layers[0].activation, Activation::OnEvent { source, event, .. } if source == "build" && event == "artifact")
        );
        let mut expected = original.clone();
        expected.layers[0].activation = changed.layers[0].activation.clone();
        assert_eq!(changed, &expected);
        assert_eq!(editor.applied.as_ref(), Some(&original));
        click(&ctx, &mut editor, "Add behavior appearance", 0);
        click(&ctx, &mut editor, "Always", 0);
        click(&ctx, &mut editor, "When an event occurs", 0);
        let changed = editor.draft.as_ref().unwrap();
        assert_eq!(changed.layers[0].variants.len(), 2);
        assert!(
            matches!(&changed.layers[0].variants[1].activation, Activation::OnEvent { source, event, .. } if source == "build" && event == "artifact")
        );
        assert_eq!(changed.layers[0].variants[1].brightness, None);
        assert_eq!(
            changed.layers[0].variants[0],
            original.layers[0].variants[0]
        );
        assert_eq!(changed.layers[0].target, original.layers[0].target);
        assert_eq!(changed.layers[0].id, id);
        assert_eq!(&changed.layers[1..], &original.layers[1..]);
        assert_eq!(changed.groups, original.groups);
        assert!(editor.dirty);
        click(&ctx, &mut editor, "Save composition", 0);
        assert_eq!(
            editor.applied.as_ref(),
            Some(&original),
            "Save is only an intent until acknowledged"
        );
        click(&ctx, &mut editor, "Discard draft", 0);
        assert_eq!(editor.draft.as_ref(), Some(&original));
        assert!(!editor.dirty);
        assert!(editor.error.is_none(), "{:?}", editor.error);
    }

    #[test]
    fn headless_pi_template_appends_retargets_and_deletes_without_reset_or_save() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        editor.blank();
        click(&ctx, &mut editor, "Add layer", 0); // Aurora, not Matrix.
        let original = editor.draft.clone().unwrap();
        assert_eq!(original.layers[0].renderer.catalog_id(), Some("aurora"));
        assert!(!editor.can_save);
        click(&ctx, &mut editor, "Add Pi template", 0);
        let id = editor.selected.clone().unwrap();
        let draft = editor.draft.as_ref().unwrap();
        assert_eq!(draft.layers.len(), 2);
        assert_eq!(draft.layers[0], original.layers[0]);
        assert_eq!(draft.groups, original.groups);
        assert_eq!(draft.id, original.id);
        assert_eq!(draft.layers[1].variants.len(), 2);
        assert_eq!(draft.layers[1].id, id);
        assert!(editor.dirty);
        assert!(editor.applied.is_none(), "Add is a draft-only action");
        assert!(
            !editor
                .preview
                .runtime()
                .unwrap()
                .requirements()
                .analog_input
        );
        // Mounted physical-key canvas retargets every appearance together.
        click(&ctx, &mut editor, "Whole keyboard", 0);
        click(&ctx, &mut editor, "F1", 1);
        assert_eq!(
            editor.draft.as_ref().unwrap().layers[1].target,
            Target::Selection {
                zones: vec![],
                keys: vec!["F1".into()],
                groups: vec![]
            }
        );
        assert_eq!(editor.draft.as_ref().unwrap().layers[0], original.layers[0]);
        click(&ctx, &mut editor, "Delete layer", 0);
        assert_eq!(editor.draft.as_ref(), Some(&original));
        let saved = editor.draft.clone().unwrap();
        let mut reopened = Editor::default();
        reopened.load(Some(saved.clone()));
        run(&ctx, &mut reopened, vec![]);
        assert_eq!(reopened.draft, Some(saved));
        assert!(!reopened.dirty);
        reopened.blank();
        assert!(reopened.draft.as_ref().unwrap().layers.is_empty());
        assert!(editor.error.is_none(), "{:?}", editor.error);
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
        click(&ctx, &mut editor, "F1", 1); // Editing canvas, not composed preview legend.
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
    /// Last acknowledged applied composition, independent from local edits.
    pub applied: Option<SetupDocument>,
    pub preview: setup_preview::Preview,
    explained_key: Option<underglow::layout::MatrixCoord>,
    pub can_save: bool,
    pub preview_brightness: Option<u8>,
    /// Last engine revision, never a count of local keystrokes.
    pub base_revision: u64,
    pub dirty: bool,
    pub selected: Option<String>,
    targets: BTreeMap<String, setup_targets::TargetSelection>,
    behaviors: setup_behavior::BehaviorEditor,
    capabilities: Option<CapabilityRegistry>,
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
    /// Declarative metadata from Connections; never starts a producer or grants trust.
    pub fn set_capabilities(&mut self, registry: CapabilityRegistry) {
        self.preview.set_capabilities(registry.clone());
        self.capabilities = Some(registry);
    }

    /// Only acknowledge the exact draft submitted, never erase subsequent edits.
    pub fn accept_saved(&mut self, submitted: &SetupDocument, saved: SetupDocument) -> bool {
        if self.draft.as_ref() != Some(submitted) {
            return false;
        }
        self.dirty = false;
        self.error = None;
        self.load(Some(saved));
        self.preview.rebaseline();
        true
    }

    /// Background status/setup refreshes must never replace an unsaved draft.
    pub fn load(&mut self, document: Option<SetupDocument>) {
        if self.capabilities.is_none() {
            self.set_capabilities(CapabilityRegistry::with_builtins());
        }
        if self.dirty {
            return;
        }
        self.base_revision = document.as_ref().map_or(0, |doc| doc.revision);
        if self.applied != document {
            self.preview.rebaseline();
        }
        self.applied = document.clone();
        self.draft = document;
        self.reconcile_selection();
    }

    pub fn discard(&mut self) {
        self.draft = self.applied.clone();
        self.dirty = false;
        self.error = None;
        self.confirm_blank = false;
        self.targets.clear();
        self.reconcile_selection();
        self.preview.rebaseline();
    }

    pub fn save_error(&mut self, error: String) {
        self.error = Some(error);
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

    fn add_pi_template(&mut self) -> Result<(), String> {
        let id = fresh_id("pi");
        let index = self
            .draft
            .as_ref()
            .ok_or("No composition loaded")?
            .layers
            .len();
        self.apply(LayerEdit::Create {
            index,
            layer: underglow::templates::pi_status_layer(
                id.clone(),
                Target::Selection {
                    zones: vec![TargetZone::Function],
                    keys: vec![],
                    groups: vec![],
                },
                None,
            ),
        })?;
        self.selected = Some(id);
        Ok(())
    }

    /// No source polling, engine requests, SDK ownership or preview producers.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Intent> {
        let mut intent = None;
        ui.heading("Layer composition");
        ui.small("Hardware-free draft preview. Save while paused; Resume lighting separately.");
        if !self.can_save {
            ui.small("Pause lighting before Save. Offline drafts remain editable.");
        }
        ui.horizontal_wrapped(|ui| {
            if ui.button("New blank composition").clicked() {
                self.confirm_blank = true;
            }
            if self.draft.is_some() && ui.button("Add layer").clicked() {
                self.error = self.add().err();
            }
            if self.draft.is_some() && ui.button("Add Pi template").clicked() {
                self.error = self.add_pi_template().err();
            }
            if ui
                .add_enabled(
                    self.dirty && self.can_save,
                    egui::Button::new("Save composition"),
                )
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
            if ui
                .add_enabled(self.dirty, egui::Button::new("Discard draft"))
                .clicked()
            {
                self.discard();
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
        self.preview.show_controls(ui);
        self.preview
            .update(self.draft.as_ref(), self.preview_brightness.unwrap_or(255));
        if self.preview.mode == setup_preview::Mode::Live
            && self
                .preview
                .runtime()
                .is_some_and(|runtime| runtime.requirements().analog_input)
        {
            ui.small("Input-driven layers are hidden here. Choose Isolated input demo to simulate keystrokes.");
        }
        if let Some(error) = &self.preview.error {
            ui.colored_label(egui::Color32::LIGHT_RED, error);
        }
        if let Some(coord) =
            setup_targets::composition_preview(ui, self.preview.frame(), self.explained_key)
        {
            self.explained_key = Some(coord);
        }
        setup_explanation::show(ui, &self.preview, self.explained_key);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(33));
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
            // sample() is raw real-source transport data even in synthetic preview modes.
            let sample = self.preview.sample().filter(|sample| !sample.is_stale());
            self.behaviors.show(
                ui,
                &mut edited,
                self.capabilities
                    .get_or_insert_with(CapabilityRegistry::with_builtins),
                sample.map(|sample| &sample.snapshot),
                sample.map(|sample| &sample.events),
            );
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
                    activation: (edited.activation != layer.activation)
                        .then_some(edited.activation),
                    variants: (edited.variants != layer.variants).then_some(edited.variants),
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
