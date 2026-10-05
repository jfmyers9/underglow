//! Shared application chrome. Effect colors remain owned by the renderer.
use eframe::egui::{self, Color32, Pos2, Rect, Stroke, StrokeKind};

pub const BG: Color32 = Color32::from_rgb(10, 11, 17);
pub const SURFACE: Color32 = Color32::from_rgb(19, 21, 31);
pub const RAISED: Color32 = Color32::from_rgb(29, 32, 46);
pub const BORDER: Color32 = Color32::from_rgb(53, 58, 77);
pub const GLINT: Color32 = Color32::from_rgb(83, 90, 113);
pub const INK: Color32 = Color32::from_rgb(239, 242, 250);
pub const MUTED: Color32 = Color32::from_rgb(165, 173, 194);
pub const ACCENT: Color32 = Color32::from_rgb(104, 218, 255);
pub const SUCCESS: Color32 = Color32::from_rgb(110, 230, 170);
pub const WARNING: Color32 = Color32::from_rgb(255, 201, 116);
pub const ERROR: Color32 = Color32::from_rgb(255, 143, 161);
const SPECTRUM: [Color32; 6] = [
    Color32::from_rgb(52, 132, 255),
    Color32::from_rgb(149, 87, 255),
    Color32::from_rgb(245, 76, 211),
    Color32::from_rgb(255, 115, 131),
    Color32::from_rgb(255, 209, 99),
    Color32::from_rgb(85, 230, 161),
];

pub fn icon_data() -> egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon/underglow-256.png"))
        .expect("checked-in application icon must be a valid PNG")
}

fn icon_texture(context: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new("underglow-brand-icon");
    if let Some(texture) = context.data(|data| data.get_temp::<egui::TextureHandle>(id)) {
        return texture;
    }
    let icon = icon_data();
    let texture = context.load_texture(
        "Underglow keycap",
        egui::ColorImage::from_rgba_unmultiplied(
            [icon.width as usize, icon.height as usize],
            &icon.rgba,
        ),
        egui::TextureOptions::LINEAR,
    );
    context.data_mut(|data| data.insert_temp(id, texture.clone()));
    texture
}

pub fn wordmark(ui: &mut egui::Ui, compact: bool) {
    let size = if compact { 36.0 } else { 54.0 };
    ui.horizontal(|ui| {
        let texture = icon_texture(ui.ctx());
        ui.add(egui::Image::new((texture.id(), egui::vec2(size, size))))
            .on_hover_text("Underglow · keyboard lighting and notifications");
        ui.vertical(|ui| {
            ui.spacing_mut().item_spacing.y = 3.0;
            ui.label(
                egui::RichText::new("UNDERGLOW")
                    .size(if compact { 14.0 } else { 18.0 })
                    .strong()
                    .color(INK),
            );
            ui.label(
                egui::RichText::new("Lighting + signals")
                    .size(11.0)
                    .color(MUTED),
            );
        });
    });
}

/// A static, narrow light seam, not an activity indicator or a lighting palette.
pub fn spectrum(painter: &egui::Painter, rect: Rect, opacity: f32) {
    let mut mesh = egui::Mesh::default();
    let segments = (SPECTRUM.len() - 1) as f32;
    for (i, colors) in SPECTRUM.windows(2).enumerate() {
        let left = rect.left() + rect.width() * i as f32 / segments;
        let right = rect.left() + rect.width() * (i + 1) as f32 / segments;
        let index = mesh.vertices.len() as u32;
        let a = colors[0].gamma_multiply(opacity);
        let b = colors[1].gamma_multiply(opacity);
        mesh.colored_vertex(Pos2::new(left, rect.top()), a);
        mesh.colored_vertex(Pos2::new(right, rect.top()), b);
        mesh.colored_vertex(Pos2::new(left, rect.bottom()), a);
        mesh.colored_vertex(Pos2::new(right, rect.bottom()), b);
        mesh.add_triangle(index, index + 1, index + 2);
        mesh.add_triangle(index + 2, index + 1, index + 3);
    }
    painter.add(egui::Shape::mesh(mesh));
}

pub fn divider(ui: &mut egui::Ui) {
    let (rect, _) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 7.0), egui::Sense::hover());
    spectrum(ui.painter(), rect.shrink2(egui::vec2(0.0, 1.0)), 0.06);
    spectrum(ui.painter(), rect.shrink2(egui::vec2(0.0, 3.0)), 0.65);
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(SURFACE)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(16)
        .inner_margin(20)
        .shadow(egui::epaint::Shadow {
            offset: [0, 4],
            blur: 16,
            spread: 0,
            color: Color32::from_black_alpha(45),
        })
}

