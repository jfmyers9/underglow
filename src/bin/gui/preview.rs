//! Hardware-free previews using runtime renderers and the canonical LED geometry.

use super::brand;
use clap::ValueEnum;
use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::time::Duration;
use underglow::catalog::{self, RendererKind};
use underglow::device::DeviceInfo;
use underglow::focus::{FocusPhase, FocusState, render_focus};
use underglow::layout::{KeyboardLayout, MatrixCoord};
use underglow::reactive::{ReactiveKind, ReactiveSimulation};
use underglow::render::{Frame, PaletteName, RenderContext};
use underglow::ripple::{RippleSimulation, hid_coord, palette_gradient, wooting_80he_geometry};

pub type RippleColors = (Option<[u8; 3]>, Option<[u8; 3]>);

#[derive(Clone, Copy)]
pub struct PreviewTiming {
    pub fps: u32,
    pub speed: u32,
}

impl Default for PreviewTiming {
    fn default() -> Self {
        Self {
            fps: 30,
            speed: underglow::animation::DEFAULT_SPEED,
        }
    }
}

pub fn fixed_color_note(mode: &str) -> Option<&'static str> {
    catalog::find(mode).and_then(|visualization| visualization.fixed_color_note)
}

pub struct KeyboardPreview {
    animation: underglow::animation::AnimationClock,
    effect_tick: Option<f64>,
    effect_seconds: f64,
    simulation: RippleSimulation,
    last_tick: Option<f64>,
    demo: bool,
    pressure: f32,
    reactive: Option<(ReactiveKind, ReactiveSimulation)>,
    reactive_last_tick: Option<f64>,
    reactive_sample: Option<ReactiveSimulation>,
    reactive_frame_tick: Option<f64>,
    reactive_demo: bool,
    reactive_pressure: f32,
}

impl Default for KeyboardPreview {
    fn default() -> Self {
        Self {
            animation: Default::default(),
            effect_tick: None,
            effect_seconds: 0.0,
            simulation: RippleSimulation::default(),
            last_tick: None,
            demo: true,
            pressure: 0.8,
            reactive: None,
            reactive_last_tick: None,
            reactive_sample: None,
            reactive_frame_tick: None,
            reactive_demo: true,
            reactive_pressure: 0.8,
        }
    }
}

impl KeyboardPreview {
    fn select_reactive(&mut self, kind: Option<ReactiveKind>) {
        if self.reactive.as_ref().map(|(kind, _)| *kind) != kind {
            // Changing modes must not retain activity from the previous simulation.
            self.reactive = kind.map(|kind| (kind, ReactiveSimulation::new(kind)));
            self.reactive_last_tick = None;
            self.reactive_sample = None;
            self.reactive_frame_tick = None;
        }
    }

    fn advance_reactive(&mut self, now: f64, fps: u32, keys: Vec<(u16, f32)>) {
        let elapsed = self
            .reactive_last_tick
            .map_or(0.0, |last| (now - last).max(0.0));
        if let Some((_, simulation)) = &mut self.reactive {
            // Input must not be discarded between rendered frames: a short tap
            // at low FPS still contributes to the next sampled image.
            simulation.advance(elapsed as f32, keys);
            self.reactive_last_tick = Some(now);
            if self
                .reactive_frame_tick
                .is_none_or(|last| now - last + 1e-9 >= 1.0 / f64::from(fps.clamp(1, 120)))
            {
                self.reactive_sample = Some(simulation.clone());
                self.reactive_frame_tick = Some(now);
            }
        }
    }

    fn effect_time(&mut self, now: f64, timing: PreviewTiming) -> f64 {
        let seconds = self
            .animation
            .set_speed(Duration::from_secs_f64(now.max(0.0)), timing.speed);
        if self
            .effect_tick
            .is_none_or(|last| now - last + 1e-9 >= 1.0 / f64::from(timing.fps.clamp(1, 120)))
        {
            self.effect_seconds = seconds;
            self.effect_tick = Some(now);
        }
        self.effect_seconds
    }

