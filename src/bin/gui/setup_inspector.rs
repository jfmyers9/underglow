//! Appearance-only, declarative layer inspector. No engine, SDK or input producers.
//! The caller owns validation, document revision, persistence and explicit Resume.

use clap::ValueEnum;
use eframe::egui::{self, Ui};
use underglow::catalog::{self, PaletteControl, RendererKind, Visualization};
use underglow::render::PaletteName;
use underglow::setup::{BlendMode, Layer, Renderer, SweepBackground, TransitionCurve};

// Scene primitives are not catalog visualizations: these are their only extra
// capabilities (color; Pulse/Sweep also period), never a copied effect catalog.
const PRIMITIVES: [(&str, &str); 3] = [("solid", "Solid"), ("pulse", "Pulse"), ("sweep", "Sweep")];

fn renderer_id(renderer: &Renderer) -> &'static str {
    renderer.catalog_id().unwrap_or(match renderer {
        Renderer::Solid { .. } => "solid",
        Renderer::Pulse { .. } => "pulse",
        Renderer::Sweep { .. } => "sweep",
        _ => "unknown",
    })
}

/// Fresh appearance defaults, never whole-layer defaults. Focus and primitive
/// timing defaults are deserialized through the canonical setup model.
pub fn default_renderer(id: &str) -> Option<Renderer> {
    if let Some(entry) = catalog::find(id) {
        return Some(match entry.renderer {
            RendererKind::Static(effect) => Renderer::Effect {
                effect,
                palette: None,
                speed: entry.defaults.speed,
            },
            RendererKind::Ripples => Renderer::Ripples {
                palette: None,
                base_color: None,
                ripple_color: None,
            },
            RendererKind::Reactive(effect) => Renderer::Reactive {
                effect,
                palette: None,
            },
            RendererKind::Focus => {
                serde_json::from_str(r#"{"kind":"focus"}"#).expect("canonical Focus defaults")
            }
        });
    }
    let json = match id {
        "solid" => r#"{"kind":"solid","color":[255,255,255]}"#,
        "pulse" => r#"{"kind":"pulse","color":[255,255,255]}"#,
        "sweep" => r#"{"kind":"sweep","color":[255,255,255]}"#,
        _ => return None,
    };
    Some(serde_json::from_str(json).expect("canonical primitive defaults"))
}

fn entry(renderer: &Renderer) -> Option<&'static Visualization> {
    renderer.catalog_id().and_then(catalog::find)
}

fn palette(renderer: &Renderer) -> Option<PaletteName> {
    match renderer {
        Renderer::Effect { palette, .. }
        | Renderer::Reactive { palette, .. }
        | Renderer::Ripples { palette, .. } => *palette,
        _ => None,
    }
}

fn primitive_color(renderer: &Renderer) -> Option<[u8; 3]> {
    match renderer {
        Renderer::Solid { color }
        | Renderer::Pulse { color, .. }
        | Renderer::Sweep { color, .. } => Some(*color),
        _ => None,
    }
}

/// Switch only appearance, retaining parameters supported by both renderers.
/// Fixed colors discard palettes; incompatible input/timing settings are not
/// carried over. Unknown IDs and reselecting the current effect are no-ops.
#[cfg(test)]
pub fn switch_renderer(layer: &mut Layer, id: &str) -> bool {
    switch_appearance(&mut layer.renderer, id)
}

pub fn switch_appearance(renderer: &mut Renderer, id: &str) -> bool {
    if renderer_id(renderer) == id {
        return false;
    }
    let Some(mut next) = default_renderer(id) else {
        return false;
    };
    let old = &*renderer;
    if entry(old).is_some_and(|e| e.palette_available(false))
        && entry(&next).is_some_and(|e| e.palette_available(false))
    {
        let value = palette(old);
        match &mut next {
            Renderer::Effect { palette, .. }
            | Renderer::Reactive { palette, .. }
            | Renderer::Ripples { palette, .. } => *palette = value,
            _ => {}
        }
    }
    if entry(old).is_some_and(|e| e.supports_speed)
        && entry(&next).is_some_and(|e| e.supports_speed)
        && let (
            Renderer::Effect {
                speed: previous, ..
            },
            Renderer::Effect { speed, .. },
        ) = (old, &mut next)
    {
        *speed = (*previous).clamp(10, 400);
    }
    if let Some(previous) = primitive_color(old) {
        match &mut next {
            Renderer::Solid { color }
            | Renderer::Pulse { color, .. }
            | Renderer::Sweep { color, .. } => *color = previous,
            _ => {}
        }
    }
    if let (
        Renderer::Pulse {
            period_ms: previous,
            ..
        }
        | Renderer::Sweep {
            period_ms: previous,
            ..
        },
        Renderer::Pulse { period_ms, .. } | Renderer::Sweep { period_ms, .. },
    ) = (old, &mut next)
    {
        *period_ms = (*previous).clamp(1, 60_000);
    }
    *renderer = next;
    true
}

