//! Repeatable hardware checks. SDK acknowledgements are not visual verification.
use crate::effects::EffectKind;
use crate::runner::{RunOptions, run_effect};
use crate::sdk::analog::AnalogSdk;
use crate::sdk::rgb::{Layout, WootingRgb};
use clap::Args;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Debug, Args)]
pub struct DoctorOptions {
    /// Explicitly paint a short comet animation before restoring lighting.
    #[arg(long)]
    probe_rgb: bool,
    #[arg(long, requires = "probe_rgb", default_value_t = 3, value_parser = clap::value_parser!(u64).range(1..=30))]
    seconds: u64,
    #[arg(long, requires = "probe_rgb", default_value_t = underglow::DEFAULT_BRIGHTNESS)]
    brightness: u8,
    /// Also check the ripple Analog SDK backend (one 80HE); not needed for RGB effects.
    #[arg(long)]
    analog: bool,
    #[arg(long, requires = "analog")]
    analog_sdk_path: Option<PathBuf>,
    /// Emit a structured report to stdout. Diagnostics do not log pressed keys.
    #[arg(long)]
    json: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
enum Status {
    Passed,
    Failed,
    Skipped,
}

#[derive(Serialize)]
struct Check {
    name: &'static str,
    status: Status,
    detail: String,
}

#[derive(Serialize)]
struct Report {
    schema_version: u8,
    ok: bool,
    interrupted: bool,
    checks: Vec<Check>,
    manual_checks: [&'static str; 3],
}

impl Report {
    fn new() -> Self {
        Self {
            schema_version: 1,
            ok: true,
            interrupted: false,
            checks: Vec::new(),
            manual_checks: [
                "Confirm normal lighting returns; reset acknowledgement alone cannot prove this.",
                "Confirm Wootility can edit lighting after this command exits.",
                "Record Wootility/Background Service state and any flicker or competing profile changes.",
            ],
        }
    }

    fn add(&mut self, name: &'static str, status: Status, detail: impl Into<String>) {
        if matches!(status, Status::Failed) {
            self.ok = false;
        }
        self.checks.push(Check {
            name,
            status,
            detail: detail.into(),
        });
    }

    fn result(
        &mut self,
        name: &'static str,
        result: Result<(), impl std::fmt::Display>,
        success: &str,
    ) {
        match result {
            Ok(()) => self.add(name, Status::Passed, success),
            Err(error) => self.add(name, Status::Failed, error.to_string()),
        }
    }
}

pub fn run(
    options: DoctorOptions,
    rgb_path: Option<&Path>,
    interrupted: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    // Even enumeration calls RGB SDK initialization, which can acquire/reset RGB.
    // Never describe this command as read-only or use it as a passive monitor.
    eprintln!(
        "Doctor opens and closes an RGB session; even without --probe-rgb this may change lighting. No keyboard configuration is written."
    );
    let mut report = Report::new();
    let mut rgb_usable = false;
    let mut keyboard = match WootingRgb::open(rgb_path) {
        Ok(keyboard) => {
            report.add(
                "rgb-open",
                Status::Passed,
                "RGB SDK opened a keyboard; enumeration alone is not proof of working RGB commands",
            );
            let info = keyboard.info();
            let valid = info.connected
                && info.max_rows > 0
                && info.max_columns > 0
                && info.layout != Layout::Unknown;
            rgb_usable = valid;
            report.add("rgb-metadata", if valid { Status::Passed } else { Status::Failed },
                format!("{}: {}x{}, {:?}; multi-report={}.{}", info.model, info.max_rows, info.max_columns, info.layout, info.uses_multi_report,
                    if valid { "" } else { " Rebuild the pinned RGB SDK; check USB access and competing writers. Do not infer Wootility contention from this alone." }));
            Some(keyboard)
        }
        Err(error) => {
            report.add("rgb-open", Status::Failed, error.to_string());
            None
        }
    };

    // Optional: RGB-only diagnostics must work without any analog library installed.
    let mut analog = if options.analog && !interrupted.load(Ordering::SeqCst) {
        match AnalogSdk::open(options.analog_sdk_path.as_deref()) {
            Ok(analog) => {
                report.add("analog-open", Status::Passed, "Analog SDK opened one 80HE");
                Some(analog)
            }
            Err(error) => {
                report.add("analog-open", Status::Failed, error.to_string());
                None
            }
        }
    } else {
        report.add(
            "analog-open",
            Status::Skipped,
            "Not requested or interrupted; analog is optional for RGB effects",
        );
        None
    };

    if let Some(keyboard) = keyboard.as_ref() {
        if options.probe_rgb && rgb_usable && !interrupted.load(Ordering::SeqCst) {
            eprintln!(
                "Running {}-second RGB probe at brightness {}; Ctrl-C stops and attempts restoration.",
                options.seconds, options.brightness
            );
            let result = run_effect(
                keyboard,
                &RunOptions {
                    effect: EffectKind::Comet,
                    brightness: options.brightness,
                    seconds: Some(options.seconds),
                    fps: 30,
                    ..RunOptions::default()
                },
                interrupted,
            );
            if interrupted.load(Ordering::SeqCst) && result.is_ok() {
                report.add(
                    "rgb-probe",
                    Status::Skipped,
                    "Interrupted; requested probe duration was not completed",
                );
            } else {
                report.result("rgb-probe", result, "Frame writes completed without SDK errors; visual appearance still needs confirmation");
            }
        } else {
            report.add("rgb-probe", Status::Skipped, "Not requested, metadata invalid, or interrupted; use --probe-rgb for an explicit lighting test");
        }
    } else {
        report.add("rgb-probe", Status::Skipped, "No RGB session");
    }

    if let Some(analog) = analog.as_ref() {
        if interrupted.load(Ordering::SeqCst) {
            report.add("analog-read", Status::Skipped, "Interrupted");
        } else {
            report.result(
                "analog-read",
                analog.read().map(|_| ()),
                "One buffer read succeeded (empty is valid); no key contents logged",
            );
        }
    }

    // No early returns after opening a session: report cleanup even when the
    // probe/analog read fails. Close before printing so clients see final status.
    if let Some(keyboard) = keyboard.as_mut() {
        report.result(
            "rgb-restore",
            keyboard.close(),
            "SDK acknowledged restore/close; confirm normal lighting visually",
        );
    } else {
        report.add(
            "rgb-restore",
            Status::Skipped,
            "No completed RGB session; any setup cleanup warnings are on stderr",
        );
    }
    if let Some(analog) = analog.as_mut() {
        report.result(
            "analog-close",
            analog.close(),
            "Analog SDK acknowledged uninitialisation",
        );
    }
    drop(analog);
    report.interrupted = interrupted.load(Ordering::SeqCst);
    report.ok &= !report.interrupted;
    if options.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
    } else {
        for check in &report.checks {
            let status = match check.status {
                Status::Passed => "PASS",
                Status::Failed => "FAIL",
                Status::Skipped => "SKIP",
            };
            println!("{status} {}: {}", check.name, check.detail);
        }
        for manual in report.manual_checks {
            println!("MANUAL: {manual}");
        }
        println!(
            "SDK checks: {}",
            if report.ok {
                "passed (not a visual/coexistence certification)"
            } else {
                "failed or interrupted"
            }
        );
    }
    if report.ok {
        Ok(())
    } else {
        Err("doctor checks failed or were interrupted; see report".into())
    }
}
