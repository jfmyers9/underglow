//! Opt-in software captures of the real headless egui workspace.
//!
//! These preserve actual geometry, fonts and clipping, but are not native/GPU
//! screenshots or a substitute for the user walkthrough. No window or SDK runs.
use super::egui::{self, Color32, Pos2, Rect, TextureId};
use std::collections::HashMap;

struct Texture {
    size: [usize; 2],
    pixels: Vec<[u8; 4]>,
}

struct Capture {
    size: [usize; 2],
    pixels: Vec<[u8; 4]>,
    textures: HashMap<TextureId, Texture>,
}

impl Capture {
    fn new(size: [usize; 2]) -> Self {
        Self {
            size,
            pixels: vec![[0, 0, 0, 255]; size[0] * size[1]],
            textures: HashMap::new(),
        }
    }

    fn update_textures(&mut self, delta: &egui::TexturesDelta) {
        for (id, change) in &delta.set {
            let size = change.image.size();
            let pixels: Vec<_> = match &change.image {
                egui::ImageData::Color(image) => image.pixels.clone(),
                egui::ImageData::Font(image) => image.srgba_pixels(None).collect(),
            }
            .into_iter()
            .map(|color| color.to_array())
            .collect();
            if let Some([x, y]) = change.pos {
                let texture = self.textures.get_mut(id).expect("patch without allocation");
                assert!(x + size[0] <= texture.size[0] && y + size[1] <= texture.size[1]);
                for row in 0..size[1] {
                    let start = (y + row) * texture.size[0] + x;
                    texture.pixels[start..start + size[0]]
                        .copy_from_slice(&pixels[row * size[0]..(row + 1) * size[0]]);
                }
            } else {
                self.textures.insert(*id, Texture { size, pixels });
            }
        }
    }

    fn paint(&mut self, context: &egui::Context, output: egui::FullOutput) {
        assert_eq!(
            output.pixels_per_point, 1.0,
            "captures use logical 1x pixels"
        );
        let screen = Rect::from_min_size(
            Pos2::ZERO,
            egui::vec2(self.size[0] as f32, self.size[1] as f32),
        );
        for primitive in context.tessellate(output.shapes, output.pixels_per_point) {
            let egui::epaint::Primitive::Mesh(mesh) = primitive.primitive else {
                panic!("native paint callback cannot be represented by this capture");
            };
            let texture = self
                .textures
                .get(&mesh.texture_id)
                .expect("missing texture");
            let clip = primitive.clip_rect.intersect(screen);
            for indices in mesh.indices.chunks_exact(3) {
                let vertices = std::array::from_fn(|index| mesh.vertices[indices[index] as usize]);
                triangle(&mut self.pixels, self.size, texture, clip, vertices);
            }
        }
        for id in output.textures_delta.free {
            self.textures.remove(&id);
        }
    }