/// Field layout follows the inspector's real width, not the workspace breakpoint.
/// This keeps the normal lower pane aligned and the narrow right pane readable.
pub fn field<R>(ui: &mut Ui, label: &str, control: impl FnOnce(&mut Ui) -> R) -> R {
    let text = egui::RichText::new(label).color(super::brand::MUTED);
    if ui.available_width() >= 440.0 {
        ui.horizontal(|ui| {
            ui.allocate_ui_with_layout(
                egui::vec2(144.0, 28.0),
                egui::Layout::left_to_right(egui::Align::Center),
                |ui| {
                    ui.label(text);
                },
            );
            ui.vertical(control).inner
        })
        .inner
    } else {
        ui.vertical(|ui| {
            ui.label(text);
            control(ui)
        })
        .inner
    }
}

pub fn section_heading(ui: &mut Ui, title: &str, action: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(title).strong().size(14.0));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), action);
    });
}

pub fn section_divider(ui: &mut Ui) {
    ui.add_space(8.0);
    super::brand::divider(ui);
    ui.add_space(8.0);
}

fn palette_control(ui: &mut Ui, palette: &mut Option<PaletteName>, entry: &Visualization) {
    field(ui, "Palette", |ui| {
        egui::ComboBox::from_id_salt("palette")
            .width(ui.available_width().min(300.0))
            .truncate()
            .selected_text(palette.map_or_else(
                || format!("Default ({})", entry.defaults.palette),
                |p| p.to_string(),
            ))
            .show_ui(ui, |ui| {
                ui.selectable_value(
                    palette,
                    None,
                    format!("Default ({})", entry.defaults.palette),
                );
                for &name in PaletteName::value_variants() {
                    ui.selectable_value(palette, Some(name), name.to_string());
                }
            });
    });
}

fn color_control(ui: &mut Ui, label: &str, color: &mut [u8; 3]) {
    field(ui, label, |ui| {
        ui.color_edit_button_srgb(color);
    });
}

/// Returns true iff this frame changed appearance, compositing or transitions.
/// Stable layer ID scopes all widget state; names and document order are untouched.
#[cfg(test)]
pub fn show(ui: &mut Ui, layer: &mut Layer) -> bool {
    let before = layer.clone();
    ui.push_id(("layer-appearance", layer.id.clone()), |ui| {
        renderer_controls(ui, &mut layer.renderer, false);
        ui.add(egui::Slider::new(&mut layer.brightness, 0..=255).text("Layer brightness"));
        ui.small("Only this layer's visual intensity. Global brightness and safety limits remain independent.");
        show_advanced(ui, layer);
    });
    before != *layer
}

/// Layer-wide settings, explicitly separate from editing one appearance.
pub fn show_advanced(ui: &mut Ui, layer: &mut Layer) {
    ui.add(
        egui::Slider::new(&mut layer.opacity, 0..=255)
            .text("Opacity")
            .custom_formatter(|v, _| format!("{:.1}%", v * 100.0 / 255.0))
            .custom_parser(brightness_from_percent),
    );
    egui::ComboBox::from_id_salt("blend-mode")
        .selected_text(format!("{:?}", layer.blend))
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut layer.blend,
                BlendMode::Normal,
                "Normal (alpha replacement)",
            );
            ui.selectable_value(&mut layer.blend, BlendMode::Add, "Add (saturating)");
            ui.selectable_value(&mut layer.blend, BlendMode::Multiply, "Multiply");
        });
    ui.small("Opacity and blend apply to every appearance in this logical layer. Zero opacity is transparent; black is a color, not transparency.");
    ui.add(
        egui::Slider::new(&mut layer.transition.duration_ms, 0..=5000)
            .text("Transition duration (ms)"),
    );
    egui::ComboBox::from_id_salt("transition-curve")
        .selected_text(match layer.transition.curve {
            TransitionCurve::Linear => "Linear",
            TransitionCurve::SmoothStep => "Smooth step",
        })
        .show_ui(ui, |ui| {
            ui.selectable_value(
                &mut layer.transition.curve,
                TransitionCurve::Linear,
                "Linear",
            );
            ui.selectable_value(
                &mut layer.transition.curve,
                TransitionCurve::SmoothStep,
                "Smooth step",
            );
        });
    ui.small("One transition for this logical layer, shared by base/fallback and all variants. 0 ms switches instantly; the curve eases the crossfade.");
    ui.small(super::setup_explanation::TRANSITION_HELP);
    ui.small("Preview only; saving and lighting activation are separate.");
}

