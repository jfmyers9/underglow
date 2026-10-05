//! Hardware-free previews. Ripples use the real shared renderer with synthetic input.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::time::Duration;
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
    match mode {
        "rainbow" => Some("Spectrum uses a fixed rainbow, not a palette."),
        "matrix" => Some("Matrix uses the fixed Terminal green palette."),
        "focus-cockpit" => Some(
            "Focus uses phase colors; the preview illustrates the blue focus phase, not the live timer.",
        ),
        _ => None,
    }
}

pub struct KeyboardPreview {
    animation: underglow::animation::AnimationClock,
    effect_tick: Option<f64>,
    effect_seconds: f64,
    simulation: RippleSimulation,
    last_tick: Option<f64>,
    demo: bool,
    pressure: f32,
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
        }
    }
}

impl KeyboardPreview {
    fn effect_time(&mut self, now: f64, timing: PreviewTiming) -> f32 {
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
        self.effect_seconds as f32
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
        let origin = Pos2::new(space.center().x - width / 2.0, space.top());
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
            ui.painter().rect(
                *rect,
                4.0,
                color,
                Stroke::new(1.0, Color32::from_gray(65)),
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

/// Paint an animated, illustrative 80%-style keyboard at the available width.
/// `brightness` is 0–255. The caller must label this as a simulated preview.
pub fn keyboard(
    ui: &mut egui::Ui,
    mode: &str,
    palette: &str,
    brightness: u8,
    ripple_colors: RippleColors,
    timing: PreviewTiming,
    keyboard_preview: &mut KeyboardPreview,
) {
    if mode == "ripples" {
        keyboard_preview.show(ui, palette, brightness, ripple_colors, timing.fps);
        return;
    }
    let width = ui.available_width().clamp(400.0, 600.0);
    let unit = width / 18.7;
    let (space, _) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(width), unit * 7.8),
        Sense::hover(),
    );
    if !ui.is_rect_visible(space) {
        return;
    }
    let painter = ui.painter();
    let origin = Pos2::new(space.center().x - width / 2.0, space.top() + unit * 0.25);
    let board = Rect::from_min_size(origin, Vec2::new(width, unit * 7.05));
    let time = keyboard_preview.effect_time(ui.input(|i| i.time), timing);
    let accent = effect_color(mode, palette, 0.25);

    // Layered chassis and a narrow underside highlight give the board depth
    // without overwhelming the key illumination with a neon frame.
    painter.rect_filled(
        board.translate(Vec2::new(0.0, unit * 0.17)),
        14.0,
        Color32::from_black_alpha(70),
    );
    painter.rect(
        board,
        14.0,
        Color32::from_rgb(16, 22, 27),
        Stroke::new(1.0, Color32::from_rgb(48, 59, 64)),
        StrokeKind::Inside,
    );
    painter.rect_filled(
        board.shrink(unit * 0.22),
        9.0,
        Color32::from_rgb(10, 15, 19),
    );
    let base = origin + Vec2::splat(unit * 0.55);

    let key = |x: f32, y: f32, w: f32, label: &str| {
        let rect = Rect::from_min_size(
            base + Vec2::new(x * unit, y * unit),
            Vec2::new(w * unit - unit * 0.12, unit * 0.86),
        );
        let (strength, hue) = illumination(mode, x + w * 0.5, y, time);
        let (color, strength) = (
            effect_color(mode, palette, hue),
            strength * f32::from(brightness) / 255.0,
        );
        if strength > 0.05 {
            for (spread, alpha) in [(0.11, 13.0), (0.055, 27.0)] {
                painter.rect_filled(
                    rect.expand(unit * spread),
                    5.0,
                    color.gamma_multiply(strength * alpha / 255.0),
                );
            }
        }
        painter.rect_filled(
            rect.translate(Vec2::new(0.0, unit * 0.065)),
            4.0,
            Color32::from_rgb(4, 8, 11),
        );
        painter.rect(
            rect,
            4.0,
            mix(Color32::from_rgb(25, 33, 39), color, strength * 0.28),
            Stroke::new(
                0.8,
                mix(Color32::from_rgb(45, 56, 63), color, strength * 0.78),
            ),
            StrokeKind::Inside,
        );
        let cap = rect.shrink(unit * 0.09);
        painter.line_segment(
            [cap.left_top(), cap.right_top()],
            Stroke::new(
                0.6,
                mix(Color32::from_rgb(55, 65, 72), color, strength * 0.5),
            ),
        );
        painter.text(
            rect.center() - Vec2::new(0.0, unit * 0.035),
            Align2::CENTER_CENTER,
            label,
            FontId::monospace((unit * 0.23).clamp(7.0, 12.0)),
            mix(Color32::from_rgb(133, 148, 157), color, strength * 0.85),
        );
    };

    key(0.0, 0.0, 1.0, "esc");
    for (i, label) in [
        "F1", "F2", "F3", "F4", "F5", "F6", "F7", "F8", "F9", "F10", "F11", "F12",
    ]
    .iter()
    .enumerate()
    {
        key(1.6 + i as f32 + (i / 4) as f32 * 0.35, 0.0, 1.0, label);
    }
    key(15.35, 0.0, 1.0, "prt");
    key(16.35, 0.0, 1.0, "del");

    let rows: &[&[(&str, f32)]] = &[
        &[
            ("`", 1.0),
            ("1", 1.0),
            ("2", 1.0),
            ("3", 1.0),
            ("4", 1.0),
            ("5", 1.0),
            ("6", 1.0),
            ("7", 1.0),
            ("8", 1.0),
            ("9", 1.0),
            ("0", 1.0),
            ("−", 1.0),
            ("=", 1.0),
            ("back", 2.0),
        ],
        &[
            ("tab", 1.5),
            ("Q", 1.0),
            ("W", 1.0),
            ("E", 1.0),
            ("R", 1.0),
            ("T", 1.0),
            ("Y", 1.0),
            ("U", 1.0),
            ("I", 1.0),
            ("O", 1.0),
            ("P", 1.0),
            ("[", 1.0),
            ("]", 1.0),
            ("\\", 1.5),
        ],
        &[
            ("caps", 1.75),
            ("A", 1.0),
            ("S", 1.0),
            ("D", 1.0),
            ("F", 1.0),
            ("G", 1.0),
            ("H", 1.0),
            ("J", 1.0),
            ("K", 1.0),
            ("L", 1.0),
            (";", 1.0),
            ("'", 1.0),
            ("enter", 2.25),
        ],
        &[
            ("shift", 2.25),
            ("Z", 1.0),
            ("X", 1.0),
            ("C", 1.0),
            ("V", 1.0),
            ("B", 1.0),
            ("N", 1.0),
            ("M", 1.0),
            (",", 1.0),
            (".", 1.0),
            ("/", 1.0),
            ("shift", 2.75),
        ],
        &[
            ("ctrl", 1.25),
            ("⌘", 1.25),
            ("alt", 1.25),
            ("", 6.25),
            ("alt", 1.25),
            ("fn", 1.25),
            ("ctrl", 1.25),
        ],
    ];
    for (row, keys) in rows.iter().enumerate() {
        let mut x = 0.0;
        for &(label, width) in *keys {
            key(x, row as f32 + 1.3, width, label);
            x += width;
        }
    }
    for (y, left, right) in [(1.3, "ins", "home"), (2.3, "pg↑", "pg↓")] {
        key(15.35, y, 1.0, left);
        key(16.35, y, 1.0, right);
    }
    key(15.35, 4.3, 1.0, "↑");
    key(14.35, 5.3, 1.0, "←");
    key(15.35, 5.3, 1.0, "↓");
    key(16.35, 5.3, 1.0, "→");
    // Small status-light recess; purely decorative like the rest of the drawing.
    let light = base + Vec2::new(16.0 * unit, 3.65 * unit);
    painter.line_segment(
        [
            light - Vec2::new(unit * 0.28, 0.0),
            light + Vec2::new(unit * 0.28, 0.0),
        ],
        Stroke::new(2.0, accent.gamma_multiply(f32::from(brightness) / 255.0)),
    );
    ui.ctx().request_repaint_after(Duration::from_secs_f64(
        1.0 / f64::from(timing.fps.clamp(1, 120)),
    ));
}

fn illumination(mode: &str, x: f32, y: f32, time: f32) -> (f32, f32) {
    let wave = (x * 0.055 + time / 30.0).fract();
    let strength = match mode {
        "comet" => {
            let head = (time * 2.0).rem_euclid(23.0);
            let behind = head - x - y * 0.5;
            if (0.0..5.0).contains(&behind) {
                (1.0 - behind / 5.0).powi(2)
            } else {
                0.0
            }
        }
        "rainbow" => 0.8,
        "breath" => 0.12 + 0.75 * (1.0 - (time.rem_euclid(6.0) / 3.0 - 1.0).abs()),
        "matrix" => {
            let head = (time * 7.5 + (x.floor() * 12.9898).sin() * 7.0).rem_euclid(9.0);
            (1.0 - (head - y).rem_euclid(9.0) / 2.8).max(0.0)
        }
        "focus-cockpit" => {
            if y < 0.5 || x > 15.0 {
                0.65
            } else if y > 5.0 {
                0.4 + 0.12 * time.sin()
            } else {
                0.08
            }
        }
        _ => 0.18,
    };
    (strength.clamp(0.0, 1.0), wave)
}

fn mix(a: Color32, b: Color32, amount: f32) -> Color32 {
    let lerp = |a: u8, b: u8| {
        (f32::from(a) + (f32::from(b) - f32::from(a)) * amount.clamp(0.0, 1.0)) as u8
    };
    Color32::from_rgb(lerp(a.r(), b.r()), lerp(a.g(), b.g()), lerp(a.b(), b.b()))
}

fn effect_color(mode: &str, palette: &str, phase: f32) -> Color32 {
    // Motion remains illustrative, but color choices must respect the runtime.
    match mode {
        "rainbow" => Color32::from(egui::ecolor::Hsva::new(
            phase.rem_euclid(1.0),
            1.0,
            1.0,
            1.0,
        )),
        "focus-cockpit" => Color32::from_rgb(0, 180, 255),
        _ => {
            let palette = if mode == "matrix" {
                "terminal"
            } else {
                palette
            };
            let [r, g, b] = palette_gradient(palette, (phase.rem_euclid(1.0) * 255.0) as u8);
            Color32::from_rgb(r, g, b)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn fixed_color_previews_ignore_palette_but_palette_effects_respond() {
        let render = |mode: &str, palette: &str| {
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
                                &mut KeyboardPreview::default(),
                            );
                        });
                    },
                )
                .shapes
        };
        for mode in ["rainbow", "matrix", "focus-cockpit"] {
            assert_eq!(render(mode, "ocean"), render(mode, "heat"), "{mode}");
        }
        for mode in ["comet", "breath"] {
            assert_ne!(render(mode, "ocean"), render(mode, "heat"), "{mode}");
        }
        assert_eq!(effect_color("rainbow", "ocean", 0.0), Color32::RED);
        assert_eq!(
            effect_color("focus-cockpit", "heat", 0.5),
            Color32::from_rgb(0, 180, 255)
        );
        let [r, g, b] = palette_gradient("terminal", 127);
        assert_eq!(
            effect_color("matrix", "heat", 0.5),
            Color32::from_rgb(r, g, b)
        );
        for palette in ["wooting", "cyberpunk", "ocean", "heat", "terminal"] {
            let [r, g, b] = palette_gradient(palette, 127);
            assert_eq!(
                effect_color("comet", palette, 0.5),
                Color32::from_rgb(r, g, b)
            );
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
    fn every_effect_stays_in_color_bounds() {
        for mode in [
            "comet",
            "rainbow",
            "breath",
            "matrix",
            "focus-cockpit",
            "unknown",
        ] {
            for t in 0..100 {
                for x in 0..18 {
                    let (strength, phase) = illumination(mode, x as f32, 3.0, t as f32 * 0.37);
                    assert!((0.0..=1.0).contains(&strength));
                    assert!((0.0..=1.0).contains(&phase));
                }
            }
        }
    }

    #[test]
    fn color_mixing_preserves_endpoints() {
        let a = Color32::from_rgb(15, 30, 45);
        let b = Color32::from_rgb(230, 180, 120);
        assert_eq!(mix(a, b, 0.0), a);
        assert_eq!(mix(a, b, 1.0), b);
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