    /// Minimal lossless RGBA PNG: stored deflate blocks avoid another dependency.
    fn png(&self) -> Vec<u8> {
        let mut raw = Vec::with_capacity((self.size[0] * 4 + 1) * self.size[1]);
        for row in self.pixels.chunks_exact(self.size[0]) {
            raw.push(0); // PNG's None row filter.
            for pixel in row {
                raw.extend_from_slice(pixel);
            }
        }
        let mut deflate = vec![0x78, 0x01];
        let count = raw.len().div_ceil(65_535);
        for (index, block) in raw.chunks(65_535).enumerate() {
            deflate.push(u8::from(index + 1 == count));
            let len = block.len() as u16;
            deflate.extend_from_slice(&len.to_le_bytes());
            deflate.extend_from_slice(&(!len).to_le_bytes());
            deflate.extend_from_slice(block);
        }
        let (mut a, mut b) = (1_u32, 0_u32);
        for byte in &raw {
            a = (a + u32::from(*byte)) % 65_521;
            b = (b + a) % 65_521;
        }
        deflate.extend_from_slice(&((b << 16) | a).to_be_bytes());
        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut header = Vec::new();
        header.extend_from_slice(&(self.size[0] as u32).to_be_bytes());
        header.extend_from_slice(&(self.size[1] as u32).to_be_bytes());
        header.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(&mut png, b"IHDR", &header);
        chunk(&mut png, b"IDAT", &deflate);
        chunk(&mut png, b"IEND", &[]);
        png
    }
}
#[test]
#[ignore = "opt-in preview control captures; set UNDERGLOW_PREVIEW_CONTROLS_SNAPSHOTS"]
fn export_preview_controls_snapshots() {
    use super::workspace_test_ui::Harness;
    let destination = std::path::PathBuf::from(
        std::env::var_os("UNDERGLOW_PREVIEW_CONTROLS_SNAPSHOTS")
            .expect("set UNDERGLOW_PREVIEW_CONTROLS_SNAPSHOTS to an explicit directory"),
    );
    std::fs::create_dir_all(&destination).unwrap();
    for size in [[940, 850], [620, 640], [1280, 800]] {
        for state in ["real", "choices", "simulating", "more", "advanced"] {
            let (mut app, incoming) = state_fixture("normal");
            app.setup_editor
                .preview
                .set_mode(super::setup_preview::Mode::Live);
            let draft = app.setup_editor.draft.clone();
            let mut harness = Harness::new(egui::vec2(size[0] as f32, size[1] as f32));
            let mut capture = Capture::new(size);
            if matches!(state, "choices" | "simulating") {
                capture_click_label(&mut capture, &mut harness, &mut app, "Simulate activity…");
                if state == "simulating" {
                    capture_click_label(&mut capture, &mut harness, &mut app, "Working");
                    capture_click_label(&mut capture, &mut harness, &mut app, "Pause simulation");
                }
            }
            if matches!(state, "more" | "advanced") {
                capture_click_label(&mut capture, &mut harness, &mut app, "More…");
                if state == "advanced" {
                    capture_click_label(&mut capture, &mut harness, &mut app, "Advanced preview…");
                }
            }
            for index in 0..30 {
                let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
                capture.update_textures(&frame.output.textures_delta);
                if index == 29 {
                    capture.paint(&harness.context, frame.output);
                }
            }
            assert_eq!(app.setup_editor.draft, draft);
            assert!(!app.setup_editor.dirty);
            assert!(app.setup_editor.preview.sample().is_none());
            assert!(incoming.try_recv().is_err());
            let path = destination.join(format!("preview-{state}-{}x{}.png", size[0], size[1]));
            std::fs::write(path, capture.png()).unwrap();
        }
    }
}

fn chunk(png: &mut Vec<u8>, kind: &[u8; 4], data: &[u8]) {
    png.extend_from_slice(&(data.len() as u32).to_be_bytes());
    png.extend_from_slice(kind);
    png.extend_from_slice(data);
    let mut crc = !0_u32;
    for byte in kind.iter().chain(data) {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = (crc >> 1) ^ (0xedb8_8320 & 0_u32.wrapping_sub(crc & 1));
        }
    }
    png.extend_from_slice(&(!crc).to_be_bytes());
}

fn edge(a: Pos2, b: Pos2, point: Pos2) -> f32 {
    (b.x - a.x) * (point.y - a.y) - (b.y - a.y) * (point.x - a.x)
}

