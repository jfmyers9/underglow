//! Declarative target inspector. No SDK, engine commands, or real input sources.
//! The layer editor owns document validation/revision and persists dirty edits.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::collections::BTreeMap;
use underglow::automation::{AutomationConfig, Scene, SceneZone};
use underglow::compositor::KeyMask;
use underglow::device::DeviceInfo;
use underglow::key_targets::{canonical_coord, validate_device};
use underglow::layout::{KeyboardLayout, MatrixCoord};
use underglow::setup::{MAX_GROUPS, MAX_KEYS, Target, TargetZone};

type Groups = BTreeMap<String, Vec<String>>;

const ZONES: [(TargetZone, &str); 5] = [
    (TargetZone::Function, "Function"),
    (TargetZone::Alpha, "Alpha"),
    (TargetZone::Navigation, "Navigation"),
    (TargetZone::Arrows, "Arrows"),
    (TargetZone::System, "System"),
];
const ARROWS: [&str; 4] = ["ArrowUp", "ArrowLeft", "ArrowDown", "ArrowRight"];

/// Physical names only: coordinates and display geometry come from the runtime.
fn names() -> Vec<String> {
    let mut names: Vec<String> = ('A'..='Z')
        .chain('0'..='9')
        .map(|c| c.to_string())
        .collect();
    names.extend((1..=12).map(|n| format!("F{n}")));
    names.extend(
        [
            "Escape",
            "PrintScreen",
            "Pause",
            "Grave",
            "Minus",
            "Equal",
            "Backspace",
            "Tab",
            "BracketLeft",
            "BracketRight",
            "Backslash",
            "CapsLock",
            "Semicolon",
            "Quote",
            "Enter",
            "ShiftLeft",
            "ShiftRight",
            "Comma",
            "Period",
            "Slash",
            "Space",
            "Insert",
            "Home",
            "PageUp",
            "Delete",
            "End",
            "PageDown",
            "ArrowUp",
            "ArrowLeft",
            "ArrowDown",
            "ArrowRight",
            "ControlLeft",
            "MetaLeft",
            "AltLeft",
            "AltRight",
            "ControlRight",
        ]
        .map(str::to_owned),
    );
    names
}

/// Use the engine's existing selector resolver, not a second GUI interpretation.
fn target_mask(target: &Target, groups: &Groups) -> Result<KeyMask, String> {
    let Target::Selection {
        zones,
        keys,
        groups: refs,
    } = target
    else {
        return Ok(KeyMask::Whole);
    };
    if zones.is_empty() && keys.is_empty() && refs.is_empty() {
        return Err(
            "Choose at least one key, zone, or group; empty selection is not Whole.".into(),
        );
    }
    if zones.len() > 5 || keys.len() > MAX_KEYS || refs.len() > MAX_GROUPS {
        return Err("Target exceeds setup selector limits.".into());
    }
    let scene = Scene {
        zones: zones
            .iter()
            .map(|zone| match zone {
                TargetZone::Function => SceneZone::Function,
                TargetZone::Alpha => SceneZone::Alpha,
                TargetZone::Navigation => SceneZone::Navigation,
                TargetZone::Arrows => SceneZone::Arrows,
                TargetZone::System => SceneZone::System,
            })
            .collect(),
        keys: keys.clone(),
        groups: refs.clone(),
        ..Scene::default()
    };
    AutomationConfig {
        groups: groups.clone(),
        ..AutomationConfig::default()
    }
    .scene_mask(&scene)
}

fn selection(target: &mut Target) -> (&mut Vec<TargetZone>, &mut Vec<String>, &mut Vec<String>) {
    if matches!(target, Target::Whole {}) {
        *target = Target::Selection {
            zones: vec![],
            keys: vec![],
            groups: vec![],
        };
    }
    let Target::Selection {
        zones,
        keys,
        groups,
    } = target
    else {
        unreachable!()
    };
    (zones, keys, groups)
}

#[derive(Clone, Copy)]
enum Edit {
    ReplaceDirect,
    Toggle,
    Add,
}

fn edit_keys(target: &mut Target, hits: &[String], edit: Edit) -> Result<(), String> {
    for name in hits {
        if canonical_coord(name).is_none() {
            return Err(format!(
                "Unknown or unsupported physical key {name:?} (80HE ANSI)."
            ));
        }
    }
    if hits.is_empty() {
        return Ok(()); // Clicking/dragging gaps never destroys existing selectors.
    }
    if matches!(target, Target::Whole {}) && !matches!(edit, Edit::ReplaceDirect) {
        return Err("Whole is still selected. Click without modifiers to replace it first.".into());
    }
    let (_, keys, _) = selection(target);
    if matches!(edit, Edit::ReplaceDirect) {
        keys.clear();
    }
    for name in hits {
        let coord = canonical_coord(name).unwrap();
        if keys.iter().any(|key| canonical_coord(key) == Some(coord)) {
            if matches!(edit, Edit::Toggle) {
                keys.retain(|key| canonical_coord(key) != Some(coord));
            }
        } else {
            keys.push(name.clone());
        }
    }
    Ok(())
}

fn references(target: &Target, id: &str) -> bool {
    matches!(target, Target::Selection { groups, .. } if groups.iter().any(|g| g == id))
}

fn valid_group(id: &str, keys: &[String]) -> Result<(), String> {
    if id.is_empty()
        || id.len() > 128
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.:/".contains(&b))
    {
        return Err(
            "Group name must be 1–128 ASCII letters/digits or -_.:/ (case-sensitive).".into(),
        );
    }
    if keys.is_empty() || keys.len() > MAX_KEYS {
        return Err("A group needs 1–128 physical keys.".into());
    }
    for key in keys {
        if canonical_coord(key).is_none() {
            return Err(format!(
                "Unknown or unsupported physical key {key:?} (80HE ANSI)."
            ));
        }
    }
    Ok(())
}

/// Rename only the selected layer's reference. Refuse changes that would require
/// mutating another layer; the caller supplies ALL other targets, including disabled layers.
fn save_group(
    groups: &mut Groups,
    target: &mut Target,
    other_targets: &[Target],
    old: Option<&str>,
    id: &str,
    keys: &[String],
) -> Result<(), String> {
    valid_group(id, keys)?;
    if let Some(old) = old {
        if !groups.contains_key(old) {
            return Err(format!(
                "Group {old:?} no longer exists; reopen the editor."
            ));
        }
        if old != id && other_targets.iter().any(|t| references(t, old)) {
            return Err(
                "Cannot rename a group used by another layer; its references must be preserved."
                    .into(),
            );
        }
    }
    if old != Some(id) && groups.contains_key(id) {
        return Err(format!(
            "Group {id:?} already exists; choose Edit to update it."
        ));
    }
    if old.is_none() && groups.len() >= MAX_GROUPS {
        return Err("At most 64 custom groups are supported.".into());
    }
    if let Some(old) = old {
        groups.remove(old);
        if let Target::Selection { groups: refs, .. } = target {
            for name in refs.iter_mut().filter(|name| name.as_str() == old) {
                *name = id.to_owned();
            }
        }
    }
    groups.insert(id.to_owned(), keys.to_vec());
    Ok(())
}