/// Display percentages while retaining exact saved 0–255 values until edited.
pub fn brightness_control(ui: &mut Ui, value: &mut u8, label: &str) {
    field(ui, label, |ui| {
        ui.add(
            egui::Slider::new(value, 0..=255)
                .custom_formatter(|value, _| format!("{:.0}%", value * 100.0 / 255.0))
                .custom_parser(brightness_from_percent),
        );
    });
}

fn brightness_from_percent(text: &str) -> Option<f64> {
    let percent = text
        .trim()
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()?;
    (percent.is_finite() && (0.0..=100.0).contains(&percent)).then_some(percent * 255.0 / 100.0)
}

fn renderer_description(ui: &mut Ui, renderer: &Renderer) {
    let metadata = entry(renderer);
    if let Some(entry) = metadata {
        ui.label(entry.description);
        if entry.palette == PaletteControl::Fixed {
            ui.small(entry.fixed_color_note.unwrap_or(
                "This renderer uses fixed colors; palette and color controls are unavailable.",
            ));
        } else if !entry.supports_two_tone {
            ui.small("Uses a palette; arbitrary color controls are unavailable.");
        }
        if !entry.supports_speed {
            ui.small(
                "Animation speed is controlled by this renderer, not an editable speed setting.",
            );
        }
        if !entry.selectable {
            ui.small(
                "Diagnostic effect: preserved from this setup, not offered as an everyday effect.",
            );
        }
    } else if matches!(renderer, Renderer::Sweep { .. }) {
        ui.small("Active color sweeps over a transparent or solid-color background.");
    } else {
        ui.small("Uses a fixed color. Pulse also has a timing control.")
            .on_hover_text("Palettes and effect speed do not apply to these appearances.");
    }
}

pub fn show_renderer_compact(ui: &mut Ui, renderer: &mut Renderer) -> bool {
    renderer_controls(ui, renderer, true)
}

