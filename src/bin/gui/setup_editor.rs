//! Hardware-free layer draft and stable-ID operations. Transport belongs to the host.
use super::{
    setup_behavior, setup_coverage, setup_explanation, setup_inspector, setup_preview,
    setup_targets,
};
use eframe::egui;
#[path = "setup_editor_canvas.rs"]
mod canvas;
#[path = "setup_editor_inspector.rs"]
mod inspector;
#[path = "setup_editor_workspace.rs"]
mod workspace;
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

    #[test]
    fn starter_copy_is_a_draft_that_can_be_dismantled_named_and_saved_without_reinjection() {
        let original = document();
        let mut editor = Editor::default();
        editor.load(Some(original.clone()));
        let starter = underglow::setup_library::starter(
            underglow::setup_library::Starter::MatrixPi,
            "user-copy".into(),
            "Editable starter".into(),
        )
        .unwrap();
        assert!(editor.starter_draft(starter.clone()));
        assert!(
            !editor.starter_draft(original.clone()),
            "cannot replace a dirty starter draft"
        );
        assert_eq!(editor.applied, Some(original));
        assert_eq!(editor.base_revision, 8);
        for layer in starter.layers {
            editor.apply(LayerEdit::Remove { id: layer.id }).unwrap();
        }
        editor.name_draft("Entirely mine".into()).unwrap();
        let submitted = editor.draft.clone().unwrap();
        assert!(submitted.layers.is_empty());
        let mut saved = submitted.clone();
        saved.revision = 9;
        assert!(editor.accept_saved(&submitted, saved.clone()));
        editor.load(Some(saved.clone()));
        assert_eq!(editor.draft, Some(saved));
        assert!(!editor.dirty);
    }

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
    fn coverage_highlighting_is_read_only_and_conflict_is_visibly_resolved_by_stack_edit() {
        let ctx = egui::Context::default();
        let mut original = document();
        original.layers.truncate(1);
        original.layers[0].variants.clear();
        original.layers[0].renderer = underglow::setup::Renderer::Solid { color: [0, 80, 80] };
        original.layers[0].target = Target::Whole {};
        original.layers[0].activation = Activation::Always {};
        let mut top = original.layers[0].clone();
        top.id = "black-conflict".into();
        top.name = "Black conflict".into();
        top.renderer = underglow::setup::Renderer::Solid { color: [0; 3] };
        top.target = Target::Selection {
            zones: vec![],
            keys: vec!["F3".into()],
            groups: vec![],
        };
        original.layers.push(top);
        let mut editor = Editor::default();
        editor.load(Some(original.clone()));
        editor.selected = Some(original.layers[0].id.clone());
        editor.preview.set_mode(setup_preview::Mode::Scenario);
        click(&ctx, &mut editor, "Preview tools", 0);
        run(&ctx, &mut editor, vec![]);
        let generation = editor.preview.scenario.generation();
        let frame = editor.preview.frame().clone();
        let revision = editor.base_revision;
        let facts = editor.preview.evaluation_facts().0.clone();
        for highlighted in [true, false, true] {
            click(&ctx, &mut editor, "Highlight selected layer coverage", 0);
            assert_eq!(editor.highlight_coverage, highlighted);
            let (output, intent) = run(&ctx, &mut editor, vec![]);
            assert!(intent.is_none());
            assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text().contains("partly obscured"))));
            assert_eq!(editor.draft.as_ref(), Some(&original));
            assert_eq!(
                editor.selected.as_deref(),
                Some(original.layers[0].id.as_str())
            );
            assert!(!editor.dirty);
            assert_eq!(editor.preview.scenario.generation(), generation);
            assert_eq!(editor.preview.evaluation_facts().0, &facts);
            assert_eq!(editor.preview.frame(), &frame);
            assert_eq!(editor.base_revision, revision);
        }
        // An explicit normal stack edit resolves the black overlap. Inspection
        // itself neither rewrites targets nor issues a physical Apply intent.
        editor
            .apply(LayerEdit::Reorder {
                ids: vec![original.layers[1].id.clone(), original.layers[0].id.clone()],
            })
            .unwrap();
        let (output, intent) = run(&ctx, &mut editor, vec![]);
        assert!(intent.is_none());
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Text(text) if text.galley.text().contains("fully obscured"))));
        let report = editor.preview.coverage().unwrap();
        assert_eq!(report.layers[1].visible.len(), report.keys.len());
        assert_eq!(report.layers[0].visible.len(), 0);
        assert_eq!(
            editor.draft.as_ref().unwrap().layers[0].target,
            original.layers[1].target
        );
    }

    #[test]
    fn composed_key_click_inspects_f3_without_editing_or_retargeting() {
        let ctx = egui::Context::default();
        let mut editor = Editor::default();
        let original = document();
        editor.load(Some(original.clone()));
        // Preview is read-only; inspection requires the explicit Inspect mode.
        click(&ctx, &mut editor, "Inspect", 0);
        click(&ctx, &mut editor, "F3", 0);
        let coord = underglow::key_targets::canonical_coord("F3").unwrap();
        assert_eq!(editor.explained_key, Some(coord));
        assert_eq!(editor.draft.as_ref(), Some(&original));
        assert!(!editor.dirty);
        let selected = editor.selected.clone();
        let report = setup_explanation::report(&editor.preview, coord).unwrap();
        assert_eq!(report.color, editor.preview.frame().get_coord(coord));
        click(&ctx, &mut editor, "Preview tools", 0);
        let (output, intent) = run(&ctx, &mut editor, vec![]);
        assert!(
            intent.is_none(),
            "inspection has no physical activation intent"
        );
        assert_eq!(editor.selected, selected);
        assert!(output.shapes.iter().any(|shape| matches!(&shape.shape,egui::Shape::Text(text) if text.galley.text().starts_with("Why F3?"))));
    }

    #[test]
    fn connections_repair_is_only_a_scoped_unsaved_draft_edit_and_rejects_stale_dialogs() {
        let original = document();
        let binding = underglow::binding_repair::bindings(&original)
            .into_iter()
            .find(|b| b.key.variant_id.as_deref() == Some("working"))
            .unwrap();
        let repair = underglow::binding_repair::BindingRepair {
            setup_id: original.id.clone(),
            key: binding.key,
            expected_activation: binding.activation.clone(),
            source: "pi".into(),
            instance: Some("new-session".into()),
        };
        let mut editor = Editor::default();
        editor.load(Some(original.clone()));
        editor.repair_sources(&Default::default(), &repair).unwrap();
        assert!(editor.dirty);
        assert_eq!(editor.applied.as_ref(), Some(&original));
        assert_eq!(editor.base_revision, original.revision);
        let changed = editor.draft.clone();
        assert!(editor.repair_sources(&Default::default(), &repair).is_err());
        assert_eq!(editor.draft, changed);
        let mut expected = original.clone();
        expected.layers[0].variants[0].activation = Activation::WhileState {
            source: "pi".into(),
            instance: Some("new-session".into()),
            state: Some("working".into()),
        };
        assert_eq!(editor.draft.as_ref(), Some(&expected));
        editor.discard();
        assert_eq!(editor.draft.as_ref(), Some(&original));
        assert!(!editor.dirty);
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
                    opacity: Some(128),
                    blend: Some(underglow::setup::BlendMode::Add),
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
        assert_eq!(edited.opacity, 128);
        assert_eq!(edited.blend, underglow::setup::BlendMode::Add);
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
                    egui::vec2(940.0, 850.0),
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
                egui::Shape::Text(text)
                    if text.galley.text() == label
                        && shape
                            .clip_rect
                            .contains(text.pos + text.galley.size() * 0.5) =>
                {
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
        if matches!(label, "New blank composition" | "Add Pi template") {
            click(ctx, editor, "More", 0);
        }
        // Reveal the requested control in its real bounded inspector, not a taller window.
        for step in 0..65 {
            run(ctx, editor, vec![]);
            let (output, _) = run(ctx, editor, vec![]);
            let hit = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.text() == label => {
                        let pos = text.pos + text.galley.size() * 0.5;
                        (shape.clip_rect.contains(pos) && pos.y < 850.0).then_some(pos)
                    }
                    _ => None,
                })
                .nth(nth);
            if let Some(pos) = hit {
                click_at(ctx, editor, pos);
                return;
            }
            let delta = if step == 32 { 20000.0 } else { -220.0 };
            run(
                ctx,
                editor,
                vec![
                    egui::Event::PointerMoved(egui::pos2(
                        if editor.preview_tools_open {
                            450.0
                        } else {
                            900.0
                        },
                        450.0,
                    )),
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(0.0, delta),
                        modifiers: Default::default(),
                    },
                ],
            );
        }
        panic!("missing reachable {label} #{nth}");
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
        click(&ctx, &mut editor, "Composition preview", 0); // close color popup
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
        click(&ctx, &mut editor, "Select keys", 0);
        click(&ctx, &mut editor, "F1", 0);
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
        click(&ctx, &mut editor, "Select keys", 0);
        click(&ctx, &mut editor, "F1", 0); // One composed RGB canvas, explicit target mode.
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
        click(&ctx, &mut editor, "Enabled", 0); // topmost layer is displayed first
        assert!(!editor.draft.as_ref().unwrap().layers[1].enabled);
        assert_eq!(editor.draft.as_ref().unwrap().layers[0], unrelated);
        click(&ctx, &mut editor, "Duplicate layer", 0);
        let copy_id = editor.selected.clone().unwrap();
        let before = editor.draft.as_ref().unwrap().clone();
        assert_eq!(before.layers.len(), 3);
        let mut copy = before.layers[2].clone();
        copy.id = top.clone();
        assert_eq!(copy, before.layers[1]);
        click(&ctx, &mut editor, "Toward bottom", 0);
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
    highlight_coverage: bool,
    pub can_save: bool,
    pub preview_brightness: Option<u8>,
    /// Last engine revision, never a count of local keystrokes.
    pub base_revision: u64,
    pub dirty: bool,
    pub selected: Option<String>,
    pub canvas_mode: setup_targets::CanvasMode,
    canvas_selection: Option<String>,
    empty_canvas: setup_targets::TargetSelection,
    targets: BTreeMap<String, setup_targets::TargetSelection>,
    behaviors: setup_behavior::BehaviorEditor,
    capabilities: Option<CapabilityRegistry>,
    error: Option<String>,
    confirm_blank: bool,
    preview_tools_open: bool,
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
    /// Explicit Connections repair changes only a local draft; never saves,
    /// starts a producer, sends real facts, or resumes physical lighting.
    pub fn repair_sources(
        &mut self,
        registry: &underglow::connection_registry::ConnectionRegistry,
        repair: &underglow::binding_repair::BindingRepair,
    ) -> Result<(), String> {
        let current = self.draft.as_ref().ok_or("No composition draft loaded")?;
        let (candidate, _) = underglow::binding_repair::repair_binding(current, registry, repair)?;
        self.draft = Some(candidate);
        self.dirty = true;
        self.selected = Some(repair.key.layer_id.clone());
        self.error = None;
        self.preview.rebaseline();
        Ok(())
    }

    /// Explicit starter copy; never discard edits and never change applied data.
    pub fn starter_draft(&mut self, document: SetupDocument) -> bool {
        if self.dirty {
            return false;
        }
        self.draft = Some(document);
        self.dirty = true;
        self.reset_canvas_context();
        self.selected = None;
        self.error = None;
        self.reconcile_selection();
        self.preview.rebaseline();
        true
    }

    pub fn name_draft(&mut self, name: String) -> Result<(), String> {
        let mut document = self.draft.clone().ok_or("No composition loaded")?;
        document.name = name;
        document.validate().map_err(|error| error.to_string())?;
        self.dirty |= self.draft.as_ref() != Some(&document);
        self.draft = Some(document);
        Ok(())
    }

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
            self.reset_canvas_context();
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
        self.reset_canvas_context();
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
        self.reset_canvas_context();
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
                transition: Default::default(),
                opacity: 255,
                blend: Default::default(),
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

    /// Single persistence-control hook; the host may mount this outside the workspace.
    pub fn show_save_controls(&mut self, ui: &mut egui::Ui) -> Option<Intent> {
        let mut intent = None;
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

        intent
    }

    /// No source polling, engine requests, SDK ownership or preview producers.
    pub fn show(&mut self, ui: &mut egui::Ui) -> Option<Intent> {
        let mut intent = None;
        // Workspace-specific density uses the production fonts/colors, not a test-only style.
        ui.spacing_mut().item_spacing = egui::vec2(6.0, 6.0);
        ui.spacing_mut().button_padding = egui::vec2(8.0, 5.0);
        ui.horizontal_wrapped(|ui| {
            intent = self.show_save_controls(ui);
            if self.draft.is_some() && ui.button("Add layer").clicked() {
                self.error = self.add().err();
            }
            ui.menu_button("More", |ui| {
                if ui.button("New blank composition").clicked() {
                    self.confirm_blank = true;
                    ui.close_menu();
                }
                if self.draft.is_some() && ui.button("Add Pi template").clicked() {
                    self.error = self.add_pi_template().err();
                    ui.close_menu();
                }
                ui.small(format!(
                    "Engine revision {} · {}",
                    self.base_revision,
                    if self.dirty {
                        "Unapplied draft"
                    } else {
                        "Persisted"
                    }
                ));
                if !self.can_save {
                    ui.small("Pause lighting before Save. Offline drafts remain editable.");
                }
            });
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
            ui.add(
                egui::Label::new(egui::RichText::new(error).color(egui::Color32::LIGHT_RED))
                    .truncate(),
            )
            .on_hover_text(error);
        }
        if self.draft.is_some() {
            self.preview
                .update(self.draft.as_ref(), self.preview_brightness.unwrap_or(255));
            self.show_workspace(ui);
        } else {
            ui.label(
                "No canonical setup loaded. Legacy compatibility controls are separate below.",
            );
        }
        if self.preview_tools_open {
            self.show_preview_tools_window(ui.ctx());
        }
        intent
    }
}