fn sample(texture: &Texture, uv: Pos2) -> [f32; 4] {
    let x = (uv.x * texture.size[0] as f32 - 0.5).clamp(0.0, (texture.size[0] - 1) as f32);
    let y = (uv.y * texture.size[1] as f32 - 0.5).clamp(0.0, (texture.size[1] - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = (
        (x0 + 1).min(texture.size[0] - 1),
        (y0 + 1).min(texture.size[1] - 1),
    );
    let (tx, ty) = (x.fract(), y.fract());
    std::array::from_fn(|channel| {
        [
            (x0, y0, (1.0 - tx) * (1.0 - ty)),
            (x1, y0, tx * (1.0 - ty)),
            (x0, y1, (1.0 - tx) * ty),
            (x1, y1, tx * ty),
        ]
        .into_iter()
        .map(|(x, y, weight)| {
            f32::from(texture.pixels[y * texture.size[0] + x][channel]) * weight / 255.0
        })
        .sum()
    })
}

fn triangle(
    pixels: &mut [[u8; 4]],
    size: [usize; 2],
    texture: &Texture,
    clip: Rect,
    vertices: [egui::epaint::Vertex; 3],
) {
    let [a, b, c] = vertices;
    let area = edge(a.pos, b.pos, c.pos);
    if area.abs() < f32::EPSILON || !clip.is_positive() {
        return;
    }
    let bounds = Rect::from_points(&[a.pos, b.pos, c.pos]).intersect(clip);
    if !bounds.is_positive() {
        return;
    }
    let colors = vertices.map(|vertex| vertex.color.to_array());
    for y in
        (bounds.top().floor().max(0.0) as usize)..(bounds.bottom().ceil() as usize).min(size[1])
    {
        for x in
            (bounds.left().floor().max(0.0) as usize)..(bounds.right().ceil() as usize).min(size[0])
        {
            let point = egui::pos2(x as f32 + 0.5, y as f32 + 0.5);
            if !clip.contains(point) {
                continue;
            }
            let weights = [
                edge(b.pos, c.pos, point) / area,
                edge(c.pos, a.pos, point) / area,
                edge(a.pos, b.pos, point) / area,
            ];
            if weights.iter().any(|weight| *weight < -0.000_001) {
                continue;
            }
            let uv = egui::pos2(
                a.uv.x * weights[0] + b.uv.x * weights[1] + c.uv.x * weights[2],
                a.uv.y * weights[0] + b.uv.y * weights[1] + c.uv.y * weights[2],
            );
            let texture = sample(texture, uv);
            let color: [f32; 4] = std::array::from_fn(|channel| {
                (0..3)
                    .map(|i| f32::from(colors[i][channel]) * weights[i] / 255.0)
                    .sum::<f32>()
                    * texture[channel]
            });
            let pixel = &mut pixels[y * size[0] + x];
            for channel in 0..3 {
                pixel[channel] = (color[channel] * 255.0
                    + f32::from(pixel[channel]) * (1.0 - color[3]))
                    .round()
                    .clamp(0.0, 255.0) as u8;
            }
        }
    }
}

#[test]
fn software_capture_handles_real_egui_meshes_fonts_and_png_encoding() {
    let context = egui::Context::default();
    let mut capture = Capture::new([64, 64]);
    let output = context.run(
        egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(64.0, 64.0))),
            ..Default::default()
        },
        |context| {
            egui::CentralPanel::default().show(context, |ui| {
                ui.painter().rect_filled(
                    Rect::from_min_max(egui::pos2(8.0, 8.0), egui::pos2(32.0, 32.0)),
                    0.0,
                    Color32::from_rgb(120, 30, 60),
                );
                ui.painter().text(
                    egui::pos2(4.0, 40.0),
                    egui::Align2::LEFT_TOP,
                    "Test",
                    egui::FontId::proportional(12.0),
                    Color32::WHITE,
                );
            });
        },
    );
    capture.update_textures(&output.textures_delta);
    capture.paint(&context, output);
    assert_eq!(capture.pixels[16 * 64 + 16], [120, 30, 60, 255]);
    let png = capture.png();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(png.windows(4).any(|bytes| bytes == b"IHDR"));
    assert!(png.windows(4).any(|bytes| bytes == b"IDAT"));
    assert_eq!(&png[png.len() - 8..png.len() - 4], b"IEND");
}