fn renderer_controls(ui: &mut Ui, renderer: &mut Renderer, compact: bool) -> bool {
    let before = renderer.clone();
    ui.push_id("appearance", |ui| {
        let mut selected = renderer_id(renderer);
        let title = entry(renderer).map(|e| e.title)
            .or_else(|| PRIMITIVES.iter().find(|(id, _)| *id == selected).map(|(_, title)| *title))
            .unwrap_or("Unknown renderer");
        let inline_color = ui.available_width() >= 560.0 && primitive_color(renderer).is_some();
        let appearance = field(ui, "Effect", |ui| {
            ui.horizontal(|ui| {
            let picker = egui::ComboBox::from_id_salt("renderer").selected_text(title)
            .width(ui.available_width().min(if inline_color { 220.0 } else { 300.0 })).truncate()
            .height(500.0).show_ui(ui, |ui| {
                for choice in catalog::presets() {
                    ui.selectable_value(&mut selected, choice.id, choice.title);
                }
                ui.separator();
                for (id, title) in PRIMITIVES {
                    ui.selectable_value(&mut selected, id, title);
                }
            });
            if inline_color {
                match renderer {
                    Renderer::Solid { color } | Renderer::Pulse { color, .. } | Renderer::Sweep { color, .. } => {
                        ui.label(egui::RichText::new(if matches!(before, Renderer::Sweep { .. }) { "Active color" } else { "Color" }).color(super::brand::MUTED));
                        ui.color_edit_button_srgb(color);
                    }
                    _ => {}
                }
            }
            picker
            }).inner
        });
        switch_appearance(renderer, selected);

        let metadata = entry(renderer);
        if compact {
            appearance.response.on_hover_ui(|ui| renderer_description(ui, renderer));
        } else {
            renderer_description(ui, renderer);
        }
        match renderer {
            Renderer::Effect { palette, speed, .. } => {
                if let Some(entry) = metadata {
                    if entry.palette_available(false) { palette_control(ui, palette, entry); }
                    if entry.supports_speed {
                        field(ui, "Speed (%)", |ui| { ui.add(egui::Slider::new(speed, 10..=400)); });
                    }
                }
                if !compact { ui.small("Input: elapsed time only; no live input settings."); }
            }
            Renderer::Reactive { palette, .. } => {
                if let Some(entry) = metadata.filter(|e| e.palette_available(false)) { palette_control(ui, palette, entry); }
                if !compact { ui.small("Input: keyboard activity. Preview uses synthetic input; live input requires separate explicit activation. No permission is requested here."); }
            }
            Renderer::Ripples { palette, base_color, ripple_color } => {
                if let Some(entry) = metadata {
                    let mut two_tone = base_color.is_some() && ripple_color.is_some();
                    if entry.supports_two_tone && ui.checkbox(&mut two_tone, "Two-tone colors").changed() {
                        if two_tone {
                            *palette = None;
                            *base_color = Some([0, 0, 0]);
                            *ripple_color = Some([255, 255, 255]);
                        } else {
                            *base_color = None;
                            *ripple_color = None;
                        }
                    }
                    if entry.palette_available(two_tone) { palette_control(ui, palette, entry); }
                    if two_tone {
                        if !compact { ui.small("Two-tone mode replaces the palette with these two colors."); }
                        if let Some(color) = base_color { color_control(ui, "Base color", color); }
                        if let Some(color) = ripple_color { color_control(ui, "Ripple color", color); }
                    }
                }
                if !compact { ui.small("Input: key presses. Preview uses synthetic input; live input requires separate explicit activation. No SDK paths or permission toggles belong to this layer."); }
            }
            Renderer::Solid { color } => { if !inline_color { color_control(ui, "Color", color); } }
            Renderer::Pulse { color, period_ms } => {
                if !inline_color { color_control(ui, "Color", color); }
                field(ui, "Cycle duration (ms)", |ui| {
                    ui.add(egui::Slider::new(period_ms, 1..=60_000))
                        .on_hover_text("Time for one animation cycle. A smaller duration makes the animation faster.");
                });
            }
            Renderer::Sweep { color, period_ms, background } => {
                if !inline_color { color_control(ui, "Active color", color); }
                field(ui, "Background", |ui| {
                    let mut transparent = matches!(background, SweepBackground::Transparent {});
                    let before = transparent;
                    ui.horizontal(|ui| {
                        egui::ComboBox::from_id_salt("sweep-background").width(ui.available_width().min(180.0))
                            .selected_text(if transparent { "Transparent" } else { "Solid color" })
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut transparent, true, "Transparent");
                                ui.selectable_value(&mut transparent, false, "Solid color");
                            }).response.on_hover_text("Transparent reveals lower layers (off if none). Solid black is opaque and switches the background off.");
                        if transparent != before {
                            *background = if transparent { SweepBackground::Transparent {} } else { SweepBackground::default() };
                        }
                        if let SweepBackground::Solid { color } = background { ui.color_edit_button_srgb(color); }
                    });
                });
                field(ui, "Cycle duration (ms)", |ui| {
                    ui.add(egui::Slider::new(period_ms, 1..=60_000))
                        .on_hover_text("Time for one sweep cycle. This is separate from the appearance's Show for duration.");
                });
            }
            Renderer::Focus { focus_minutes, break_minutes, cycles, start_paused, meeting_safe, dim } => {
                field(ui, "Focus minutes", |ui| { ui.add(egui::Slider::new(focus_minutes, 1..=1440)); });
                field(ui, "Break minutes", |ui| { ui.add(egui::Slider::new(break_minutes, 0..=1440)); });
                field(ui, "Cycles", |ui| { ui.add(egui::Slider::new(cycles, 1..=100)); });
                ui.checkbox(start_paused, "Start timer paused");
                ui.checkbox(meeting_safe, "Meeting-safe phase transitions");
                ui.checkbox(dim, "Dim Focus visuals");
                if !compact { ui.small("Input: timer only. Preview simulates its phase, not a live timer. Timer pause is independent of physical lighting Resume."); }
            }
        }
        if compact {
            ui.collapsing("Effect details & input", |ui| {
                renderer_description(ui, renderer);
                if matches!(renderer, Renderer::Ripples { base_color: Some(_), ripple_color: Some(_), .. }) {
                    ui.small("Two-tone mode replaces the palette with these two colors.");
                }
                ui.small(match renderer {
                    Renderer::Effect { .. } => "Input: elapsed time only; no live input settings.",
                    Renderer::Reactive { .. } => "Input: keyboard activity. Preview uses synthetic input; live input requires separate explicit activation. No permission is requested here.",
                    Renderer::Ripples { .. } => "Input: key presses. Preview uses synthetic input; live input requires separate explicit activation. No SDK paths or permission toggles belong to this layer.",
                    Renderer::Focus { .. } => "Input: timer only. Preview simulates its phase, not a live timer. Timer pause is independent of physical lighting Resume.",
                    _ => "Input: elapsed time only; no live input settings.",
                });
            });
        }
        if let Err(error) = renderer.validate() {
            ui.colored_label(super::brand::WARNING, format!("Unsupported settings: {error}. Choose another effect to reset incompatible appearance settings."));
        }

    });
    before != *renderer
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Context, Event, Pos2, Rect, Vec2};
    use underglow::effects::EffectKind;
    use underglow::setup::{Activation, SetupDocument, Target};

    fn layer(id: &str) -> Layer {
        Layer {
            transition: Default::default(),
            opacity: 255,
            blend: Default::default(),
            id: id.into(),
            name: "Same display name".into(),
            variants: Vec::new(),
            enabled: false,
            brightness: 73,
            renderer: default_renderer("aurora").unwrap(),
            target: Target::Selection {
                zones: vec![],
                keys: vec!["Escape".into()],
                groups: vec![],
            },
            activation: Activation::WhileState {
                source: "pi".into(),
                instance: Some("work".into()),
                state: Some("working".into()),
            },
        }
    }

    #[test]
    fn fields_align_at_lower_pane_widths_and_stack_in_the_right_inspector() {
        use super::super::workspace_test_ui::Harness;
        for width in [292.0, 570.0, 688.0] {
            let mut h = Harness::new(egui::vec2(width + 16.0, 300.0));
            let frame = h.settle(|ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    field(ui, "Editing", |ui| {
                        egui::ComboBox::from_id_salt("alignment")
                            .width(ui.available_width().min(300.0))
                            .selected_text("Current appearance")
                            .show_ui(ui, |_| {});
                    });
                });
            });
            let texts = frame.texts();
            let label = texts.iter().find(|t| t.text == "Editing").unwrap();
            let value = texts
                .iter()
                .find(|t| t.text == "Current appearance")
                .unwrap();
            assert!(label.fully_visible() && value.fully_visible());
            if width >= 440.0 {
                assert!((label.rect.center().y - value.rect.center().y).abs() < 2.0);
                assert!(value.rect.left() > label.rect.right());
            } else {
                assert!(value.rect.top() > label.rect.bottom());
                assert!((value.rect.left() - label.rect.left()).abs() < 16.0);
            }
        }
    }

    #[test]
    fn percentage_inputs_use_saved_units_and_reject_invalid_values() {
        assert_eq!(brightness_from_percent("100%"), Some(255.0));
        assert_eq!(brightness_from_percent("0"), Some(0.0));
        assert_eq!(brightness_from_percent(" 50 % "), Some(127.5));
        for invalid in ["NaN", "inf", "-1", "101%", "bad"] {
            assert_eq!(brightness_from_percent(invalid), None);
        }
    }

    #[test]
    fn reading_percentage_controls_does_not_quantize_saved_brightness_or_opacity() {
        use super::super::workspace_test_ui::Harness;
        for width in [292.0, 570.0, 688.0] {
            let mut h = Harness::new(egui::vec2(width + 16.0, 1000.0));
            for original in [0, 1, 73, 128, 254, 255] {
                let mut value = original;
                let mut layer = layer("exact-percentages");
                layer.opacity = original;
                h.settle(|ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        brightness_control(ui, &mut value, "Brightness");
                        show_advanced(ui, &mut layer);
                    });
                });
                assert_eq!(value, original);
                assert_eq!(layer.opacity, original);
            }
        }
    }

    #[test]
    fn every_factory_matches_catalog_and_model_defaults() {
        for entry in catalog::all() {
            let renderer = default_renderer(entry.id).unwrap();
            renderer.validate().unwrap();
            assert_eq!(renderer.catalog_id(), Some(entry.id));
            assert_eq!(palette(&renderer), None);
        }
        for (id, _) in PRIMITIVES {
            default_renderer(id).unwrap().validate().unwrap();
        }
        assert!(default_renderer("not-a-renderer").is_none());
        let canonical: Renderer = serde_json::from_str(r#"{"kind":"focus"}"#).unwrap();
        assert_eq!(default_renderer("focus-cockpit"), Some(canonical));
    }

    #[test]
    fn switching_keeps_only_compatible_appearance_and_preserves_layer_fields() {
        let mut layer = layer("identity");
        layer.renderer = Renderer::Effect {
            effect: EffectKind::Aurora,
            palette: Some(PaletteName::Heat),
            speed: 234,
        };
        let before = layer.clone();
        assert!(switch_renderer(&mut layer, "comet"));
        assert_eq!(
            layer.renderer,
            Renderer::Effect {
                effect: EffectKind::Comet,
                palette: Some(PaletteName::Heat),
                speed: 234
            }
        );
        assert!(switch_renderer(&mut layer, "matrix"));
        assert_eq!(
            layer.renderer,
            Renderer::Effect {
                effect: EffectKind::Matrix,
                palette: None,
                speed: 234
            }
        );
        assert!(switch_renderer(&mut layer, "heatmap"));
        assert_eq!(palette(&layer.renderer), None);
        assert!(switch_renderer(&mut layer, "aurora"));
        assert_eq!(layer.renderer, default_renderer("aurora").unwrap());
        let appearance = layer.renderer.clone();
        layer.renderer = before.renderer.clone();
        assert_eq!(layer, before);
        layer.renderer = appearance;
        assert!(!switch_renderer(&mut layer, "aurora"));
        assert!(!switch_renderer(&mut layer, "unknown"));
        layer.renderer = Renderer::Pulse {
            color: [1, 2, 3],
            period_ms: 1234,
        };
        assert!(switch_renderer(&mut layer, "sweep"));
        assert_eq!(
            layer.renderer,
            Renderer::Sweep {
                background: Default::default(),
                color: [1, 2, 3],
                period_ms: 1234
            }
        );
        assert!(switch_renderer(&mut layer, "solid"));
        assert_eq!(layer.renderer, Renderer::Solid { color: [1, 2, 3] });
        assert!(switch_renderer(&mut layer, "ripples"));
        assert_eq!(layer.renderer, default_renderer("ripples").unwrap());
    }

    fn run(
        ctx: &Context,
        layer: &mut Layer,
        events: Vec<Event>,
    ) -> (bool, Vec<egui::epaint::ClippedShape>) {
        let mut changed = false;
        let output = ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(900.0, 1000.0))),
                events,
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    changed = show(ui, layer);
                });
            },
        );
        (changed, output.shapes)
    }

    fn text_pos(shapes: &[egui::epaint::ClippedShape], label: &str) -> Option<Pos2> {
        shapes.iter().find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
    }

    fn click(
        ctx: &Context,
        layer: &mut Layer,
        pos: Pos2,
    ) -> (bool, Vec<egui::epaint::ClippedShape>) {
        let mut changed = false;
        for pressed in [true, false] {
            changed |= run(
                ctx,
                layer,
                vec![
                    Event::PointerMoved(pos),
                    Event::PointerButton {
                        pos,
                        button: egui::PointerButton::Primary,
                        pressed,
                        modifiers: Default::default(),
                    },
                ],
            )
            .0;
        }
        let (idle, shapes) = run(ctx, layer, vec![]);
        (changed || idle, shapes)
    }

    fn select(ctx: &Context, layer: &mut Layer, current: &str, next: &str) -> bool {
        run(ctx, layer, vec![]);
        let (_, shapes) = run(ctx, layer, vec![]);
        let pos = text_pos(&shapes, current).unwrap_or_else(|| panic!("missing {current}"));
        let (_, shapes) = click(ctx, layer, pos);
        let pos = text_pos(&shapes, next).unwrap_or_else(|| panic!("missing choice {next}"));
        click(ctx, layer, pos).0
    }

    #[test]
    fn headless_transition_changes_only_selected_logical_settings() {
        let mut document =
            SetupDocument::from_toml(include_str!("../../../examples/layer-setup.toml")).unwrap();
        let before = document.clone();
        let ctx = Context::default();
        let layer = &mut document.layers[0];
        run(&ctx, layer, vec![]);
        let (changed, shapes) = run(&ctx, layer, vec![]);
        assert!(!changed);
        let y = text_pos(&shapes, "Transition duration (ms)").unwrap().y;
        assert!(click(&ctx, layer, Pos2::new(60.0, y)).0);
        assert!(layer.transition.duration_ms > 0 && layer.transition.duration_ms <= 5000);
        assert!(select(&ctx, layer, "Linear", "Smooth step"));
        assert_eq!(layer.transition.curve, TransitionCurve::SmoothStep);
        assert!(
            !run(&ctx, layer, vec![]).0,
            "idle transition controls must not edit the draft"
        );
        let edited = layer.clone();
        layer.transition = before.layers[0].transition;
        assert_eq!(
            document, before,
            "activation, identities, variants, appearance, targets and other layers remain unchanged"
        );
        document.layers[0] = edited;
        assert_eq!(
            SetupDocument::from_toml(&document.to_toml().unwrap()).unwrap(),
            document
        );
    }

    #[test]
    fn headless_opacity_and_blend_change_only_selected_logical_settings() {
        let mut document =
            SetupDocument::from_toml(include_str!("../../../examples/layer-setup.toml")).unwrap();
        let before = document.clone();
        let ctx = Context::default();
        let layer = &mut document.layers[0];
        run(&ctx, layer, vec![]);
        let (_, shapes) = run(&ctx, layer, vec![]);
        let y = text_pos(&shapes, "Opacity").unwrap().y;
        assert!(text_pos(&shapes, "100.0%").is_some());
        assert!(click(&ctx, layer, Pos2::new(60.0, y)).0);
        assert!(layer.opacity > 0 && layer.opacity < 255);
        assert!(select(&ctx, layer, "Normal", "Add (saturating)"));
        assert_eq!(layer.blend, BlendMode::Add);
        let edited = layer.clone();
        layer.opacity = before.layers[0].opacity;
        layer.blend = before.layers[0].blend;
        assert_eq!(
            document, before,
            "all identities, renderer settings, groups, targets, variants and other layers are unchanged"
        );
        document.layers[0] = edited;
        assert_eq!(
            SetupDocument::from_toml(&document.to_toml().unwrap()).unwrap(),
            document
        );
    }

    #[test]
    fn headless_switch_palette_brightness_and_fixed_colors_preserve_other_layers() {
        let mut document =
            SetupDocument::from_toml(include_str!("../../../examples/layer-setup.toml")).unwrap();
        document.layers[0] = layer("selected");
        let untouched = document.clone();
        let ctx = Context::default();
        let layer = &mut document.layers[0];
        assert!(!run(&ctx, layer, vec![]).0);
        assert!(select(&ctx, layer, "Default (ocean)", "heat"));
        assert_eq!(palette(&layer.renderer), Some(PaletteName::Heat));
        assert!(select(&ctx, layer, "Aurora", "Matrix"));
        assert!(matches!(
            layer.renderer,
            Renderer::Effect {
                effect: EffectKind::Matrix,
                palette: None,
                ..
            }
        ));
        let (_, shapes) = run(&ctx, layer, vec![]);
        assert!(text_pos(&shapes, "Palette").is_none());
        assert!(text_pos(&shapes, "Color").is_none());
        assert!(
            text_pos(
                &shapes,
                catalog::find("matrix").unwrap().fixed_color_note.unwrap()
            )
            .is_some()
        );
        let y = text_pos(&shapes, "Layer brightness").unwrap().y;
        assert!(click(&ctx, layer, Pos2::new(80.0, y)).0);
        assert_ne!(layer.brightness, untouched.layers[0].brightness);
        assert!(select(&ctx, layer, "Matrix", "Aurora"));
        assert_eq!(palette(&layer.renderer), None); // old Heat must not leak back
        assert_eq!(layer.id, untouched.layers[0].id);
        assert_eq!(layer.target, untouched.layers[0].target);
        assert_eq!(layer.activation, untouched.layers[0].activation);
        assert_eq!(layer.enabled, untouched.layers[0].enabled);
        assert_eq!(layer.name, untouched.layers[0].name);
        assert_eq!(document.layers[1..], untouched.layers[1..]);
        assert_eq!(document.revision, untouched.revision);
        document.validate().unwrap();
    }

    #[test]
    fn headless_two_tone_and_focus_expose_only_supported_controls() {
        let ctx = Context::default();
        let mut layer = layer("input");
        switch_renderer(&mut layer, "ripples");
        run(&ctx, &mut layer, vec![]);
        let (_, shapes) = run(&ctx, &mut layer, vec![]);
        let pos = text_pos(&shapes, "Two-tone colors").unwrap();
        let (changed, shapes) = click(&ctx, &mut layer, pos);
        assert!(changed);
        assert!(text_pos(&shapes, "Base color").is_some());
        assert!(text_pos(&shapes, "Ripple color").is_some());
        assert!(text_pos(&shapes, "Palette").is_none());
        layer.renderer.validate().unwrap();
        let pos = text_pos(&shapes, "Two-tone colors").unwrap();
        assert!(click(&ctx, &mut layer, pos).0);
        assert_eq!(layer.renderer, default_renderer("ripples").unwrap());
        assert!(select(&ctx, &mut layer, "Ripples", "Focus"));
        let (_, shapes) = run(&ctx, &mut layer, vec![]);
        assert!(text_pos(&shapes, "Focus minutes").is_some());
        assert!(text_pos(&shapes, "Palette").is_none());
        assert!(text_pos(&shapes, "Speed (%)").is_none());
        assert!(text_pos(&shapes, "Color").is_none());
        let pos = text_pos(&shapes, "Start timer paused").unwrap();
        assert!(click(&ctx, &mut layer, pos).0);
        assert!(matches!(
            layer.renderer,
            Renderer::Focus {
                start_paused: true,
                ..
            }
        ));
        assert!(!run(&ctx, &mut layer, vec![]).0);
    }

    #[test]
    fn headless_idle_controls_follow_every_catalog_capability() {
        for metadata in catalog::all() {
            let ctx = Context::default();
            let mut layer = layer(metadata.id);
            layer.renderer = default_renderer(metadata.id).unwrap();
            let before = layer.clone();
            run(&ctx, &mut layer, vec![]);
            let (changed, shapes) = run(&ctx, &mut layer, vec![]);
            assert!(!changed, "{} changed on idle", metadata.id);
            assert_eq!(layer, before);
            assert_eq!(
                text_pos(&shapes, "Palette").is_some(),
                metadata.palette_available(false),
                "{} palette",
                metadata.id
            );
            assert_eq!(
                text_pos(&shapes, "Speed (%)").is_some(),
                metadata.supports_speed,
                "{} speed",
                metadata.id
            );
            assert_eq!(
                text_pos(&shapes, "Two-tone colors").is_some(),
                metadata.supports_two_tone,
                "{} two-tone",
                metadata.id
            );
            assert!(text_pos(&shapes, "Color").is_none());
        }
        // Imported unsupported palettes must be diagnosed, not silently ignored
        // or rewritten merely by opening the inspector.
        let ctx = Context::default();
        let mut layer = layer("unsupported");
        layer.renderer = Renderer::Effect {
            effect: EffectKind::Matrix,
            palette: Some(PaletteName::Heat),
            speed: 100,
        };
        let before = layer.clone();
        run(&ctx, &mut layer, vec![]);
        let (changed, shapes) = run(&ctx, &mut layer, vec![]);
        assert!(!changed);
        assert_eq!(layer, before);
        assert!(text_pos(&shapes, "Palette").is_none());
        assert!(shapes.iter().any(|shape| matches!(&shape.shape, egui::Shape::Text(text) if text.galley.text().starts_with("Unsupported settings:"))));
    }
}