    fn show(
        &mut self,
        ui: &mut egui::Ui,
        palette: &str,
        brightness: u8,
        colors: RippleColors,
        fps: u32,
    ) {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut self.demo, "Auto demo");
            ui.add(egui::Slider::new(&mut self.pressure, 0.0..=1.0).text("Pressure"));
            if ui.button("Clear waves").clicked() {
                self.simulation = RippleSimulation::default();
                self.demo = false;
            }
        });
        ui.small("Real ripple renderer · synthetic F-key demo or click/hold letter keys below");
        let geometry = wooting_80he_geometry();
        let width = ui.available_width().min(600.0);
        let unit = width / 18.5;
        let (space, _) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), unit * 6.4), Sense::hover());
        let board = Rect::from_min_size(
            Pos2::new(space.center().x - width / 2.0, space.top()),
            Vec2::new(width, unit * 6.4),
        );
        brand::keyboard_shell(ui.painter(), board, unit * 0.12);
        let origin = board.min + Vec2::splat(unit * 0.25);
        let now = ui.input(|i| i.time);
        let mut pressures = Vec::new();
        if self.demo && now.rem_euclid(4.0) < 0.65 {
            pressures.push((0x09, self.pressure));
        }
        let mut keys = Vec::new();
        for key in &geometry {
            let rect = Rect::from_min_size(
                origin + Vec2::new(key.x * unit, key.y * unit),
                Vec2::splat(unit * 0.88),
            );
            let hid = (1..256).find(|&code| {
                hid_coord(code).is_some_and(|c| c.row == key.row && c.column == key.column)
            });
            let response = ui.interact(
                rect,
                ui.id().with(("ripple-key", key.row, key.column)),
                Sense::click_and_drag(),
            );
            if response.is_pointer_button_down_on()
                && let Some(code) = hid
            {
                pressures.push((code, self.pressure));
            }
            keys.push((rect, hid));
        }
        let interval = 1.0 / f64::from(fps.clamp(1, 120));
        let elapsed = self
            .last_tick
            .map(|last| (now - last).max(0.0))
            .unwrap_or(0.0);
        if self.last_tick.is_none() || elapsed >= interval {
            self.simulation.advance(elapsed as f32, pressures);
            self.last_tick = Some(now);
        }
        let frame = self.simulation.render(
            &geometry,
            |position| palette_gradient(palette, position),
            brightness,
            colors.0,
            colors.1,
        );
        for ((rect, hid), rgb) in keys.iter().zip(frame) {
            let color = Color32::from_rgb(rgb[0], rgb[1], rgb[2]);
            ui.painter()
                .rect_filled(rect.expand(unit * 0.055), 5, color.gamma_multiply(0.15));
            ui.painter()
                .rect_filled(rect.translate(Vec2::new(0.0, unit * 0.06)), 4, brand::BG);
            ui.painter().rect(
                *rect,
                4.0,
                color,
                Stroke::new(1.0, brand::BORDER),
                StrokeKind::Inside,
            );
            let label = match hid {
                Some(code @ 0x04..=0x1d) => char::from(b'A' + (*code as u8 - 0x04)).to_string(),
                Some(code @ 0x1e..=0x27) => ((*code - 0x1d) % 10).to_string(),
                Some(0x2c) => "SP".into(),
                Some(_) => "•".into(),
                None => String::new(),
            };
            // A fixed backing preserves contrast without flashing text colors as waves pass.
            if !label.is_empty() {
                ui.painter().rect_filled(
                    Rect::from_center_size(rect.center(), Vec2::new(unit * 0.55, unit * 0.5)),
                    2.0,
                    Color32::from_black_alpha(190),
                );
            }
            ui.painter().text(
                rect.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::monospace(unit * 0.34),
                Color32::WHITE,
            );
        }
        ui.ctx()
            .request_repaint_after(Duration::from_secs_f64(interval));
    }
}