fn delete_group(
    groups: &mut Groups,
    target: &Target,
    others: &[Target],
    id: &str,
) -> Result<(), String> {
    if references(target, id) || others.iter().any(|t| references(t, id)) {
        return Err(
            "Cannot delete a group while any layer references it. Uncheck those references first."
                .into(),
        );
    }
    if groups.remove(id).is_none() {
        return Err(format!("Unknown group {id:?}."));
    }
    Ok(())
}

fn named_coverage(
    target: &Target,
    groups: &Groups,
    layout: &KeyboardLayout,
) -> Result<Vec<String>, String> {
    let mask = target_mask(target, groups)?;
    let names = names();
    let mut result = vec![];
    for key in layout.keys().iter().filter(|key| mask.contains(key)) {
        let name = names.iter().find(|name| canonical_coord(name) == Some(key.coord))
            .ok_or("Target includes unnamed matrix cells. Choose named keys before saving as a group; no cells were dropped.")?;
        result.push(name.clone());
    }
    Ok(result)
}

struct CanvasKey {
    coord: MatrixCoord,
    rect: Rect,
    name: Option<String>,
}

fn canvas_keys(layout: &KeyboardLayout, origin: Pos2, unit: f32) -> Vec<CanvasKey> {
    let names = names();
    layout
        .keys()
        .iter()
        .map(|key| CanvasKey {
            coord: key.coord,
            rect: Rect::from_min_size(
                origin + Vec2::new(key.x * unit, key.y * unit),
                Vec2::splat(unit * 0.88),
            ),
            name: names
                .iter()
                .find(|name| canonical_coord(name) == Some(key.coord))
                .cloned(),
        })
        .collect()
}

fn rectangle_hits(keys: &[CanvasKey], rect: Rect) -> Vec<String> {
    keys.iter()
        .filter(|key| rect.intersects(key.rect))
        .filter_map(|key| key.name.clone())
        .collect()
}

fn legend(name: &str) -> &str {
    match name {
        "Escape" => "Esc",
        "PrintScreen" => "Prt",
        "Backspace" => "Bksp",
        "BracketLeft" => "[",
        "BracketRight" => "]",
        "Backslash" => "\\",
        "CapsLock" => "Caps",
        "Semicolon" => ";",
        "Quote" => "'",
        "ShiftLeft" | "ShiftRight" => "Shift",
        "ControlLeft" | "ControlRight" => "Ctrl",
        "MetaLeft" => "Meta",
        "AltLeft" | "AltRight" => "Alt",
        "PageUp" => "PgUp",
        "PageDown" => "PgDn",
        "ArrowUp" => "↑",
        "ArrowLeft" => "←",
        "ArrowDown" => "↓",
        "ArrowRight" => "→",
        "Minus" => "−",
        "Equal" => "=",
        "Grave" => "`",
        "Comma" => ",",
        "Period" => ".",
        "Slash" => "/",
        _ => name,
    }
}

struct GroupDraft {
    old: Option<String>,
    id: String,
    target: Target,
}

/// Explicit click semantics for the one composed keyboard.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum CanvasMode {
    #[default]
    Preview,
    SelectKeys,
    Inspect,
}

impl CanvasMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Preview => "View",
            Self::SelectKeys => "Select keys",
            Self::Inspect => "Inspect key",
        }
    }

    pub fn help(self) -> &'static str {
        match self {
            Self::Preview => {
                "View the lighting without editing. Clicking keys does nothing. Choose Select keys to change the layer's keys, or Inspect key to explain a color."
            }
            Self::SelectKeys => {
                "Choose keys for the selected layer. Click replaces direct keys; Shift/Ctrl/⌘-click toggles; drag selects; modifier-drag adds. Zones/groups stay unioned. Changes stay in the draft until Save."
            }
            Self::Inspect => {
                "Click a key to see why it has its current color. This only opens an explanation; it does not change the layer's keys or the activity being previewed."
            }
        }
    }
}

/// The editor owns this selected-layer context; the canvas never owns a draft.
pub struct SelectionContext<'a> {
    pub target: &'a mut Target,
    pub groups: &'a Groups,
    pub device: &'a DeviceInfo,
}

#[derive(Default, Debug, PartialEq, Eq)]
pub struct CanvasResponse {
    pub inspected: Option<MatrixCoord>,
    /// Only a committed target edit, never hover, inspection, or group drafting.
    pub changed: bool,
}

/// Read-only compatibility view for sharing. Uses the same RGB canvas renderer.
pub fn composition_preview(
    ui: &mut egui::Ui,
    frame: &underglow::render::Frame,
    selected: Option<MatrixCoord>,
    coverage: Option<&underglow::explanations::LayerCoverage>,
) -> Option<MatrixCoord> {
    TargetSelection::default()
        .show_canvas(ui, frame, CanvasMode::Inspect, None, selected, coverage)
        .inspected
}

/// Reuse the physical-name resolver; unnamed cells remain honest coordinates.
pub fn key_label(coord: MatrixCoord) -> String {
    names()
        .into_iter()
        .find(|name| canonical_coord(name) == Some(coord))
        .unwrap_or_else(|| format!("row {} / column {}", coord.row, coord.column))
}

/// Keep one instance per stable layer ID; call cancel_interaction on selection
/// changes before rendering the new layer. Group drafts remain local UI state.
#[derive(Default)]
pub struct TargetSelection {
    drag: Option<(Pos2, bool)>,
    draft: Option<GroupDraft>,
    error: Option<String>,
    last_mode: Option<CanvasMode>,
    blocked_pointer: bool,
    pub(crate) options_open: bool,
}

impl TargetSelection {
    /// Drop an in-flight gesture when switching layers, modes, or documents.
    /// Draft group members are intentionally preserved.
    pub fn cancel_interaction(&mut self) {
        self.drag = None;
        self.blocked_pointer = true;
    }

