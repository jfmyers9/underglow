//! Decorative, synthetic effect preview. Never reads device frames or touches hardware.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Sense, Stroke, StrokeKind, Vec2};
use std::time::Duration;

/// Paint an animated, illustrative 80%-style keyboard at the available width.
/// `brightness` is 0–255. The caller must label this as a simulated preview.
pub fn keyboard(ui: &mut egui::Ui, mode: &str, palette: &str, brightness: u8) {
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
    let time = ui.input(|i| i.time) as f32;
    let accent = palette_color(palette, 0.25);

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
        let strength = strength * f32::from(brightness) / 255.0;
        let color = palette_color(palette, hue);
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
    ui.ctx().request_repaint_after(Duration::from_millis(33));
}

fn illumination(mode: &str, x: f32, y: f32, time: f32) -> (f32, f32) {
    let wave = (x * 0.055 + time * 0.1).fract();
    let strength = match mode {
        "ripples" => {
            let radius = (time * 2.6).rem_euclid(13.0);
            let distance = ((x - 6.0).powi(2) + (y - 3.0).powi(2)).sqrt();
            (1.0 - (distance - radius).abs() / 1.6).max(0.0)
        }
        "comet" => {
            let head = (time * 4.0).rem_euclid(23.0);
            let behind = head - x - y * 0.5;
            if (0.0..5.0).contains(&behind) {
                (1.0 - behind / 5.0).powi(2)
            } else {
                0.0
            }
        }
        "rainbow" => 0.58 + 0.22 * (x * 0.3 - time).sin(),
        "breath" => 0.12 + 0.75 * ((time * 1.5).sin() * 0.5 + 0.5).powi(2),
        "matrix" => {
            let head = (time * 2.5 + (x.floor() * 12.9898).sin() * 7.0).rem_euclid(9.0);
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

fn palette_color(palette: &str, phase: f32) -> Color32 {
    let (a, b) = match palette {
        "cyberpunk" => ([194, 123, 244], [85, 215, 232]),
        "heat" => ([248, 119, 84], [246, 206, 118]),
        "terminal" => ([94, 211, 141], [187, 235, 151]),
        "ocean" => ([70, 163, 229], [102, 226, 215]),
        _ => ([109, 230, 189], [101, 180, 235]),
    };
    mix(
        Color32::from_rgb(a[0], a[1], a[2]),
        Color32::from_rgb(b[0], b[1], b[2]),
        (phase * std::f32::consts::TAU).sin() * 0.5 + 0.5,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

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
                        keyboard(ui, "ripples", "ocean", 180);
                    });
                },
            );
            assert!(output.shapes.len() > 200);
        }
    }

    #[test]
    fn every_effect_stays_in_color_bounds() {
        for mode in [
            "ripples",
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
}