/// Paint runtime LED colors on the shared synthetic 80HE layout.
/// This never queries a device or starts a signal/provider. Focus is synthetic.
pub fn keyboard(
    ui: &mut egui::Ui,
    mode: &str,
    palette: &str,
    brightness: u8,
    ripple_colors: RippleColors,
    timing: PreviewTiming,
    keyboard_preview: &mut KeyboardPreview,
) {
    let reactive_kind = catalog::find(mode).and_then(|v| match v.renderer {
        RendererKind::Reactive(kind) => Some(kind),
        _ => None,
    });
    keyboard_preview.select_reactive(reactive_kind);
    let Some(visualization) = catalog::find(mode) else {
        ui.label("Preview unavailable for this custom or unsupported mode.");
        ui.small("No command, provider, or hardware is started by this preview.");
        return;
    };
    if matches!(visualization.renderer, RendererKind::Ripples) {
        keyboard_preview.show(ui, palette, brightness, ripple_colors, timing.fps);
        return;
    }
    if matches!(visualization.renderer, RendererKind::Focus) {
        ui.small("Synthetic focus phase at 55% progress · not the live timer");
    }
    if reactive_kind.is_some() {
        ui.horizontal_wrapped(|ui| {
            ui.checkbox(&mut keyboard_preview.reactive_demo, "Auto demo");
            ui.add(
                egui::Slider::new(&mut keyboard_preview.reactive_pressure, 0.0..=1.0)
                    .text("Pressure"),
            );
            if ui.button("Clear activity").clicked() {
                if let Some((_, simulation)) = &mut keyboard_preview.reactive {
                    simulation.clear();
                }
                keyboard_preview.reactive_demo = false;
                keyboard_preview.reactive_last_tick = None;
                keyboard_preview.reactive_sample = None;
                keyboard_preview.reactive_frame_tick = None;
            }
        });
        ui.small(
            "Synthetic input · click/hold typing keys or Space · no real keyboard input is read",
        );
    }
    let info = preview_device();
    let layout = KeyboardLayout::for_device(&info);
    let now = ui.input(|i| i.time);
    let time = keyboard_preview.effect_time(now, timing);
    let width = ui.available_width().min(600.0);
    let unit = width / 18.5;
    let (space, _) =
        ui.allocate_exact_size(Vec2::new(ui.available_width(), unit * 6.4), Sense::hover());
    if !ui.is_rect_visible(space) {
        return;
    }
    let board = Rect::from_min_size(
        Pos2::new(space.center().x - width / 2.0, space.top()),
        Vec2::new(width, unit * 6.4),
    );
    let origin = board.min + Vec2::splat(unit * 0.25);
    if reactive_kind.is_some() {
        let mut keys = Vec::new();
        // Rotate held keys instead of sampling narrow pulses, which can alias
        // into permanent darkness at low frame rates or shifted start times.
        if keyboard_preview.reactive_demo {
            let code = [0x09, 0x0d, 0x14][now.rem_euclid(3.0).floor() as usize];
            keys.push((code, keyboard_preview.reactive_pressure));
        }
        for key in layout.keys() {
            let Some(code) = (1..256).find(|&code| {
                hid_coord(code).is_some_and(|coord| {
                    coord.row == key.coord.row && coord.column == key.coord.column
                })
            }) else {
                continue;
            };
            let rect = Rect::from_min_size(
                origin + Vec2::new(key.x * unit, key.y * unit),
                Vec2::splat(unit * 0.88),
            );
            let response = ui.interact(
                rect,
                ui.id()
                    .with(("reactive-key", key.coord.row, key.coord.column)),
                Sense::click_and_drag(),
            );
            if response.is_pointer_button_down_on() || response.clicked() {
                keys.push((code, keyboard_preview.reactive_pressure));
            }
        }
        keyboard_preview.advance_reactive(now, timing.fps, keys);
    }
    let frame = if let Some(simulation) = &keyboard_preview.reactive_sample {
        PaletteName::from_str(palette, false).ok().map(|palette| {
            simulation.render(&RenderContext {
                info: &info,
                layout: &layout,
                brightness,
                palette,
                tick: 0,
                animation_seconds: 0.0,
            })
        })
    } else {
        preview_frame(mode, palette, brightness, time, &info, &layout)
    };
    let Some(frame) = frame else {
        ui.label("Preview unavailable for this palette or renderer.");
        return;
    };
    let painter = ui.painter();
    brand::keyboard_shell(painter, board, unit * 0.12);
    for key in layout.keys() {
        let rect = Rect::from_min_size(
            origin + Vec2::new(key.x * unit, key.y * unit),
            Vec2::splat(unit * 0.88),
        );
        // Sample by LED coordinate, not display position or an illustrative wave.
        // The key face is the unmodified runtime RGB; only the outer glow is toned down.
        let color = frame_color(&frame, key.coord);
        painter.rect_filled(rect.expand(unit * 0.055), 5, color.gamma_multiply(0.15));
        painter.rect_filled(rect.translate(Vec2::new(0.0, unit * 0.06)), 4, brand::BG);
        painter.rect(
            rect,
            4,
            color,
            Stroke::new(1.0, brand::BORDER),
            StrokeKind::Inside,
        );
        let label = key_label(key.coord);
        if !label.is_empty() {
            painter.rect_filled(
                Rect::from_center_size(rect.center(), Vec2::new(unit * 0.74, unit * 0.5)),
                2,
                Color32::from_black_alpha(190),
            );
            painter.text(
                rect.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::monospace((unit * 0.23).clamp(7.0, 12.0)),
                Color32::WHITE,
            );
        }
    }
    ui.ctx().request_repaint_after(Duration::from_secs_f64(
        1.0 / f64::from(timing.fps.clamp(1, 120)),
    ));
}