    /// Returns true only when target/groups changed, not for hover/draft edits.
    /// `other_targets` MUST contain every other layer, even disabled ones, for
    /// reference-safe group operations. `device` may be synthetic_80he().
    pub fn show_controls(
        &mut self,
        ui: &mut egui::Ui,
        target: &mut Target,
        groups: &mut Groups,
        other_targets: &[Target],
        device: &DeviceInfo,
    ) -> bool {
        if let Err(error) = validate_device(device) {
            ui.colored_label(Color32::LIGHT_RED, error);
            ui.small("Targets were not changed. Only the 80HE ANSI physical map is supported.");
            return false;
        }
        let before = (target.clone(), groups.clone());
        let layout = KeyboardLayout::for_device(device);
        ui.small("Select keys on the keyboard above.");

        if self.draft.is_none() {
            if let Target::Selection { keys, zones, groups } = target
                && (!zones.is_empty() || !groups.is_empty()) && !keys.is_empty()
                && ui.button("Use selected keys only")
                    .on_hover_text("Remove this layer's zone and group references, keeping its direct keys. Other layers and saved groups are unchanged.")
                    .clicked() {
                zones.clear();
                groups.clear();
            }
            self.options_open = ui.collapsing("Selection options", |ui| {
            ui.horizontal_wrapped(|ui| {
                if ui
                    .selectable_label(matches!(target, Target::Whole {}), "Whole keyboard")
                    .clicked()
                {
                    *target = Target::Whole {};
                }
                if ui.button("Arrow keys (4 physical)").clicked() {
                    self.error =
                        edit_keys(target, &ARROWS.map(str::to_owned), Edit::ReplaceDirect).err();
                }
            });
            ui.collapsing("Saved zones", |ui| {
                ui.small("Zones use the engine map, including unnamed cells. Legacy Arrows contains right Ctrl and excludes ↑; use Arrow keys for the four physical arrows.");
                for (zone, label) in ZONES {
                    let mut checked = matches!(target, Target::Selection { zones, .. } if zones.contains(&zone));
                    if ui.checkbox(&mut checked, label).changed() {
                        let (zones, _, _) = selection(target);
                        zones.retain(|z| *z != zone);
                        if checked { zones.push(zone); }
                    }
                }
            });
            ui.collapsing("Reusable custom groups", |ui| {
                for (id, keys) in groups.clone() {
                    ui.horizontal_wrapped(|ui| {
                        let mut checked = references(target, &id);
                        if ui.checkbox(&mut checked, &id).changed() {
                            let (_, _, refs) = selection(target);
                            refs.retain(|name| name != &id);
                            if checked {
                                refs.push(id.clone());
                            }
                        }
                        if ui.small_button("Edit / rename").clicked() {
                            self.draft = Some(GroupDraft {
                                old: Some(id.clone()),
                                id: id.clone(),
                                target: Target::Selection {
                                    zones: vec![],
                                    keys,
                                    groups: vec![],
                                },
                            });
                        }
                        if ui.small_button("Delete").clicked() {
                            self.error = delete_group(groups, target, other_targets, &id).err();
                        }
                    });
                }
                if ui.button("New group from highlighted target").clicked() {
                    match named_coverage(target, groups, &layout) {
                        Ok(keys) => {
                            self.draft = Some(GroupDraft {
                                old: None,
                                id: String::new(),
                                target: Target::Selection {
                                    zones: vec![],
                                    keys,
                                    groups: vec![],
                                },
                            })
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
            });
            ui.small("80HE ANSI physical positions. Saved zones use the legacy engine map; groups and direct keys are combined.");
            ui.collapsing("Key selection help", |ui| {
                ui.small("Fn, Mode, right Meta, numpad and light bar are unsupported. Dots are unnamed engine matrix cells, not invented keys.");
            });
            }).openness > 0.0;
        }
        if let Some(draft) = &mut self.draft {
            ui.strong("Editing group members (draft) · layer target unchanged until Save");
            ui.text_edit_singleline(&mut draft.id);
            let mut close = false;
            ui.horizontal(|ui| {
                if ui.button("Save group").clicked() {
                    let Target::Selection { keys, .. } = &draft.target else {
                        unreachable!()
                    };
                    match save_group(
                        groups,
                        target,
                        other_targets,
                        draft.old.as_deref(),
                        &draft.id,
                        keys,
                    ) {
                        Ok(()) => {
                            close = true;
                            self.error = None;
                        }
                        Err(error) => self.error = Some(error),
                    }
                }
                if ui.button("Cancel group edit").clicked() {
                    close = true;
                }
            });
            if close {
                self.draft = None;
            }
        }
        let active = self.draft.as_ref().map_or(&*target, |draft| &draft.target);
        if let Target::Selection {
            keys, groups: refs, ..
        } = active
        {
            let mut selected = Vec::new();
            if !keys.is_empty() {
                selected.push(format!("Selected keys: {}", keys.join(", ")));
            }
            if !refs.is_empty() {
                selected.push(format!("Groups: {}", refs.join(", ")));
            }
            if !selected.is_empty() {
                ui.small(selected.join(" · "));
            }
        }
        if let Err(error) = target_mask(active, groups) {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }

        before != (target.clone(), groups.clone())
    }

    /// One persistent RGB canvas. Only SelectKeys may edit the selected target;
    /// Inspect returns a coordinate for the caller's canonical capture report.
    /// All annotations are strokes: black is a real RGB pixel, not transparency.
    #[allow(clippy::too_many_arguments)]
    pub fn show_canvas(
        &mut self,
        ui: &mut egui::Ui,
        frame: &underglow::render::Frame,
        mode: CanvasMode,
        mut selection: Option<SelectionContext<'_>>,
        inspected: Option<MatrixCoord>,
        coverage: Option<&underglow::explanations::LayerCoverage>,
    ) -> CanvasResponse {
        let mut result = CanvasResponse::default();
        if self.last_mode.is_some_and(|previous| previous != mode) {
            self.cancel_interaction();
        }
        self.last_mode = Some(mode);
        // A mode/layer switch while the button is held must not reinterpret its
        // release as a new click. An idle switch can accept the next gesture.
        let pointer_busy = ui.input(|i| i.pointer.primary_down() || i.pointer.any_released());
        if !pointer_busy {
            self.blocked_pointer = false;
        }
        let supported = selection
            .as_ref()
            .is_some_and(|s| validate_device(s.device).is_ok());
        let can_edit = mode == CanvasMode::SelectKeys && supported;
        let before = selection.as_ref().map(|s| s.target.clone());
        let layout = KeyboardLayout::for_device(&DeviceInfo::synthetic_80he());
        let width = ui.available_width().clamp(1.0, 700.0);
        let unit = width / 18.5;
        let (board, _) = ui.allocate_exact_size(Vec2::new(width, unit * 6.4), Sense::hover());
        // Explicit ID and allocation are independent of mode and layer. The
        // editor must mount this once, outside per-layer inspector ID scopes.
        let response = ui.interact(
            board,
            ui.id().with("composition-keyboard"),
            if can_edit {
                Sense::click_and_drag()
            } else {
                Sense::hover()
            },
        );
        let keys = canvas_keys(&layout, board.min + Vec2::splat(unit * 0.25), unit);
        let additive = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl || i.modifiers.command);
        let pointer = response.interact_pointer_pos();
        if can_edit && !self.blocked_pointer {
            if response.drag_started() {
                self.drag = ui
                    .input(|i| i.pointer.press_origin())
                    .map(|start| (start, additive));
            }
            let s = selection.as_mut().unwrap();
            let active = self
                .draft
                .as_mut()
                .map_or(&mut *s.target, |draft| &mut draft.target);
            if response.clicked()
                && let Some(key) =
                    pointer.and_then(|pos| keys.iter().find(|key| key.rect.contains(pos)))
            {
                if let Some(name) = &key.name {
                    self.error = edit_keys(
                        active,
                        std::slice::from_ref(name),
                        if additive {
                            Edit::Toggle
                        } else {
                            Edit::ReplaceDirect
                        },
                    )
                    .err();
                } else {
                    self.error = Some("Unnamed matrix cell: Fn/Mode/custom namespaces/light bar are not selectable physical keys.".into());
                }
            }
            if response.drag_stopped()
                && let (Some((start, add)), Some(end)) = (self.drag.take(), pointer)
            {
                self.error = edit_keys(
                    active,
                    &rectangle_hits(&keys, Rect::from_two_pos(start, end)),
                    if add { Edit::Add } else { Edit::ReplaceDirect },
                )
                .err();
            }
        } else {
            self.drag = None;
        }
        let layer_mask = selection
            .as_ref()
            .filter(|_| supported)
            .and_then(|s| target_mask(s.target, s.groups).ok());
        let draft_mask = selection.as_ref().filter(|_| supported).and_then(|s| {
            self.draft
                .as_ref()
                .and_then(|draft| target_mask(&draft.target, s.groups).ok())
        });
        for (key, position) in keys.iter().zip(layout.keys()) {
            let key_response = ui.interact(
                key.rect,
                ui.id()
                    .with(("composition-key", key.coord.row, key.coord.column)),
                if mode == CanvasMode::Inspect {
                    Sense::click()
                } else {
                    Sense::hover()
                },
            );
            if mode == CanvasMode::Inspect {
                key_response.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::WidgetType::Button,
                        ui.is_enabled(),
                        format!("Inspect {}", key_label(key.coord)),
                    )
                });
                if key_response.clicked() && !self.blocked_pointer {
                    result.inspected = Some(key.coord);
                }
            }
            let color = frame.get_coord(key.coord);
            ui.painter().rect_filled(
                key.rect,
                3,
                Color32::from_rgb(color.red, color.green, color.blue),
            );
            if layer_mask
                .as_ref()
                .is_some_and(|mask| mask.contains(position))
            {
                ui.painter().rect_stroke(
                    key.rect,
                    3,
                    Stroke::new(1.0, Color32::GOLD),
                    StrokeKind::Inside,
                );
            }
            if draft_mask
                .as_ref()
                .is_some_and(|mask| mask.contains(position))
            {
                ui.painter().rect_stroke(
                    key.rect.shrink(2.0),
                    2,
                    Stroke::new(2.0, Color32::LIGHT_BLUE),
                    StrokeKind::Inside,
                );
            }
            if let Some(layer) = coverage.filter(|layer| layer.targeted.contains(&key.coord)) {
                let visible = layer.visible.contains(&key.coord);
                let obscured = layer.obscured.contains(&key.coord);
                let accent = if visible {
                    Color32::LIGHT_GREEN
                } else if obscured {
                    Color32::LIGHT_RED
                } else {
                    Color32::LIGHT_BLUE
                };
                ui.painter().rect_stroke(
                    key.rect,
                    3,
                    Stroke::new(2.0, accent),
                    StrokeKind::Outside,
                );
                if obscured {
                    // Cross-hatching is an annotation, never substitute RGB fill.
                    let inner = key.rect.shrink(3.0);
                    ui.painter().line_segment(
                        [inner.left_bottom(), inner.right_top()],
                        Stroke::new(1.0, accent),
                    );
                    ui.painter().line_segment(
                        [inner.left_top(), inner.right_bottom()],
                        Stroke::new(1.0, accent),
                    );
                }
            }
            if inspected == Some(key.coord) {
                ui.painter().rect_stroke(
                    key.rect,
                    3,
                    Stroke::new(2.0, Color32::YELLOW),
                    StrokeKind::Inside,
                );
            }
            if let Some(name) = &key.name {
                let luminance = u32::from(color.red) * 299
                    + u32::from(color.green) * 587
                    + u32::from(color.blue) * 114;
                let ink = if luminance > 140_000 {
                    Color32::BLACK
                } else {
                    Color32::WHITE
                };
                let mut size = (unit * 0.29).clamp(8.0, 11.0);
                let measured =
                    ui.painter()
                        .layout_no_wrap(legend(name).into(), FontId::monospace(size), ink);
                // Longer physical legends fit their actual key face, without
                // reducing every short legend to the old six-pixel minimum.
                size *= ((key.rect.width() - 3.0) / measured.size().x).min(1.0);
                ui.painter().text(
                    key.rect.center(),
                    Align2::CENTER_CENTER,
                    legend(name),
                    FontId::monospace(size),
                    ink,
                );
            } else {
                // A hollow marker identifies an unnamed position without inventing
                // a physical legend or obscuring its composed RGB fill.
                ui.painter().circle_stroke(
                    key.rect.center(),
                    unit * 0.07,
                    Stroke::new(1.0, Color32::GRAY),
                );
            }
            key_response.on_hover_text(format!(
                "{} · row {} col {}{}",
                key.name
                    .as_deref()
                    .unwrap_or("Unnamed matrix position · not a selectable physical key"),
                key.coord.row,
                key.coord.column,
                if layer_mask.as_ref().is_some_and(|m| m.contains(position)) {
                    " · targeted"
                } else {
                    ""
                },
            ));
        }
        if let (Some((start, _)), Some(end)) = (self.drag, pointer) {
            ui.painter().rect_stroke(
                Rect::from_two_pos(start, end),
                0,
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Inside,
            );
        }
        let mut caption = match mode {
            CanvasMode::SelectKeys if selection.is_none() => "Select keys · choose a layer".into(),
            CanvasMode::SelectKeys if !supported => "Select keys · unsupported map".into(),
            CanvasMode::SelectKeys if self.draft.is_some() => {
                "Select keys · group draft · 80HE ANSI".into()
            }
            _ => format!("{} · 80HE ANSI", mode.label()),
        };
        let mut help = format!(
            "{}\n80HE ANSI physical keys only; Fn, Mode, right Meta, numpad and light bar are unsupported. Gold = layer target; blue = group draft. RGB fills are unchanged.",
            mode.help()
        );
        if self.draft.is_some() {
            help.push_str("\nEditing group draft members only. Save or cancel the draft in the Keys inspector.");
        }
        if let Some(error) = selection
            .as_ref()
            .and_then(|s| validate_device(s.device).err())
        {
            help.push('\n');
            help.push_str(&error);
        }
        let target_error = if mode == CanvasMode::SelectKeys {
            selection.as_ref().filter(|_| supported).and_then(|s| {
                let active = self
                    .draft
                    .as_ref()
                    .map_or(&*s.target, |draft| &draft.target);
                target_mask(active, s.groups).err()
            })
        } else {
            None
        };
        let error = if mode == CanvasMode::SelectKeys {
            self.error.as_deref().or(target_error.as_deref())
        } else {
            None
        };
        if let Some(error) = error {
            caption.push_str(" · target issue");
            help.push('\n');
            help.push_str(error);
        }
        // Errors must not grow the pinned pane or cover its source-facts badge.
        // Full text also remains visible in the scrollable Keys controls.
        let caption = egui::RichText::new(caption).small();
        let caption = if error.is_some() {
            caption.color(Color32::LIGHT_RED)
        } else {
            caption
        };
        ui.add(egui::Label::new(caption).truncate())
            .on_hover_text(help);
        if let Some(s) = selection.as_ref() {
            result.changed = before.as_ref().is_some_and(|before| before != s.target);
        }
        if !ui.input(|i| i.pointer.primary_down()) {
            self.blocked_pointer = false;
        }
        result
    }

    /// Transitional compatibility mount. The workspace uses show_controls in
    /// its Keys inspector and show_canvas once in its persistent center pane.
    #[cfg(test)]
    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        target: &mut Target,
        groups: &mut Groups,
        other_targets: &[Target],
        device: &DeviceInfo,
    ) -> bool {
        let controls_changed = self.show_controls(ui, target, groups, other_targets, device);
        let canvas = self.show_canvas(
            ui,
            &underglow::render::Frame::black(),
            CanvasMode::SelectKeys,
            Some(SelectionContext {
                target,
                groups,
                device,
            }),
            None,
            None,
        );
        controls_changed || canvas.changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use underglow::compositor::{self, Layer};
    use underglow::device::Layout;
    use underglow::render::{Color, Frame};

    fn keys(names: &[&str]) -> Vec<String> {
        names.iter().map(|s| (*s).to_owned()).collect()
    }
    fn direct(names: &[&str]) -> Target {
        Target::Selection {
            zones: vec![],
            keys: keys(names),
            groups: vec![],
        }
    }
    fn layout() -> KeyboardLayout {
        KeyboardLayout::for_device(&DeviceInfo::synthetic_80he())
    }

    #[test]
    fn canvas_names_are_unique_supported_and_use_runtime_geometry() {
        let layout = layout();
        let caps = canvas_keys(&layout, Pos2::new(20.0, 30.0), 10.0);
        assert_eq!(caps.len(), layout.keys().len());
        let named: Vec<_> = caps.iter().filter(|c| c.name.is_some()).collect();
        assert_eq!(named.len(), names().len());
        for (cap, position) in caps.iter().zip(layout.keys()) {
            assert_eq!(cap.coord, position.coord);
            assert_eq!(
                cap.rect.min,
                Pos2::new(20.0 + position.x * 10.0, 30.0 + position.y * 10.0)
            );
            if let Some(name) = &cap.name {
                assert_eq!(canonical_coord(name), Some(position.coord));
            }
        }
        for name in names() {
            assert_eq!(
                named
                    .iter()
                    .filter(|c| c.name.as_ref() == Some(&name))
                    .count(),
                1
            );
        }
        assert_eq!(
            caps.iter()
                .find(|c| c.name.as_deref() == Some("F1"))
                .unwrap()
                .coord,
            MatrixCoord { row: 0, column: 2 }
        );
        assert_eq!(
            caps.iter()
                .find(|c| c.name.as_deref() == Some("ControlRight"))
                .unwrap()
                .coord
                .column,
            13
        );
    }

    #[test]
    fn click_toggle_drag_preserve_union_selectors_and_aliases() {
        let mut target = Target::Selection {
            zones: vec![TargetZone::Function],
            keys: keys(&["f1"]),
            groups: keys(&["saved"]),
        };
        edit_keys(&mut target, &keys(&["F1"]), Edit::Toggle).unwrap();
        let Target::Selection {
            keys: selected,
            zones,
            groups,
        } = &target
        else {
            panic!()
        };
        assert!(selected.is_empty());
        assert_eq!(*zones, vec![TargetZone::Function]);
        assert_eq!(*groups, keys(&["saved"]));
        edit_keys(&mut target, &keys(&["F2", "F3"]), Edit::ReplaceDirect).unwrap();
        edit_keys(&mut target, &keys(&["f2", "F4"]), Edit::Add).unwrap();
        assert_eq!(selection(&mut target).1, &keys(&["F2", "F3", "F4"]));
        let before = target.clone();
        edit_keys(&mut target, &[], Edit::ReplaceDirect).unwrap();
        assert_eq!(target, before);
        assert!(edit_keys(&mut target, &keys(&["Fn"]), Edit::ReplaceDirect).is_err());
        assert_eq!(target, before);
        let caps = canvas_keys(&layout(), Pos2::ZERO, 10.0);
        let f1 = caps
            .iter()
            .find(|c| c.name.as_deref() == Some("F1"))
            .unwrap();
        let f4 = caps
            .iter()
            .find(|c| c.name.as_deref() == Some("F4"))
            .unwrap();
        let forward = rectangle_hits(&caps, Rect::from_two_pos(f1.rect.min, f4.rect.max));
        let reverse = rectangle_hits(&caps, Rect::from_two_pos(f4.rect.max, f1.rect.min));
        assert_eq!(forward, keys(&["F1", "F2", "F3", "F4"]));
        assert_eq!(forward, reverse);
        assert_eq!(rectangle_hits(&caps, Rect::EVERYTHING).len(), names().len());
    }

    #[test]
    fn arrows_subset_saved_groups_and_union_match_composited_pixels() {
        let layout = layout();
        let mut groups = Groups::new();
        let mut target = Target::Whole {};
        edit_keys(&mut target, &ARROWS.map(str::to_owned), Edit::ReplaceDirect).unwrap();
        assert_eq!(named_coverage(&target, &groups, &layout).unwrap().len(), 4);
        edit_keys(&mut target, &keys(&["F1", "F3", "F4"]), Edit::ReplaceDirect).unwrap();
        let members = named_coverage(&target, &groups, &layout).unwrap();
        save_group(
            &mut groups,
            &mut target,
            &[],
            None,
            "function-subset",
            &members,
        )
        .unwrap();
        target = Target::Selection {
            zones: vec![TargetZone::Navigation],
            keys: keys(&["F1", "ArrowUp"]),
            groups: keys(&["function-subset"]),
        };
        let mask = target_mask(&target, &groups).unwrap();
        let mut overlay = Frame::black();
        for key in layout.keys() {
            overlay.set_coord(key.coord, Color::new(42, 0, 0));
        }
        let composed = compositor::compose(
            Frame::black(),
            &layout,
            &[Layer {
                opacity: 255,
                blend: Default::default(),
                frame: overlay,
                mask: mask.clone(),
                priority: 1,
                expires: None,
            }],
            Duration::ZERO,
        );
        for key in layout.keys() {
            assert_eq!(composed.get_coord(key.coord).red == 42, mask.contains(key));
        }
        let arrow_up = layout
            .keys()
            .iter()
            .find(|key| key.coord == canonical_coord("ArrowUp").unwrap())
            .unwrap();
        assert!(mask.contains(arrow_up));
        assert!(
            target_mask(
                &Target::Selection {
                    zones: vec![],
                    keys: vec![],
                    groups: vec![]
                },
                &groups
            )
            .is_err()
        );
        assert!(
            named_coverage(&Target::Whole {}, &groups, &layout)
                .unwrap_err()
                .contains("no cells were dropped")
        );
        assert!(target_mask(&direct(&["Fn"]), &groups).is_err());
        assert!(
            target_mask(
                &Target::Selection {
                    zones: vec![],
                    keys: vec![],
                    groups: keys(&["missing"])
                },
                &groups
            )
            .is_err()
        );
    }

    #[test]
    fn group_crud_is_atomic_and_never_dangles_other_layer_references() {
        let mut groups = Groups::new();
        let mut target = direct(&["F1"]);
        save_group(&mut groups, &mut target, &[], None, "first", &keys(&["F1"])).unwrap();
        selection(&mut target).2.push("first".into());
        let before = (groups.clone(), target.clone());
        for (old, id, members) in [
            (None, "first", keys(&["F2"])),
            (None, "bad name", keys(&["F2"])),
            (None, "second", keys(&["Fn"])),
            (None, "empty", vec![]),
        ] {
            assert!(save_group(&mut groups, &mut target, &[], old, id, &members).is_err());
            assert_eq!((groups.clone(), target.clone()), before);
        }
        assert!(
            save_group(
                &mut groups,
                &mut target,
                std::slice::from_ref(&before.1),
                Some("first"),
                "renamed",
                &keys(&["F2"])
            )
            .is_err()
        );
        assert_eq!((groups.clone(), target.clone()), before);
        save_group(
            &mut groups,
            &mut target,
            &[],
            Some("first"),
            "renamed",
            &keys(&["F2"]),
        )
        .unwrap();
        assert!(references(&target, "renamed"));
        assert!(!groups.contains_key("first"));
        assert!(delete_group(&mut groups, &target, &[], "renamed").is_err());
        let other = target.clone();
        selection(&mut target).2.clear();
        assert!(delete_group(&mut groups, &target, &[other], "renamed").is_err());
        delete_group(&mut groups, &target, &[], "renamed").unwrap();
        assert!(groups.is_empty());
        groups = (0..MAX_GROUPS)
            .map(|n| (format!("g{n}"), keys(&["F1"])))
            .collect();
        assert!(
            save_group(
                &mut groups,
                &mut target,
                &[],
                None,
                "overflow",
                &keys(&["F1"])
            )
            .is_err()
        );
    }

    fn run_ui(
        ctx: &egui::Context,
        state: &mut TargetSelection,
        target: &mut Target,
        groups: &mut Groups,
        device: &DeviceInfo,
        events: Vec<egui::Event>,
        modifiers: egui::Modifiers,
    ) -> (bool, Vec<egui::epaint::ClippedShape>) {
        let mut dirty = false;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 900.0))),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    dirty = state.show(ui, target, groups, &[], device);
                });
            },
        );
        (dirty, output.shapes)
    }

    fn text_center(shapes: &[egui::epaint::ClippedShape], label: &str) -> Pos2 {
        shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Text(t) if t.galley.text() == label => {
                    Some(t.pos + t.galley.size() * 0.5)
                }
                _ => None,
            })
            .unwrap_or_else(|| panic!("missing rendered legend {label}"))
    }

    fn button(pos: Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        }
    }

    struct CanvasHarness {
        ctx: egui::Context,
        state: TargetSelection,
        target: Target,
        groups: Groups,
        device: DeviceInfo,
        frame: Frame,
        size: Vec2,
        context: bool,
        coverage: Option<underglow::explanations::LayerCoverage>,
    }

    impl CanvasHarness {
        fn new(size: Vec2) -> Self {
            let mut frame = Frame::black();
            for key in layout().keys() {
                frame.set_coord(
                    key.coord,
                    Color::new(key.coord.row * 29, key.coord.column * 11, 73),
                );
            }
            frame.set_coord(canonical_coord("F1").unwrap(), Color::BLACK);
            Self {
                ctx: egui::Context::default(),
                state: TargetSelection::default(),
                target: direct(&["F1"]),
                groups: Groups::new(),
                device: DeviceInfo::synthetic_80he(),
                frame,
                size,
                context: true,
                coverage: None,
            }
        }

        fn run(
            &mut self,
            mode: CanvasMode,
            events: Vec<egui::Event>,
            modifiers: egui::Modifiers,
        ) -> (CanvasResponse, Vec<egui::epaint::ClippedShape>) {
            let mut response = CanvasResponse::default();
            let output = self.ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                    events,
                    modifiers,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        let selection = self.context.then_some(SelectionContext {
                            target: &mut self.target,
                            groups: &self.groups,
                            device: &self.device,
                        });
                        response = self.state.show_canvas(
                            ui,
                            &self.frame,
                            mode,
                            selection,
                            canonical_coord("F1"),
                            self.coverage.as_ref(),
                        );
                    });
                },
            );
            (response, output.shapes)
        }

        fn idle(&mut self, mode: CanvasMode) -> Vec<egui::epaint::ClippedShape> {
            let (response, shapes) = self.run(mode, vec![], egui::Modifiers::NONE);
            assert_eq!(response, CanvasResponse::default());
            shapes
        }

        fn click(
            &mut self,
            mode: CanvasMode,
            pos: Pos2,
            modifiers: egui::Modifiers,
        ) -> CanvasResponse {
            assert!(
                !self
                    .run(
                        mode,
                        vec![egui::Event::PointerMoved(pos), button(pos, true, modifiers)],
                        modifiers
                    )
                    .0
                    .changed
            );
            self.run(mode, vec![button(pos, false, modifiers)], modifiers)
                .0
        }

        fn controls(
            &mut self,
            events: Vec<egui::Event>,
        ) -> (bool, Vec<egui::epaint::ClippedShape>) {
            let mut dirty = false;
            let output = self.ctx.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, self.size)),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        dirty = self.state.show_controls(
                            ui,
                            &mut self.target,
                            &mut self.groups,
                            &[],
                            &self.device,
                        );
                    });
                },
            );
            (dirty, output.shapes)
        }
    }

    fn rgb_rects(
        shapes: &[egui::epaint::ClippedShape],
        unit: f32,
    ) -> Vec<&egui::epaint::RectShape> {
        shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if (rect.rect.width() - unit * 0.88).abs() < 0.01
                        && (rect.rect.height() - unit * 0.88).abs() < 0.01
                        && rect.fill != Color32::TRANSPARENT =>
                {
                    Some(rect)
                }
                _ => None,
            })
            .collect()
    }

    #[test]
    fn shared_canvas_modes_keep_exact_composed_rgb_and_one_keyboard_at_both_sizes() {
        use underglow::explanations::{CoverageStatus, LayerCoverage};
        for size in [Vec2::new(940.0, 850.0), Vec2::new(620.0, 640.0)] {
            let mut h = CanvasHarness::new(size);
            let f1 = canonical_coord("F1").unwrap();
            h.coverage = Some(LayerCoverage {
                transition: None,
                index: 0,
                id: "layer".into(),
                name: "Layer".into(),
                status: CoverageStatus::FullyObscured,
                reason: "test".into(),
                selected_variant: None,
                targeted: vec![f1],
                contributing: vec![f1],
                visible: vec![],
                blended: vec![],
                obscured: vec![f1],
                overlapping: vec![f1],
                obscured_by: vec![1],
                conditions: vec![],
                deadline: None,
            });
            h.state.draft = Some(GroupDraft {
                old: None,
                id: "draft".into(),
                target: direct(&["F1", "F2"]),
            });
            let before = h.target.clone();
            let mut board_rects = None;
            for mode in [
                CanvasMode::Preview,
                CanvasMode::SelectKeys,
                CanvasMode::Inspect,
            ] {
                assert!(!mode.label().is_empty());
                let shapes = h.idle(mode);
                let caption = shapes
                    .iter()
                    .find_map(|shape| match &shape.shape {
                        egui::Shape::Text(text) if text.galley.text().contains("80HE ANSI") => {
                            Some(text)
                        }
                        _ => None,
                    })
                    .expect("short mode caption");
                assert!(
                    caption.galley.size().y < 20.0,
                    "the caption must stay one line"
                );
                assert!(!shapes.iter().any(|shape| matches!(&shape.shape,
                    egui::Shape::Text(text) if text.galley.text().contains(mode.help()))));
                let unit = (size.x - 16.0).min(700.0) / 18.5;
                let fills = rgb_rects(&shapes, unit);
                assert_eq!(
                    fills.len(),
                    layout().keys().len(),
                    "exactly one RGB keyboard"
                );
                let rects: Vec<_> = fills.iter().map(|rect| rect.rect).collect();
                if let Some(previous) = &board_rects {
                    assert_eq!(&rects, previous, "mode changes must not move the keyboard");
                }
                board_rects = Some(rects);
                for (rect, key) in fills.iter().zip(layout().keys()) {
                    let rgb = h.frame.get_coord(key.coord);
                    assert_eq!(rect.fill, Color32::from_rgb(rgb.red, rgb.green, rgb.blue));
                    assert!(Rect::from_min_size(Pos2::ZERO, size).contains_rect(rect.rect));
                }
                assert!(fills.iter().any(|rect| rect.fill == Color32::BLACK));
                assert_eq!(shapes.iter().filter(|shape|
                    matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "F1")
                ).count(), 1);
            }
            assert_eq!(h.target, before);
            assert!(h.groups.is_empty());
        }
    }

    #[test]
    fn shared_canvas_preview_and_inspect_are_read_only_including_unnamed_positions() {
        let mut h = CanvasHarness::new(Vec2::new(620.0, 640.0));
        let before = h.target.clone();
        let shapes = h.idle(CanvasMode::Preview);
        let f1 = text_center(&shapes, "F1");
        assert_eq!(
            h.click(CanvasMode::Preview, f1, egui::Modifiers::NONE),
            CanvasResponse::default()
        );
        h.idle(CanvasMode::Inspect);
        assert_eq!(
            h.click(CanvasMode::Inspect, f1, egui::Modifiers::NONE),
            CanvasResponse {
                inspected: canonical_coord("F1"),
                changed: false
            }
        );
        let shapes = h.idle(CanvasMode::Inspect);
        let fills = rgb_rects(&shapes, (h.size.x - 16.0) / 18.5);
        let unnamed_index = canvas_keys(&layout(), Pos2::ZERO, 1.0)
            .iter()
            .position(|key| key.name.is_none())
            .unwrap();
        let unnamed = layout().keys()[unnamed_index].coord;
        let pos = fills[unnamed_index].rect.center();
        assert_eq!(
            h.click(CanvasMode::Inspect, pos, egui::Modifiers::NONE)
                .inspected,
            Some(unnamed)
        );
        h.idle(CanvasMode::SelectKeys);
        assert!(
            !h.click(CanvasMode::SelectKeys, pos, egui::Modifiers::NONE)
                .changed
        );
        assert!(h.state.error.as_ref().unwrap().contains("Unnamed"));
        let shapes = h.idle(CanvasMode::SelectKeys);
        let issue = shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.text().contains("target issue") => {
                    Some(text)
                }
                _ => None,
            })
            .expect("target error remains discoverable in the short caption");
        assert!(
            issue.galley.size().y < 20.0,
            "errors must not grow the pinned canvas"
        );
        assert_eq!(h.target, before);
        assert!(h.groups.is_empty());
    }

    #[test]
    fn shared_canvas_selection_union_modifiers_drag_and_unsupported_contexts() {
        let mut h = CanvasHarness::new(Vec2::new(620.0, 640.0));
        h.target = Target::Selection {
            zones: vec![TargetZone::Navigation],
            keys: keys(&["F1"]),
            groups: keys(&["saved"]),
        };
        h.groups.insert("saved".into(), keys(&["ArrowUp"]));
        let shapes = h.idle(CanvasMode::SelectKeys);
        let f1 = text_center(&shapes, "F1");
        let f2 = text_center(&shapes, "F2");
        let f4 = text_center(&shapes, "F4");
        let shift = egui::Modifiers {
            shift: true,
            ..egui::Modifiers::NONE
        };
        assert!(h.click(CanvasMode::SelectKeys, f2, shift).changed);
        assert_eq!(selection(&mut h.target).1, &keys(&["F1", "F2"]));
        h.run(
            CanvasMode::SelectKeys,
            vec![egui::Event::PointerMoved(f1), button(f1, true, shift)],
            shift,
        );
        assert!(
            !h.run(
                CanvasMode::SelectKeys,
                vec![egui::Event::PointerMoved(f4)],
                shift
            )
            .0
            .changed
        );
        assert!(
            h.run(
                CanvasMode::SelectKeys,
                vec![button(f4, false, shift)],
                shift
            )
            .0
            .changed
        );
        let (zones, direct, groups) = selection(&mut h.target);
        assert_eq!(direct, &keys(&["F1", "F2", "F3", "F4"]));
        assert_eq!(zones, &vec![TargetZone::Navigation]);
        assert_eq!(groups, &keys(&["saved"]));
        let before = h.target.clone();
        h.device.layout = Layout::Iso;
        h.idle(CanvasMode::SelectKeys);
        assert!(
            !h.click(CanvasMode::SelectKeys, f2, egui::Modifiers::NONE)
                .changed
        );
        assert_eq!(h.target, before);
        h.context = false;
        h.idle(CanvasMode::SelectKeys);
        assert!(
            !h.click(CanvasMode::SelectKeys, f2, egui::Modifiers::NONE)
                .changed
        );
        assert_eq!(h.target, before);
    }

    #[test]
    fn shared_canvas_cancels_gestures_across_modes_and_selected_layers() {
        let mut h = CanvasHarness::new(Vec2::new(620.0, 640.0));
        for cancel_by_mode in [true, false] {
            let shapes = h.idle(CanvasMode::SelectKeys);
            let f1 = text_center(&shapes, "F1");
            let f4 = text_center(&shapes, "F4");
            h.run(
                CanvasMode::SelectKeys,
                vec![
                    egui::Event::PointerMoved(f1),
                    button(f1, true, egui::Modifiers::NONE),
                ],
                egui::Modifiers::NONE,
            );
            h.run(
                CanvasMode::SelectKeys,
                vec![egui::Event::PointerMoved(f4)],
                egui::Modifiers::NONE,
            );
            assert!(h.state.drag.is_some());
            if cancel_by_mode {
                h.run(CanvasMode::Preview, vec![], egui::Modifiers::NONE);
                h.run(CanvasMode::SelectKeys, vec![], egui::Modifiers::NONE);
            } else {
                h.state.cancel_interaction();
                h.target = direct(&["A"]); // editor resolves another stable layer
            }
            let before = h.target.clone();
            assert!(
                !h.run(
                    CanvasMode::SelectKeys,
                    vec![button(f4, false, egui::Modifiers::NONE)],
                    egui::Modifiers::NONE
                )
                .0
                .changed
            );
            assert_eq!(h.target, before);
            assert!(h.state.drag.is_none());
            h.idle(CanvasMode::SelectKeys);
            assert!(
                h.click(CanvasMode::SelectKeys, f4, egui::Modifiers::NONE)
                    .changed
            );
            h.target = direct(&["F1"]);
        }
    }

    #[test]
    fn fresh_status_zone_is_visible_and_direct_only_is_an_explicit_safe_action() {
        for size in [Vec2::new(940.0, 850.0), Vec2::new(620.0, 640.0)] {
            let mut h = CanvasHarness::new(size);
            h.target = underglow::templates::pi_status_layer(
                "pi".into(),
                Target::Selection {
                    zones: vec![TargetZone::Function],
                    keys: vec![],
                    groups: vec![],
                },
                None,
            )
            .target;
            let before = h.target.clone();
            h.controls(vec![]);
            let (_, shapes) = h.controls(vec![]);
            assert!(
                !shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Text(text) if text.galley.text() == "Use selected keys only")),
                "no meaningless disabled action without direct keys"
            );
            assert_eq!(
                h.target, before,
                "no direct keys must never become an empty or whole target"
            );

            h.groups.insert("shared".into(), keys(&["F4"]));
            if let Target::Selection { groups, .. } = &mut h.target {
                groups.push("shared".into());
            }
            edit_keys(&mut h.target, &keys(&["F1", "F2"]), Edit::ReplaceDirect).unwrap();
            let groups = h.groups.clone();
            let (_, shapes) = h.controls(vec![]);
            let only = text_center(&shapes, "Use selected keys only");
            h.controls(vec![
                egui::Event::PointerMoved(only),
                button(only, true, egui::Modifiers::NONE),
            ]);
            assert!(
                h.controls(vec![button(only, false, egui::Modifiers::NONE)])
                    .0
            );
            assert_eq!(h.target, direct(&["F1", "F2"]));
            assert_eq!(
                h.groups, groups,
                "clearing this layer's references never edits shared groups"
            );
        }
    }

    #[test]
    fn shared_canvas_group_drafts_use_outlines_and_controls_have_no_keyboard() {
        let mut h = CanvasHarness::new(Vec2::new(940.0, 850.0));
        h.state.draft = Some(GroupDraft {
            old: None,
            id: "subset".into(),
            target: direct(&["F1"]),
        });
        let shapes = h.idle(CanvasMode::SelectKeys);
        let f2 = text_center(&shapes, "F2");
        assert!(
            !h.click(CanvasMode::SelectKeys, f2, egui::Modifiers::NONE)
                .changed
        );
        assert_eq!(h.target, direct(&["F1"]));
        assert!(h.groups.is_empty());
        assert_eq!(h.state.draft.as_ref().unwrap().target, direct(&["F2"]));
        let (dirty, shapes) = h.controls(vec![]);
        assert!(!dirty);
        assert!(!shapes.iter().any(
            |shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text() == "F1")
        ));
        let save = text_center(&shapes, "Save group");
        h.controls(vec![
            egui::Event::PointerMoved(save),
            button(save, true, egui::Modifiers::NONE),
        ]);
        assert!(
            h.controls(vec![button(save, false, egui::Modifiers::NONE)])
                .0
        );
        assert_eq!(h.groups["subset"], keys(&["F2"]));
        assert_eq!(h.target, direct(&["F1"]));
        assert!(h.state.draft.is_none());
        h.state.draft = Some(GroupDraft {
            old: Some("subset".into()),
            id: "renamed".into(),
            target: direct(&["A"]),
        });
        let (_, shapes) = h.controls(vec![]);
        let cancel = text_center(&shapes, "Cancel group edit");
        h.controls(vec![
            egui::Event::PointerMoved(cancel),
            button(cancel, true, egui::Modifiers::NONE),
        ]);
        assert!(
            !h.controls(vec![button(cancel, false, egui::Modifiers::NONE)])
                .0
        );
        assert_eq!(h.groups["subset"], keys(&["F2"]));
        assert!(!h.groups.contains_key("renamed"));
        assert!(h.state.draft.is_none());
    }

    #[test]
    fn headless_widget_click_multiselect_drag_and_dirty_contract() {
        let ctx = egui::Context::default();
        let mut state = TargetSelection::default();
        let mut target = direct(&["Escape"]);
        let mut groups = Groups::new();
        let device = DeviceInfo::synthetic_80he();
        let none = egui::Modifiers::default();
        // First pass measures fonts; subsequent pass has stable widget hitboxes.
        run_ui(
            &ctx,
            &mut state,
            &mut target,
            &mut groups,
            &device,
            vec![],
            none,
        );
        let (dirty, shapes) = run_ui(
            &ctx,
            &mut state,
            &mut target,
            &mut groups,
            &device,
            vec![],
            none,
        );
        assert!(!dirty);
        let f1 = text_center(&shapes, "F1");
        run_ui(
            &ctx,
            &mut state,
            &mut target,
            &mut groups,
            &device,
            vec![egui::Event::PointerMoved(f1), button(f1, true, none)],
            none,
        );
        let (dirty, _) = run_ui(
            &ctx,
            &mut state,
            &mut target,
            &mut groups,
            &device,
            vec![button(f1, false, none)],
            none,
        );
        assert!(dirty);
        assert_eq!(target, direct(&["F1"]));
        let (_, shapes) = run_ui(
            &ctx,
            &mut state,
            &mut target,
            &mut groups,
            &device,
            vec![],
            none,
        );
        let f3 = text_center(&shapes, "F3");
        let shift = egui::Modifiers {
            shift: true,
            ..none
        };
        run_ui(
            &ctx,
            &mut state,
            &mut target,
            &mut groups,
            &device,
            vec![egui::Event::PointerMoved(f3), button(f3, true, shift)],
            shift,
        );
        assert!(
            run_ui(
                &ctx,
                &mut state,
                &mut target,
                &mut groups,
                &device,
                vec![button(f3, false, shift)],
                shift
            )
            .0
        );
        assert_eq!(target, direct(&["F1", "F3"]));
        let (_, shapes) = run_ui(
            &ctx,
            &mut state,
            &mut target,
            &mut groups,
            &device,
            vec![],
            none,
        );
        let f1 = text_center(&shapes, "F1");
        let f4 = text_center(&shapes, "F4");
        run_ui(
            &ctx,
            &mut state,
            &mut target,
            &mut groups,
            &device,
            vec![egui::Event::PointerMoved(f1), button(f1, true, none)],
            none,
        );
        assert!(
            !run_ui(
                &ctx,
                &mut state,
                &mut target,
                &mut groups,
                &device,
                vec![egui::Event::PointerMoved(f4)],
                none
            )
            .0
        );
        assert!(
            run_ui(
                &ctx,
                &mut state,
                &mut target,
                &mut groups,
                &device,
                vec![button(f4, false, none)],
                none
            )
            .0
        );
        assert_eq!(target, direct(&["F1", "F2", "F3", "F4"]));
        let before = target.clone();
        let mut unsupported = device.clone();
        unsupported.layout = Layout::Iso;
        assert!(
            !run_ui(
                &ctx,
                &mut state,
                &mut target,
                &mut groups,
                &unsupported,
                vec![],
                none
            )
            .0
        );
        assert_eq!(target, before);
        state.draft = Some(GroupDraft {
            old: None,
            id: "draft".into(),
            target: direct(&["A"]),
        });
        assert!(
            !run_ui(
                &ctx,
                &mut state,
                &mut target,
                &mut groups,
                &device,
                vec![],
                none
            )
            .0
        );
        assert!(groups.is_empty());
        assert_eq!(target, before);
    }
}
