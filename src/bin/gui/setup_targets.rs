//! Declarative target inspector. No SDK, engine commands, or real input sources.
//! The layer editor owns document validation/revision and persists dirty edits.
// Temporary until the layer inspector mounts this component (JIM-20).
#![allow(dead_code)]

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

/// Keep one instance per stable layer ID (or reset it when changing layers).
#[derive(Default)]
pub struct TargetSelection {
    drag: Option<(Pos2, bool)>,
    draft: Option<GroupDraft>,
    error: Option<String>,
}

impl TargetSelection {
    /// Returns true only when target/groups changed, not for hover/draft edits.
    /// `other_targets` MUST contain every other layer, even disabled ones, for
    /// reference-safe group operations. `device` may be synthetic_80he().
    pub fn show(
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
        ui.strong("Physical targets · 80HE ANSI");
        ui.small("Click replaces direct keys (Whole is replaced). Shift/Ctrl/⌘-click toggles; modifier-drag adds. Zones/groups remain unioned until unchecked.");
        ui.small("Physical positions, not OS characters/remaps. Fn, Mode, right Meta, numpad and light bar are unsupported. Dots are unnamed engine matrix cells, not invented keys.");
        if self.draft.is_none() {
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
            ui.collapsing("Named zones (legacy engine map)", |ui| {
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
        let layer_mask = self
            .draft
            .as_ref()
            .and_then(|_| target_mask(target, groups).ok());
        if self.draft.is_some() {
            ui.small("Blue fill: draft members · gold outline: selected layer coverage.");
        }
        let active = self
            .draft
            .as_mut()
            .map_or(&mut *target, |draft| &mut draft.target);
        let mask = target_mask(active, groups);
        if let Err(error) = &mask {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        if let Target::Selection {
            keys, groups: refs, ..
        } = active
        {
            ui.label(format!(
                "Direct: {} · Groups: {}",
                keys.join(", "),
                refs.join(", ")
            ));
        }
        let width = ui.available_width().clamp(100.0, 700.0);
        let unit = width / 18.5;
        let (board, response) =
            ui.allocate_exact_size(Vec2::new(width, unit * 6.4), Sense::click_and_drag());
        let keys = canvas_keys(&layout, board.min + Vec2::splat(unit * 0.25), unit);
        let additive = ui.input(|i| i.modifiers.shift || i.modifiers.ctrl || i.modifiers.command);
        if response.drag_started() {
            self.drag = ui
                .input(|i| i.pointer.press_origin())
                .map(|start| (start, additive));
        }
        let pointer = response.interact_pointer_pos();
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
        let mask = target_mask(active, groups).ok();
        let covered = layout
            .keys()
            .iter()
            .filter(|key| mask.as_ref().is_some_and(|m| m.contains(key)))
            .count();
        for (key, position) in keys.iter().zip(layout.keys()) {
            let selected = mask.as_ref().is_some_and(|m| m.contains(position));
            let fill = if selected {
                Color32::from_rgb(0, 100, 145)
            } else {
                Color32::from_gray(35)
            };
            let rect = if key.name.is_some() {
                key.rect
            } else {
                Rect::from_center_size(key.rect.center(), Vec2::splat(unit * 0.14))
            };
            ui.painter().rect(
                rect,
                3,
                fill,
                Stroke::new(
                    1.0,
                    if layer_mask.as_ref().is_some_and(|m| m.contains(position)) {
                        Color32::GOLD
                    } else {
                        Color32::GRAY
                    },
                ),
                StrokeKind::Inside,
            );
            if let Some(name) = &key.name {
                ui.painter().text(
                    key.rect.center(),
                    Align2::CENTER_CENTER,
                    legend(name),
                    FontId::monospace((unit * 0.21).clamp(6.0, 12.0)),
                    Color32::WHITE,
                );
            }
            if response.hover_pos().is_some_and(|p| key.rect.contains(p)) {
                response.clone().on_hover_text(format!(
                    "{} · row {} col {}{}",
                    key.name
                        .as_deref()
                        .unwrap_or("Unsupported unnamed matrix cell"),
                    key.coord.row,
                    key.coord.column,
                    if selected { " · covered" } else { "" }
                ));
            }
        }
        if let (Some((start, _)), Some(end)) = (self.drag, pointer) {
            ui.painter().rect_stroke(
                Rect::from_two_pos(start, end),
                0,
                Stroke::new(1.0, Color32::WHITE),
                StrokeKind::Inside,
            );
        }
        ui.small(format!("{covered} engine matrix positions covered (keys + dots). Overlap counts once; unchecking a key does not subtract zone/group coverage."));
        if let Some(error) = &self.error {
            ui.colored_label(Color32::LIGHT_RED, error);
        }
        before != (target.clone(), groups.clone())
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