/// Describes a synthetic device; constructing this value performs no SDK calls.
fn preview_device() -> DeviceInfo {
    DeviceInfo::synthetic_80he()
}

fn preview_frame(
    mode: &str,
    palette: &str,
    brightness: u8,
    animation_seconds: f64,
    info: &DeviceInfo,
    layout: &KeyboardLayout,
) -> Option<Frame> {
    let renderer = catalog::find(mode)?.renderer;
    let ctx = RenderContext {
        animation_seconds,
        info,
        layout,
        brightness,
        palette: PaletteName::from_str(palette, false).ok()?,
        tick: 0,
    };
    match renderer {
        RendererKind::Static(effect) => Some(effect.render(&ctx)),
        RendererKind::Focus => Some(render_focus(
            &ctx,
            FocusState {
                phase: FocusPhase::Focus,
                progress: 0.55,
                cycle: 1,
            },
            false,
        )),
        // Stateful ripples use KeyboardPreview's real simulation and synthetic input.
        RendererKind::Ripples | RendererKind::Reactive(_) => None,
    }
}

fn frame_color(frame: &Frame, coord: MatrixCoord) -> Color32 {
    let color = frame.get_coord(coord);
    Color32::from_rgb(color.red, color.green, color.blue)
}

// Legends annotate LED addresses; they do not supply rendering geometry.
fn key_label(coord: MatrixCoord) -> &'static str {
    const LABELS: [[&str; 17]; 6] = [
        [
            "esc", "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12", "",
            "prt", "del", "",
        ],
        [
            "`", "1", "2", "3", "4", "5", "6", "7", "8", "9", "0", "−", "=", "back", "ins", "home",
            "",
        ],
        [
            "tab", "Q", "W", "E", "R", "T", "Y", "U", "I", "O", "P", "[", "]", "\\", "pg↑", "pg↓",
            "",
        ],
        [
            "caps", "A", "S", "D", "F", "G", "H", "J", "K", "L", ";", "'", "", "enter", "", "", "",
        ],
        [
            "shift", "", "Z", "X", "C", "V", "B", "N", "M", ",", ".", "/", "", "shift", "", "↑", "",
        ],
        [
            "ctrl", "⌘", "alt", "", "", "", "SP", "", "", "", "alt", "fn", "ctrl", "", "←", "↓",
            "→",
        ],
    ];
    LABELS
        .get(usize::from(coord.row))
        .and_then(|row| row.get(usize::from(coord.column)))
        .copied()
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reactive_preview_matches_simulation_sampling_and_discards_old_activity() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);
        for &kind in ReactiveKind::value_variants() {
            let mut preview = KeyboardPreview::default();
            preview.select_reactive(Some(kind));
            let mut expected = ReactiveSimulation::new(kind);
            let keys = vec![(0x09, 0.8), (0x0d, 0.7)];
            preview.advance_reactive(0.0, 30, keys.clone());
            expected.advance(0.0, keys);
            preview.advance_reactive(0.01, 30, vec![]); // Between sampled frames.
            expected.advance(0.01, []);
            assert_eq!(preview.reactive_frame_tick, Some(0.0));
            preview.advance_reactive(0.1, 30, vec![]);
            expected.advance(0.09, []);
            for &palette in PaletteName::value_variants() {
                for brightness in [0, 73, 255] {
                    let ctx = RenderContext {
                        info: &info,
                        layout: &layout,
                        palette,
                        brightness,
                        tick: 0,
                        animation_seconds: 0.0,
                    };
                    let actual = preview.reactive.as_ref().unwrap().1.render(&ctx);
                    assert_eq!(actual, expected.render(&ctx));
                    preview.select_reactive(Some(kind));
                    assert_eq!(
                        actual,
                        preview.reactive.as_ref().unwrap().1.render(&ctx),
                        "ordinary redraw preserves state"
                    );
                }
            }
            preview.select_reactive(None);
            assert!(preview.reactive.is_none() && preview.reactive_last_tick.is_none());
            preview.select_reactive(Some(kind));
            assert_eq!(
                preview.reactive.as_ref().unwrap().1.render(&RenderContext {
                    info: &info,
                    layout: &layout,
                    palette: PaletteName::Heat,
                    brightness: 255,
                    tick: 0,
                    animation_seconds: 0.0
                }),
                Frame::black()
            );
        }
    }

    #[test]
    fn reactive_short_taps_survive_between_low_fps_frames() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 255,
            palette: PaletteName::Heat,
            tick: 0,
            animation_seconds: 0.0,
        };
        for &kind in ReactiveKind::value_variants() {
            let mut preview = KeyboardPreview::default();
            preview.select_reactive(Some(kind));
            preview.advance_reactive(0.0, 1, vec![]);
            preview.advance_reactive(0.1, 1, vec![(0x09, 1.0)]);
            preview.advance_reactive(0.2, 1, vec![]);
            assert_eq!(
                preview.reactive_sample.as_ref().unwrap().render(&ctx),
                Frame::black()
            );
            preview.advance_reactive(1.0, 1, vec![]);
            assert_ne!(
                preview.reactive_sample.as_ref().unwrap().render(&ctx),
                Frame::black(),
                "short {kind} input must survive to the sampled frame"
            );
        }
    }

    #[test]
    fn reactive_fast_click_is_captured_without_a_held_pointer_frame() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);
        let context = egui::Context::default();
        let mut preview = KeyboardPreview {
            reactive_demo: false,
            ..Default::default()
        };
        let mut paint = |now, events| {
            context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(620.0, 700.0))),
                    time: Some(now),
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        keyboard(
                            ui,
                            "heatmap",
                            "heat",
                            255,
                            (None, None),
                            PreviewTiming { fps: 1, speed: 100 },
                            &mut preview,
                        );
                    });
                },
            )
        };
        let output = paint(0.0, vec![]);
        let caps = output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Rect(rect)
                    if rect.stroke.color == brand::BORDER
                        && (rect.rect.width() - 600.0 / 18.5 * 0.88).abs() < 0.01 =>
                {
                    Some(rect.rect)
                }
                _ => None,
            })
            .collect::<Vec<_>>();
        let key = layout
            .keys()
            .iter()
            .position(|key| key.coord == MatrixCoord { row: 3, column: 4 })
            .unwrap();
        let pos = caps[key].center();
        paint(
            0.1,
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: Default::default(),
                },
                egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: Default::default(),
                },
            ],
        );
        paint(1.0, vec![]);
        let frame = preview
            .reactive_sample
            .as_ref()
            .unwrap()
            .render(&RenderContext {
                info: &info,
                layout: &layout,
                palette: PaletteName::Heat,
                brightness: 255,
                tick: 0,
                animation_seconds: 0.0,
            });
        assert_ne!(frame.get(3, 4), underglow::render::Color::BLACK);
    }

    #[test]
    fn reactive_keys_paint_shared_frames_and_offer_only_synthetic_input() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);
        for &kind in ReactiveKind::value_variants() {
            for width in [400.0, 960.0] {
                let mut preview = KeyboardPreview::default();
                let mut unit = 0.0;
                let output = egui::Context::default().run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 700.0))),
                        time: Some(0.5),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            unit = ui.available_width().min(600.0) / 18.5;
                            keyboard(
                                ui,
                                &kind.to_string(),
                                "heat",
                                81,
                                (None, None),
                                PreviewTiming { fps: 1, speed: 400 },
                                &mut preview,
                            );
                        });
                    },
                );
                let mut expected = ReactiveSimulation::new(kind);
                expected.advance(0.0, [(0x09, 0.8)]);
                let frame = expected.render(&RenderContext {
                    info: &info,
                    layout: &layout,
                    palette: PaletteName::Heat,
                    brightness: 81,
                    tick: 0,
                    animation_seconds: 0.0,
                });
                assert_ne!(frame, Frame::black());
                let caps = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.stroke.color == brand::BORDER
                                && (rect.rect.width() - unit * 0.88).abs() < 0.01 =>
                        {
                            Some(rect)
                        }
                        _ => None,
                    })
                    .collect::<Vec<_>>();
                assert_eq!(caps.len(), layout.keys().len());
                for (cap, key) in caps.iter().zip(layout.keys()) {
                    assert_eq!(cap.fill, frame_color(&frame, key.coord));
                }
                for text in ["Auto demo", "Clear activity"] {
                    assert!(output.shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(label) if label.galley.job.text == text)));
                }
                assert!(
                    preview.last_tick.is_none(),
                    "reactive previews must not run ripple input"
                );
            }
        }
    }

    #[test]
    fn catalog_frames_match_runtime_across_settings_and_times() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);
        for visualization in catalog::all() {
            let RendererKind::Static(effect) = visualization.renderer else {
                continue;
            };
            for palette in PaletteName::value_variants() {
                for brightness in [0, 1, 96, 255] {
                    for animation_seconds in [0.0, 0.125, 2.5, 17.321, 3600.25] {
                        let actual = preview_frame(
                            visualization.id,
                            &palette.to_string(),
                            brightness,
                            animation_seconds,
                            &info,
                            &layout,
                        )
                        .unwrap();
                        let expected = effect.render(&RenderContext {
                            animation_seconds,
                            info: &info,
                            layout: &layout,
                            brightness,
                            palette: *palette,
                            tick: 0,
                        });
                        assert_eq!(actual, expected, "{}", visualization.id);
                        if brightness == 0 {
                            assert_eq!(actual, Frame::black(), "{}", visualization.id);
                        }
                        for key in layout.keys() {
                            let rgb = expected.get_coord(key.coord);
                            assert_eq!(
                                frame_color(&actual, key.coord),
                                Color32::from_rgb(rgb.red, rgb.green, rgb.blue)
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn preview_geometry_and_addresses_are_the_runtime_layout() {
        let info = preview_device();
        assert!(!info.connected);
        let layout = KeyboardLayout::for_device(&info);
        assert_eq!(layout.name, "wooting-80he");
        let geometry = wooting_80he_geometry();
        assert_eq!(layout.keys().len(), geometry.len());
        let mut addresses = std::collections::HashSet::new();
        for (key, expected) in layout.keys().iter().zip(geometry) {
            assert_eq!((key.x, key.y), (expected.x, expected.y));
            assert_eq!(
                (key.coord.row, key.coord.column),
                (expected.row, expected.column)
            );
            assert!(key.coord.row < info.max_rows && key.coord.column < info.max_columns);
            assert!(addresses.insert((key.coord.row, key.coord.column)));
        }
    }

    #[test]
    fn focus_preview_uses_shared_renderer_with_explicit_synthetic_state() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);
        for brightness in [0, 96, 255] {
            let actual =
                preview_frame("focus-cockpit", "heat", brightness, 10.0, &info, &layout).unwrap();
            let expected = render_focus(
                &RenderContext {
                    animation_seconds: 10.0,
                    info: &info,
                    layout: &layout,
                    brightness,
                    palette: PaletteName::Heat,
                    tick: 0,
                },
                FocusState {
                    phase: FocusPhase::Focus,
                    progress: 0.55,
                    cycle: 1,
                },
                false,
            );
            assert_eq!(actual, expected);
            if brightness == 0 {
                assert_eq!(actual, Frame::black());
            }
        }
    }

    #[test]
    fn unsupported_modes_and_palettes_do_not_substitute_an_effect() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);
        for mode in ["unknown", "command-status", "custom-program"] {
            assert!(preview_frame(mode, "ocean", 255, 2.0, &info, &layout).is_none());
        }
        assert!(preview_frame("comet", "unknown", 255, 2.0, &info, &layout).is_none());
        let mut preview = KeyboardPreview::default();
        let output = egui::Context::default().run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                keyboard(
                    ui,
                    "custom-program",
                    "ocean",
                    255,
                    (None, None),
                    PreviewTiming::default(),
                    &mut preview,
                );
            });
        });
        assert!(output.shapes.iter().any(|shape| matches!(
            &shape.shape,
            egui::Shape::Text(text) if text.galley.job.text.contains("Preview unavailable")
        )));
        assert!(preview.effect_tick.is_none());
        assert!(preview.last_tick.is_none());
    }

    #[test]
    fn painted_keys_preserve_runtime_colors_coordinates_and_white_legends() {
        let info = preview_device();
        let layout = KeyboardLayout::for_device(&info);
        for visualization in catalog::all() {
            if !matches!(
                visualization.renderer,
                RendererKind::Static(_) | RendererKind::Focus
            ) {
                continue;
            }
            for width in [400.0, 960.0] {
                let mut preview = KeyboardPreview::default();
                preview.effect_time(0.0, PreviewTiming::default());
                let mut origin = Pos2::ZERO;
                let mut unit = 0.0;
                let output = egui::Context::default().run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 600.0))),
                        time: Some(2.5),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            unit = ui.available_width().min(600.0) / 18.5;
                            keyboard(
                                ui,
                                visualization.id,
                                "ocean",
                                180,
                                (None, None),
                                PreviewTiming::default(),
                                &mut preview,
                            );
                        });
                    },
                );
                let frame =
                    preview_frame(visualization.id, "ocean", 180, 2.5, &info, &layout).unwrap();
                let caps: Vec<_> = output
                    .shapes
                    .iter()
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.stroke.color == brand::BORDER
                                && (rect.rect.width() - unit * 0.88).abs() < 0.01 =>
                        {
                            Some(rect)
                        }
                        _ => None,
                    })
                    .collect();
                assert_eq!(caps.len(), layout.keys().len(), "{}", visualization.id);
                for (index, (cap, key)) in caps.iter().zip(layout.keys()).enumerate() {
                    if index == 0 {
                        origin = cap.rect.min;
                    }
                    assert!((cap.rect.min.x - origin.x - key.x * unit).abs() < 0.01);
                    assert!((cap.rect.min.y - origin.y - key.y * unit).abs() < 0.01);
                    assert_eq!(cap.fill, frame_color(&frame, key.coord));
                }
                let f_legend = output.shapes.iter().find_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) if text.galley.job.text == "F" => {
                        Some(text.galley.job.sections[0].format.color)
                    }
                    _ => None,
                });
                assert_eq!(f_legend, Some(Color32::WHITE));
            }
        }
    }

    #[test]
    fn preview_speed_is_independent_of_fps_and_changes_without_resetting() {
        for fps in [5, 10, 30, 60, 120] {
            let mut preview = KeyboardPreview::default();
            let mut timing = PreviewTiming { fps, speed: 100 };
            for tick in 0..=fps * 2 {
                preview.effect_time(f64::from(tick) / f64::from(fps), timing);
            }
            assert_eq!(preview.effect_time(2.0, timing), 2.0);
            timing.speed = 50;
            assert_eq!(preview.effect_time(2.0, timing), 2.0);
            assert_eq!(preview.effect_time(4.0, timing), 3.0);
            timing.fps = 120;
            assert_eq!(preview.effect_time(6.0, timing), 4.0);
        }
    }

    #[test]
    fn preview_holds_samples_until_the_selected_frame_interval() {
        let mut preview = KeyboardPreview::default();
        let timing = PreviewTiming {
            fps: 10,
            speed: 100,
        };
        assert_eq!(preview.effect_time(0.0, timing), 0.0);
        assert_eq!(preview.effect_time(0.05, timing), 0.0);
        assert_eq!(preview.effect_time(0.1, timing), 0.1);
        assert_eq!(preview.effect_time(0.15, timing), 0.1);
        assert_eq!(preview.effect_time(0.2, timing), 0.2);
    }

    #[test]
    fn fixed_color_previews_ignore_palette_but_palette_effects_respond() {
        let render = |mode: &str, palette: &str| {
            let mut preview = KeyboardPreview::default();
            // The real breath starts black. Advance past its first sample before
            // asserting that changing its palette changes visible LED colors.
            preview.effect_time(0.0, PreviewTiming::default());
            egui::Context::default()
                .run(
                    egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 600.0))),
                        time: Some(2.5),
                        ..Default::default()
                    },
                    |ctx| {
                        egui::CentralPanel::default().show(ctx, |ui| {
                            keyboard(
                                ui,
                                mode,
                                palette,
                                255,
                                (None, None),
                                PreviewTiming::default(),
                                &mut preview,
                            );
                        });
                    },
                )
                .shapes
        };
        for mode in ["rainbow", "matrix", "focus-cockpit"] {
            assert!(render(mode, "ocean") == render(mode, "heat"), "{mode}");
        }
        for mode in ["comet", "breath"] {
            assert!(render(mode, "ocean") != render(mode, "heat"), "{mode}");
        }
    }

    #[test]
    fn preview_paints_without_a_window_or_device() {
        for width in [400.0, 960.0] {
            let context = egui::Context::default();
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 500.0))),
                    time: Some(2.5),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        keyboard(
                            ui,
                            "ripples",
                            "ocean",
                            180,
                            (Some([0, 32, 64]), Some([120, 255, 255])),
                            PreviewTiming::default(),
                            &mut KeyboardPreview::default(),
                        );
                    });
                },
            );
            assert!(output.shapes.len() > 200);
        }
    }

    #[test]
    fn two_tone_preview_keeps_base_between_waves() {
        let base = [10, 40, 90];
        let wave = [200, 120, 40];
        let mut preview = KeyboardPreview::default();
        let geometry = wooting_80he_geometry();
        let render = |preview: &KeyboardPreview| {
            preview.simulation.render(
                &geometry,
                |p| palette_gradient("ocean", p),
                255,
                Some(base),
                Some(wave),
            )
        };
        assert!(render(&preview).iter().all(|rgb| *rgb == base));
        preview.simulation.advance(0.0, [(0x09, 1.0)]);
        let origin = hid_coord(0x09).unwrap();
        let index = geometry
            .iter()
            .position(|key| key.row == origin.row && key.column == origin.column)
            .unwrap();
        assert_eq!(render(&preview)[index], wave);
        preview.simulation.advance(3.0, []);
        assert!(render(&preview).iter().all(|rgb| *rgb == base));
    }

    #[test]
    fn ripple_legends_keep_their_color_as_waves_pass() {
        let context = egui::Context::default();
        let mut preview = KeyboardPreview::default();
        for time in [1.0, 4.0, 4.3] {
            let output = context.run(
                egui::RawInput {
                    screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(960.0, 600.0))),
                    time: Some(time),
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        keyboard(
                            ui,
                            "ripples",
                            "ocean",
                            255,
                            (Some([0; 3]), Some([255; 3])),
                            PreviewTiming::default(),
                            &mut preview,
                        );
                    });
                },
            );
            let legend = output.shapes.iter().find_map(|shape| match &shape.shape {
                egui::Shape::Text(text) if text.galley.job.text == "F" => {
                    Some(text.galley.job.sections[0].format.color)
                }
                _ => None,
            });
            assert_eq!(legend, Some(Color32::WHITE));
            for shape in &output.shapes {
                if let egui::Shape::Rect(key) = &shape.shape
                    && key.stroke.color == Color32::from_gray(65)
                {
                    assert!(
                        key.rect.width() < 30.0,
                        "matrix must not expand to the entire desktop width"
                    );
                }
            }
        }
    }
}