#[test]
#[ignore = "opt-in headless image export; set UNDERGLOW_UI_SNAPSHOTS to an output directory"]
fn export_workspace_snapshots() {
    use super::workspace_test_ui::Harness;
    let destination = std::env::var_os("UNDERGLOW_UI_SNAPSHOTS")
        .expect("set UNDERGLOW_UI_SNAPSHOTS to an explicit output directory");
    let destination = std::path::PathBuf::from(destination);
    std::fs::create_dir_all(&destination).unwrap();
    for size in [[940, 850], [620, 640], [1280, 800]] {
        for state in STATES.iter().chain(["where", "when", "testing"].iter()) {
            let mut harness = Harness::new(egui::vec2(size[0] as f32, size[1] as f32));
            let section = match *state {
                "where" => Some("Edit keys"),
                "when" => Some("Edit condition"),
                _ => None,
            };
            let (mut app, incoming) = state_fixture(if section.is_some() || *state == "testing" {
                "normal"
            } else {
                state
            });
            if *state == "testing" {
                app.setup_editor
                    .preview
                    .set_mode(super::setup_preview::Mode::Live);
            }
            let mut capture = Capture::new(size);
            if let Some(action) = section {
                // Use actual UI actions and retain every texture delta while
                // opening a section. Do not synthesize a second editing model.
                for _ in 0..2 {
                    let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
                    capture.update_textures(&frame.output.textures_delta);
                }
                let mut pos = None;
                for _ in 0..40 {
                    let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
                    capture.update_textures(&frame.output.textures_delta);
                    if let Some(hit) = frame
                        .texts()
                        .iter()
                        .find(|t| t.text == action && t.fully_visible())
                    {
                        pos = Some(hit.rect.center());
                        break;
                    }
                    let frame = harness.wheel(
                        egui::pos2(size[0] as f32 - 70.0, size[1] as f32 - 30.0),
                        egui::vec2(0.0, -60.0),
                        |ctx| app.show_lighting(ctx),
                    );
                    capture.update_textures(&frame.output.textures_delta);
                }
                let pos = pos.expect("reachable card action through real scrolling");
                for pressed in [true, false] {
                    let frame = harness
                        .run(super::workspace_test_ui::pointer(pos, pressed), |ctx| {
                            app.show_lighting(ctx)
                        });
                    capture.update_textures(&frame.output.textures_delta);
                }
            }
            if *state == "testing" {
                // These are independent actual UI selections, not injected
                // appearance state or fabricated live connection observations.
                for label in [
                    "Idle / fallback",
                    "Working",
                    "Simulate activity…",
                    "Finished",
                ] {
                    capture_click_label(&mut capture, &mut harness, &mut app, label);
                }
                assert!(app.setup_editor.preview.quick_active());
                assert!(app.setup_editor.preview.sample().is_none());
                assert!(!app.setup_editor.dirty);
            }
            for index in 0..30 {
                let frame = harness.run(vec![], |context| app.show_lighting(context));
                capture.update_textures(&frame.output.textures_delta);
                if index == 29 {
                    capture.paint(&harness.context, frame.output);
                }
            }
            assert!(
                incoming.try_recv().is_err(),
                "capturing must not dispatch actions"
            );
            let suffix = if *state == "normal" {
                String::new()
            } else {
                format!("-{state}")
            };
            let path = destination.join(format!("editor{suffix}-{}x{}.png", size[0], size[1]));
            std::fs::write(&path, capture.png()).unwrap();
            eprintln!("Headless software capture (not native): {}", path.display());
        }
    }
}

/// Capture actual input frames as well as settled frames so font-atlas patches
/// introduced by popups are never dropped from software screenshots.
fn capture_click_label(
    capture: &mut Capture,
    harness: &mut super::workspace_test_ui::Harness,
    app: &mut super::Controller,
    label: &str,
) {
    for step in 0..80 {
        for _ in 0..30 {
            let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
            capture.update_textures(&frame.output.textures_delta);
        }
        let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
        capture.update_textures(&frame.output.textures_delta);
        let hits: Vec<_> = frame
            .texts()
            .into_iter()
            .filter(|t| t.text == label && t.fully_visible())
            .collect();
        if hits.len() == 1 {
            for pressed in [true, false] {
                let frame = harness.run(
                    super::workspace_test_ui::pointer(hits[0].rect.center(), pressed),
                    |ctx| app.show_lighting(ctx),
                );
                capture.update_textures(&frame.output.textures_delta);
            }
            return;
        }
        assert!(
            hits.is_empty(),
            "ambiguous actual capture action {label}: {hits:?}"
        );
        let pane = frame
            .texts()
            .into_iter()
            .find(|t| t.text == "Layer actions")
            .expect("pinned inspector context")
            .clip;
        let frame = harness.wheel(
            egui::pos2(pane.center().x, pane.bottom() - 20.0),
            egui::vec2(
                0.0,
                if step == 39 {
                    20_000.0
                } else {
                    -(pane.height() * 0.45).min(100.0)
                },
            ),
            |ctx| app.show_lighting(ctx),
        );
        capture.update_textures(&frame.output.textures_delta);
    }
    panic!("unreachable actual capture action {label}");
}

const STATES: &[&str] = &[
    "normal",
    "empty",
    "dirty",
    "pending",
    "disconnected",
    "stale",
    "error",
    "legacy",
    "long-content",
    "many-layers",
];

