//! Test-only whole-workspace harness: no windows, workers, producers or SDK.
//!
//! Mount as a sibling of `brand` under `#[cfg(test)]`. Render the workspace
//! directly, not `Controller::new` or `eframe::App::update`. Paint summaries are
//! structural evidence, not pixel screenshots or native-window acceptance.
use eframe::egui::{self, Pos2, Rect, Vec2};

pub const DEFAULT_SIZE: Vec2 = egui::vec2(940.0, 850.0);
pub const MINIMUM_SIZE: Vec2 = egui::vec2(620.0, 640.0);
pub const WIDE_SIZE: Vec2 = egui::vec2(1280.0, 800.0);

pub struct Harness {
    pub context: egui::Context,
    size: Vec2,
    frame: u64,
}

impl Harness {
    pub fn new(size: Vec2) -> Self {
        let context = egui::Context::default();
        super::brand::configure_style(&context);
        context.set_pixels_per_point(1.0);
        let mut harness = Self {
            context,
            size: DEFAULT_SIZE,
            frame: 0,
        };
        harness.resize(size);
        harness
    }

    pub fn resize(&mut self, size: Vec2) {
        assert!(size.x.is_finite() && size.x > 0.0);
        assert!(size.y.is_finite() && size.y > 0.0);
        self.size = size;
    }

    pub fn screen(&self) -> Rect {
        Rect::from_min_size(Pos2::ZERO, self.size)
    }

    pub fn run(&mut self, events: Vec<egui::Event>, show: impl FnMut(&egui::Context)) -> Frame {
        let screen = self.screen();
        let output = self.context.run(
            egui::RawInput {
                screen_rect: Some(screen),
                time: Some(self.frame as f64 / 60.0),
                predicted_dt: 1.0 / 60.0,
                events,
                ..Default::default()
            },
            show,
        );
        self.frame += 1;
        Frame { output, screen }
    }

    /// Allow first-frame layout to settle without advancing a shared clock.
    pub fn settle(&mut self, mut show: impl FnMut(&egui::Context)) -> Frame {
        self.run(vec![], &mut show);
        self.run(vec![], show)
    }

    /// Requires one visible label, optionally narrowed to a pane or dialog.
    /// A painted label is not proof of an enabled widget; callers must assert
    /// the resulting model change or captured action (including no-op cases).
    pub fn click(
        &mut self,
        label: &str,
        region: Option<Rect>,
        mut show: impl FnMut(&egui::Context),
    ) -> Frame {
        let frame = self.settle(&mut show);
        let pos = frame.text_hit(label, region);
        self.click_at(pos, show)
    }

    pub fn click_at(&mut self, pos: Pos2, mut show: impl FnMut(&egui::Context)) -> Frame {
        assert!(
            self.screen().contains(pos),
            "click outside viewport: {pos:?}"
        );
        self.run(pointer(pos, true), &mut show);
        self.run(pointer(pos, false), show)
    }

    /// Negative delta.y scrolls content upward to reveal later controls.
    /// Hover the intended pane explicitly so nested scroll areas are exercised.
    pub fn wheel(&mut self, pos: Pos2, delta: Vec2, show: impl FnMut(&egui::Context)) -> Frame {
        assert!(
            self.screen().contains(pos),
            "wheel outside viewport: {pos:?}"
        );
        self.run(
            vec![
                egui::Event::PointerMoved(pos),
                egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta,
                    modifiers: Default::default(),
                },
            ],
            show,
        )
    }
}

pub fn pointer(pos: Pos2, pressed: bool) -> Vec<egui::Event> {
    vec![
        egui::Event::PointerMoved(pos),
        egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: Default::default(),
        },
    ]
}

pub struct Frame {
    pub output: egui::FullOutput,
    pub screen: Rect,
}

#[derive(Debug)]
pub struct PaintedText {
    pub text: String,
    pub rect: Rect,
    /// Effective clip, already intersected with the viewport.
    pub clip: Rect,
}

impl PaintedText {
    pub fn fully_visible(&self) -> bool {
        self.clip.contains(self.rect.min) && self.clip.contains(self.rect.max)
    }
}

/// Geometry in tenths of a logical point; paint order and literal RGBA retained.
/// Deliberately omits texture IDs, widget IDs and font-atlas implementation data.
#[derive(Debug, PartialEq, Eq)]
pub struct PaintSummary {
    pub kind: &'static str,
    pub text: Option<String>,
    pub bounds: [i32; 4],
    pub clip: [i32; 4],
    pub fill: Option<[u8; 4]>,
}

impl Frame {
    /// Includes only text with a nonempty intersection with its effective clip.
    pub fn texts(&self) -> Vec<PaintedText> {
        let mut result = Vec::new();
        self.visit(|shape, clip| {
            if let egui::Shape::Text(text) = shape {
                // Workspace labels are unrotated; never pretend to support
                // hit-testing rotated text with an axis-aligned galley rect.
                assert_eq!(text.angle, 0.0, "rotated label needs explicit targeting");
                let rect = Rect::from_min_size(text.pos, text.galley.size());
                if rect.intersect(clip).is_positive() {
                    result.push(PaintedText {
                        text: text.galley.text().to_owned(),
                        rect,
                        clip,
                    });
                }
            }
        });
        result
    }

    pub fn text_hit(&self, label: &str, region: Option<Rect>) -> Pos2 {
        let matches: Vec<_> = self
            .texts()
            .into_iter()
            .filter(|text| {
                text.text == label
                    && text.clip.contains(text.rect.center())
                    && region.is_none_or(|region| region.contains(text.rect.center()))
            })
            .collect();
        assert_eq!(
            matches.len(),
            1,
            "expected one visible hit for {label:?} in {region:?}, got {matches:?}; \
             scroll the intended pane or narrow duplicate labels to a region"
        );
        matches[0].rect.center()
    }