pub fn keyboard_shell(painter: &egui::Painter, rect: Rect, inset: f32) {
    painter.rect_filled(
        rect.translate(egui::vec2(0.0, 4.0)),
        14,
        Color32::from_black_alpha(85),
    );
    painter.rect(
        rect,
        14,
        RAISED,
        Stroke::new(1.0, GLINT),
        StrokeKind::Inside,
    );
    painter.rect_filled(rect.shrink(inset), 9, BG);
}

pub fn state_color(connected: bool, simulation: bool, state: Option<&str>) -> Color32 {
    if simulation {
        return ACCENT;
    }
    if !connected {
        return MUTED;
    }
    match state {
        Some("active") => SUCCESS,
        Some("retrying") => WARNING,
        Some("error") => ERROR,
        _ => MUTED,
    }
}

pub fn configure_style(context: &egui::Context) {
    let mut style = (*context.style()).clone();
    style.visuals = egui::Visuals::dark();
    style.visuals.panel_fill = BG;
    style.visuals.window_fill = SURFACE;
    style.visuals.extreme_bg_color = BG;
    style.visuals.faint_bg_color = RAISED;
    style.visuals.window_stroke = Stroke::new(1.0, BORDER);
    style.visuals.window_corner_radius = 16.into();
    style.visuals.menu_corner_radius = 10.into();
    style.visuals.override_text_color = Some(INK);
    style.visuals.warn_fg_color = WARNING;
    style.visuals.error_fg_color = ERROR;
    style.visuals.hyperlink_color = ACCENT;
    style.visuals.selection.bg_fill = Color32::from_rgb(29, 64, 83);
    style.visuals.selection.stroke = Stroke::new(1.0, ACCENT);
    let widgets = &mut style.visuals.widgets;
    for visual in [
        &mut widgets.noninteractive,
        &mut widgets.inactive,
        &mut widgets.hovered,
        &mut widgets.active,
        &mut widgets.open,
    ] {
        visual.corner_radius = 8.into();
        visual.bg_stroke = Stroke::new(1.0, BORDER);
        visual.fg_stroke.color = INK;
        visual.expansion = 0.0;
    }
    widgets.noninteractive.bg_fill = SURFACE;
    widgets.noninteractive.weak_bg_fill = SURFACE;
    widgets.inactive.bg_fill = RAISED;
    widgets.inactive.weak_bg_fill = RAISED;
    widgets.hovered.bg_fill = Color32::from_rgb(39, 45, 64);
    widgets.hovered.weak_bg_fill = widgets.hovered.bg_fill;
    widgets.hovered.bg_stroke.color = ACCENT;
    widgets.active.bg_fill = Color32::from_rgb(35, 57, 76);
    widgets.active.weak_bg_fill = widgets.active.bg_fill;
    widgets.active.bg_stroke.color = ACCENT;
    widgets.open.bg_fill = RAISED;
    widgets.open.weak_bg_fill = RAISED;
    style.visuals.slider_trailing_fill = true;
    style.spacing.item_spacing = egui::vec2(12.0, 12.0);
    style.spacing.button_padding = egui::vec2(16.0, 10.0);
    for (text, size) in [
        (egui::TextStyle::Body, 15.0),
        (egui::TextStyle::Button, 14.0),
        (egui::TextStyle::Small, 12.0),
    ] {
        style
            .text_styles
            .insert(text, egui::FontId::proportional(size));
    }
    context.set_style(style);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn main_page_and_settings_share_one_cached_icon_texture() {
        let context = egui::Context::default();
        let first = icon_texture(&context);
        let second = icon_texture(&context);
        assert_eq!(first.id(), second.id());
        assert_eq!(first.size(), [256, 256]);
    }

    #[test]
    fn semantic_state_colors_are_not_brand_spectrum_or_stale_activity() {
        assert_eq!(state_color(true, false, Some("active")), SUCCESS);
        assert_eq!(state_color(true, false, Some("retrying")), WARNING);
        assert_eq!(state_color(true, false, Some("error")), ERROR);
        assert_eq!(state_color(false, false, Some("active")), MUTED);
        assert_eq!(state_color(true, false, Some("paused")), MUTED);
        assert_eq!(state_color(true, true, Some("active")), ACCENT);
    }

    #[test]
    fn text_and_status_colors_remain_readable_on_graphite() {
        fn luminance(color: Color32) -> f32 {
            let linear = |v: u8| {
                let v = f32::from(v) / 255.0;
                if v <= 0.04045 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            linear(color.r()) * 0.2126 + linear(color.g()) * 0.7152 + linear(color.b()) * 0.0722
        }
        for background in [BG, SURFACE, RAISED] {
            for foreground in [INK, MUTED, ACCENT, SUCCESS, WARNING, ERROR] {
                let contrast = (luminance(foreground) + 0.05) / (luminance(background) + 0.05);
                assert!(
                    contrast >= 4.5,
                    "{foreground:?} on {background:?}: {contrast}"
                );
            }
        }
    }
}