/// Explicit fixtures, not captured user configuration or real transport errors.
fn state_fixture(state: &str) -> (super::Controller, super::Receiver<super::Action>) {
    use super::{setup_preview, tests::controller_fixture};
    use underglow::setup_library::{Starter, starter};
    let (mut app, incoming) = controller_fixture();
    let doc = starter(Starter::MatrixPi, "snapshot".into(), "Matrix + Pi".into()).unwrap();
    app.setup_editor.load(Some(doc.clone()));
    app.setup_editor
        .preview
        .set_mode(setup_preview::Mode::Scenario);
    app.setup_editor.preview.update(Some(&doc), app.brightness);
    app.setup_editor
        .preview
        .scenario
        .publish_state("pi", "preview-session", "working", 15_000)
        .unwrap();
    // Observe activation before advancing, so a paused capture samples the
    // actual working animation rather than its initial, potentially dark frame.
    app.setup_editor.preview.update(Some(&doc), app.brightness);
    app.setup_editor
        .preview
        .scenario
        .advance_by(std::time::Duration::from_millis(500))
        .unwrap();
    app.dev_simulation = true;
    app.message.clear();
    match state {
        "normal" => {}
        "long-content" => {
            let mut long = doc.clone();
            long.name = "A deliberately long development setup name that must not hide Save".into();
            long.layers[1].name =
                "A deliberately long status-layer name with useful context".into();
            long.layers[1].variants[0].name =
                "Working across a deliberately long custom appearance name".into();
            app.setup_editor.load(Some(long));
        }
        "many-layers" => {
            let mut many = doc.clone();
            for index in 0..24 {
                let mut layer = doc.layers[0].clone();
                layer.id = format!("snapshot-ambient-{index}");
                layer.name = format!("Ambient layer {index:02}");
                many.layers.insert(0, layer);
            }
            app.setup_editor.load(Some(many));
            app.setup_editor.selected = Some("pi-status".into());
        }
        "empty" => app.setup_editor.load(Some(
            starter(Starter::Blank, "empty".into(), "Blank".into()).unwrap(),
        )),
        "dirty" | "pending" | "disconnected" | "error" => {
            app.setup_editor.name_draft("Edited setup".into()).unwrap();
            match state {
                "pending" => {
                    app.pending = true;
                    app.message = "Saving setup…".into();
                }
                "disconnected" => {
                    app.connected = false;
                    app.message = "Engine unavailable. Your draft is retained.".into();
                }
                "error" => {
                    app.message = "Save failed: revision conflict. Your draft is retained.".into()
                }
                _ => {}
            }
        }
        "stale" => app
            .setup_editor
            .preview
            .set_sample(Some(setup_preview::LiveSample {
                generation: (1, 1),
                now: std::time::Duration::from_millis(100),
                received_at: std::time::Instant::now(),
                connected: true,
                stale: true,
                snapshot: underglow::external::Snapshot {
                    revision: 1,
                    event_cursor: 0,
                    states: vec![],
                },
                events: underglow::external::EventBatch {
                    cursor: 0,
                    gap: false,
                    events: vec![],
                },
            })),
        "legacy" => {
            app.setup_editor = super::setup_editor::Editor::default();
            app.setup_checked = true;
        }
        _ => panic!("unknown workspace fixture {state}"),
    }
    (app, incoming)
}

#[test]
fn representative_states_keep_navigation_and_preview_truthful_at_real_sizes() {
    use super::workspace_test_ui::{DEFAULT_SIZE, Harness, MINIMUM_SIZE, WIDE_SIZE};
    for size in [DEFAULT_SIZE, MINIMUM_SIZE, WIDE_SIZE] {
        for state in STATES {
            let (mut app, incoming) = state_fixture(state);
            let mut harness = Harness::new(size);
            let frame = harness.settle(|ctx| app.show_lighting(ctx));
            let texts = frame.texts();
            for label in ["Editor", "Connections", "Settings", "Hardware disabled"] {
                assert!(
                    texts.iter().any(|t| t.text == label && t.fully_visible()),
                    "{state} at {size:?}: hidden primary control {label}"
                );
            }
            if ["dirty", "pending", "disconnected", "error"].contains(state) {
                assert!(app.setup_editor.dirty);
                assert!(
                    texts
                        .iter()
                        .any(|t| t.text == "Unsaved changes" && t.fully_visible())
                );
                assert!(texts.iter().any(|t| t.text == "Save" && t.fully_visible()));
            }
            if *state == "legacy" {
                assert!(
                    texts
                        .iter()
                        .any(|t| t.text == "Keep current behavior" && t.fully_visible())
                );
                assert!(
                    texts
                        .iter()
                        .any(|t| t.text == "Begin new setup" && t.fully_visible())
                );
            } else if *state == "stale" {
                assert!(app.setup_editor.preview.sample().unwrap().stale);
                assert!(
                    app.setup_editor
                        .connection_notices
                        .iter()
                        .any(|n| n.message.contains("Live data delayed"))
                );
            } else {
                assert!(
                    app.setup_editor.preview.sample().is_none(),
                    "private facts cannot become live health"
                );
            }
            assert!(
                incoming.try_recv().is_err(),
                "painting {state} must not dispatch engine actions"
            );
        }
    }
}