    pub fn paint_summary(&self) -> Vec<PaintSummary> {
        let mut result = Vec::new();
        self.visit(|shape, clip| {
            let bounds = shape.visual_bounding_rect();
            if !bounds.intersect(clip).is_positive() {
                return;
            }
            let (kind, text, fill) = match shape {
                egui::Shape::Text(text) => ("text", Some(text.galley.text().to_owned()), None),
                egui::Shape::Rect(rect) => ("rect", None, Some(rect.fill.to_array())),
                egui::Shape::Circle(circle) => ("circle", None, Some(circle.fill.to_array())),
                egui::Shape::Ellipse(ellipse) => ("ellipse", None, Some(ellipse.fill.to_array())),
                egui::Shape::LineSegment { .. } => ("line", None, None),
                egui::Shape::Path(_) => ("path", None, None),
                egui::Shape::Mesh(_) => ("mesh", None, None),
                egui::Shape::QuadraticBezier(_) => ("quadratic", None, None),
                egui::Shape::CubicBezier(_) => ("cubic", None, None),
                egui::Shape::Callback(_) => ("callback", None, None),
                egui::Shape::Noop | egui::Shape::Vec(_) => return,
            };
            result.push(PaintSummary {
                kind,
                text,
                bounds: rounded(bounds),
                clip: rounded(clip),
                fill,
            });
        });
        result
    }

    fn visit(&self, mut visit: impl FnMut(&egui::Shape, Rect)) {
        fn recurse(shape: &egui::Shape, clip: Rect, visit: &mut impl FnMut(&egui::Shape, Rect)) {
            match shape {
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        recurse(shape, clip, visit);
                    }
                }
                _ => visit(shape, clip),
            }
        }
        for shape in &self.output.shapes {
            let clip = shape.clip_rect.intersect(self.screen);
            if clip.is_positive() {
                recurse(&shape.shape, clip, &mut visit);
            }
        }
    }
}

fn rounded(rect: Rect) -> [i32; 4] {
    [rect.min.x, rect.min.y, rect.max.x, rect.max.y].map(|v| {
        assert!(v.is_finite(), "nonfinite paint geometry: {rect:?}");
        (v * 10.0).round() as i32
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn real_sizes_brand_style_and_clock_are_local_to_each_harness() {
        for size in [DEFAULT_SIZE, MINIMUM_SIZE, WIDE_SIZE] {
            let mut first = Harness::new(size);
            let mut second = Harness::new(size);
            assert_eq!(
                first.context.style().spacing.item_spacing,
                egui::vec2(8.0, 8.0)
            );
            let mut times = Vec::new();
            let mut record = |harness: &mut Harness| {
                harness.run(vec![], |ctx| {
                    ctx.input(|input| {
                        assert_eq!(input.screen_rect(), Rect::from_min_size(Pos2::ZERO, size));
                        times.push(input.time);
                    });
                });
            };
            record(&mut first);
            record(&mut first);
            record(&mut second);
            assert_eq!(times, [0.0, 1.0 / 60.0, 0.0]);
        }
    }

    #[test]
    fn clicks_use_visible_labels_and_report_structural_paint() {
        let mut harness = Harness::new(MINIMUM_SIZE);
        let mut clicked = false;
        let frame = harness.click("Save fixture", None, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                clicked |= ui.button("Save fixture").clicked();
            });
        });
        assert!(clicked);
        assert!(
            frame
                .texts()
                .iter()
                .any(|t| t.text == "Save fixture" && t.fully_visible())
        );
        assert!(
            frame
                .paint_summary()
                .iter()
                .any(|s| s.text.as_deref() == Some("Save fixture"))
        );
    }

    #[test]
    fn wheel_scrolls_the_hovered_pane_without_enlarging_the_viewport() {
        let mut harness = Harness::new(DEFAULT_SIZE);
        harness.resize(MINIMUM_SIZE);
        let mut offset = 0.0;
        let mut show = |ctx: &egui::Context| {
            egui::CentralPanel::default().show(ctx, |ui| {
                offset = egui::ScrollArea::vertical()
                    .max_height(200.0)
                    .auto_shrink([false, false])
                    .show(ui, |ui| ui.add_space(2000.0))
                    .state
                    .offset
                    .y;
            });
        };
        harness.settle(&mut show);
        let frame = harness.wheel(egui::pos2(100.0, 100.0), egui::vec2(0.0, -240.0), show);
        assert!(offset > 0.0);
        assert_eq!(frame.screen.size(), MINIMUM_SIZE);
    }

    #[test]
    fn clipping_excludes_offscreen_text_and_region_disambiguates_duplicates() {
        let mut harness = Harness::new(DEFAULT_SIZE);
        let frame = harness.settle(|ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let painter = ui.painter();
                for x in [100.0, 500.0] {
                    painter.text(
                        egui::pos2(x, 100.0),
                        egui::Align2::LEFT_TOP,
                        "Duplicate",
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    );
                }
                painter
                    .with_clip_rect(Rect::from_min_size(Pos2::ZERO, egui::vec2(20.0, 20.0)))
                    .text(
                        egui::pos2(100.0, 200.0),
                        egui::Align2::LEFT_TOP,
                        "Clipped away",
                        egui::FontId::proportional(14.0),
                        egui::Color32::WHITE,
                    );
            });
        });
        assert!(!frame.texts().iter().any(|t| t.text == "Clipped away"));
        let left = Rect::from_min_size(Pos2::ZERO, egui::vec2(400.0, 400.0));
        assert!(left.contains(frame.text_hit("Duplicate", Some(left))));
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                frame.text_hit("Duplicate", None);
            }))
            .is_err()
        );
    }
}
