//! Headless egui interaction helpers; no window, engine, producer or SDK.
use eframe::egui;

pub fn draw<T>(
    ctx: &egui::Context,
    events: Vec<egui::Event>,
    mut show: impl FnMut(&mut egui::Ui) -> T,
) -> (egui::FullOutput, T) {
    // Parallel UI contexts need independent clocks: a shared counter can turn
    // another test's press/release into a long press instead of a click.
    let time = ctx.input(|input| input.time) + 0.25;
    let mut result = None;
    let output = ctx.run(
        egui::RawInput {
            time: Some(time),
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1500.0, 18000.0),
            )),
            events,
            ..Default::default()
        },
        |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                result = Some(show(ui));
            });
        },
    );
    (output, result.unwrap())
}

pub fn position(output: &egui::FullOutput, label: &str) -> egui::Pos2 {
    output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if text.galley.text() == label => {
                Some(text.pos + text.galley.size() * 0.5)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("missing {label}"))
}

pub fn click<T>(ctx: &egui::Context, label: &str, mut show: impl FnMut(&mut egui::Ui) -> T) -> T {
    draw(ctx, vec![], &mut show);
    let (output, _) = draw(ctx, vec![], &mut show);
    let pos = position(&output, label);
    draw(ctx, pointer(pos, true), &mut show);
    draw(ctx, pointer(pos, false), show).1
}

fn pointer(pos: egui::Pos2, pressed: bool) -> Vec<egui::Event> {
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

#[test]
fn independent_contexts_do_not_advance_each_others_input_clock() {
    let first = egui::Context::default();
    let second = egui::Context::default();
    let (_, before) = draw(&first, vec![], |ui| ui.input(|input| input.time));
    for _ in 0..10 {
        draw(&second, vec![], |_| ());
    }
    let (_, after) = draw(&first, vec![], |ui| ui.input(|input| input.time));
    assert_eq!(after - before, 0.25);
}