#[test]
#[ignore = "opt-in Connections captures; set UNDERGLOW_CONNECTION_SNAPSHOTS"]
fn export_connection_snapshots() {
    use super::workspace_test_ui::Harness;
    let destination = std::path::PathBuf::from(
        std::env::var_os("UNDERGLOW_CONNECTION_SNAPSHOTS")
            .expect("set UNDERGLOW_CONNECTION_SNAPSHOTS to an explicit output directory"),
    );
    std::fs::create_dir_all(&destination).unwrap();
    for size in [[940, 850], [620, 640], [1280, 800]] {
        for state in ["overview", "details", "no-data", "delayed", "missing"] {
            let (mut app, incoming) = super::secondary_flow_tests::connections_fixture();
            let mut sample = app.setup_editor.preview.sample().unwrap().clone();
            if state == "no-data" {
                sample.snapshot.states.clear();
            }
            if state == "delayed" {
                sample.stale = true;
            }
            if state == "missing" {
                app.connections_panel
                    .receive_report(Ok(serde_json::json!({"ok":true,
                    "connections":{"registry":{"schema_version":1,"revision":1,"connections":[]},
                    "runtime_index":{}}})
                    .to_string()));
            }
            app.setup_editor.preview.set_sample(Some(sample));
            let mut harness = Harness::new(egui::vec2(size[0] as f32, size[1] as f32));
            let mut capture = Capture::new(size);
            if state == "details" {
                capture_click_label(&mut capture, &mut harness, &mut app, "App details");
            }
            for index in 0..30 {
                let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
                capture.update_textures(&frame.output.textures_delta);
                if index == 29 {
                    capture.paint(&harness.context, frame.output);
                }
            }
            assert!(
                incoming.try_recv().is_err(),
                "capture must not dispatch any action"
            );
            let path = destination.join(format!("connections-{state}-{}x{}.png", size[0], size[1]));
            std::fs::write(&path, capture.png()).unwrap();
            eprintln!("Headless software capture (not native): {}", path.display());
        }
    }
}

#[test]
#[ignore = "opt-in Finished duration captures; set UNDERGLOW_FINISHED_SNAPSHOTS"]
fn export_finished_duration_snapshots() {
    use super::workspace_test_ui::Harness;
    let destination = std::path::PathBuf::from(
        std::env::var_os("UNDERGLOW_FINISHED_SNAPSHOTS")
            .expect("set UNDERGLOW_FINISHED_SNAPSHOTS to an explicit output directory"),
    );
    std::fs::create_dir_all(&destination).unwrap();
    for size in [[940, 850], [620, 640], [1280, 800]] {
        let (mut app, incoming) = state_fixture("normal");
        let mut doc = app.setup_editor.draft.clone().unwrap();
        if let underglow::setup::Activation::OnEvent { duration_ms, .. } =
            &mut doc.layers[1].variants[1].activation
        {
            *duration_ms = 5250;
        }
        app.setup_editor.load(Some(doc));
        app.setup_editor
            .preview
            .set_mode(super::setup_preview::Mode::Live);
        let mut harness = Harness::new(egui::vec2(size[0] as f32, size[1] as f32));
        let mut capture = Capture::new(size);
        for label in [
            "Idle / fallback",
            "Finished (settled, not success)",
            "Simulate activity…",
            "Finished",
            "Pause simulation",
        ] {
            capture_click_label(&mut capture, &mut harness, &mut app, label);
        }
        for index in 0..30 {
            let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
            capture.update_textures(&frame.output.textures_delta);
            if index == 29 {
                capture.paint(&harness.context, frame.output);
            }
        }
        assert!(incoming.try_recv().is_err());
        assert!(app.setup_editor.preview.sample().is_none());
        let path = destination.join(format!("finished-duration-{}x{}.png", size[0], size[1]));
        std::fs::write(&path, capture.png()).unwrap();
        eprintln!("Headless software capture (not native): {}", path.display());
    }
}
#[test]
#[ignore = "opt-in Sweep background captures; set UNDERGLOW_SWEEP_SNAPSHOTS"]
fn export_sweep_background_snapshots() {
    use super::workspace_test_ui::Harness;
    use underglow::setup::{Renderer, SweepBackground};
    let destination = std::path::PathBuf::from(
        std::env::var_os("UNDERGLOW_SWEEP_SNAPSHOTS")
            .expect("set UNDERGLOW_SWEEP_SNAPSHOTS to an explicit output directory"),
    );
    std::fs::create_dir_all(&destination).unwrap();
    for size in [[940, 850], [620, 640], [1280, 800]] {
        for state in ["whole-solid", "pi-solid", "pi-transparent"] {
            let (mut app, incoming) = state_fixture("normal");
            let mut doc = app.setup_editor.draft.clone().unwrap();
            let background = if state == "pi-transparent" {
                SweepBackground::Transparent {}
            } else {
                SweepBackground::Solid { color: [24, 4, 38] }
            };
            if state == "whole-solid" {
                doc.layers.truncate(1);
                doc.layers[0].renderer = Renderer::Sweep {
                    color: [0, 180, 255],
                    period_ms: 1300,
                    background,
                };
            } else {
                if let Renderer::Sweep {
                    background: saved, ..
                } = &mut doc.layers[1].variants[0].renderer
                {
                    *saved = background;
                }
            }
            let selected = doc.layers.last().unwrap().id.clone();
            app.setup_editor.load(Some(doc));
            app.setup_editor.selected = Some(selected);
            let mut harness = Harness::new(egui::vec2(size[0] as f32, size[1] as f32));
            let mut capture = Capture::new(size);
            if state != "whole-solid" {
                for label in ["Idle / fallback", "Working"] {
                    capture_click_label(&mut capture, &mut harness, &mut app, label);
                }
            }
            capture_click_label(
                &mut capture,
                &mut harness,
                &mut app,
                if state == "pi-transparent" {
                    "Transparent"
                } else {
                    "Solid color"
                },
            );
            // Close the real picker without altering the configured mode.
            for pressed in [true, false] {
                let frame = harness.run(
                    vec![egui::Event::Key {
                        key: egui::Key::Escape,
                        physical_key: None,
                        pressed,
                        repeat: false,
                        modifiers: Default::default(),
                    }],
                    |ctx| app.show_lighting(ctx),
                );
                capture.update_textures(&frame.output.textures_delta);
            }
            // Bring the whole Sweep control group into view with real wheel
            // input; clicking Background alone can leave Active color clipped.
            for step in 0..24 {
                for _ in 0..30 {
                    let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
                    capture.update_textures(&frame.output.textures_delta);
                }
                let frame = harness.run(vec![], |ctx| app.show_lighting(ctx));
                capture.update_textures(&frame.output.textures_delta);
                let texts = frame.texts();
                let labels = ["Active color", "Background", "Cycle duration (ms)"];
                let visible = |label: &str| {
                    texts.iter().any(|text| {
                        text.text == label
                            && text.fully_visible()
                            && text.rect.top() - 6.0 >= text.clip.top()
                            && text.rect.bottom() + 6.0 <= text.clip.bottom()
                    })
                };
                if labels.iter().all(|label| visible(label)) {
                    capture.paint(&harness.context, frame.output);
                    break;
                }
                assert!(
                    step < 23,
                    "Sweep controls cannot be fully revealed at {size:?}"
                );
                let active = texts.iter().find(|text| text.text == "Active color");
                let up = active.is_none_or(|text| text.rect.top() - 6.0 < text.clip.top());
                let pane = texts
                    .iter()
                    .find(|text| text.text == "Layer actions")
                    .unwrap()
                    .clip;
                let frame = harness.wheel(
                    egui::pos2(pane.center().x, pane.bottom() - 20.0),
                    egui::vec2(0.0, if up { 12.0 } else { -12.0 }),
                    |ctx| app.show_lighting(ctx),
                );
                capture.update_textures(&frame.output.textures_delta);
            }
            assert!(incoming.try_recv().is_err());
            assert!(app.setup_editor.preview.sample().is_none());
            let path = destination.join(format!("sweep-{state}-{}x{}.png", size[0], size[1]));
            std::fs::write(&path, capture.png()).unwrap();
        }
    }
}
