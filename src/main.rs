mod config;
mod doctor;
mod effects;
mod engine;
mod layout;
mod notifications;
mod ownership;
mod preview;
mod profile;
mod render;
mod runner;
mod scenes;
mod sdk;
mod signals;
mod toys;

use clap::{Parser, Subcommand};
use config::AppConfig;
use effects::EffectKind;
use layout::KeyboardLayout;
use preview::PreviewFormat;
use profile::ProfileRuntimeSignal;
use render::{Color, PaletteName};
use runner::{RunOptions, SignalRunOptions, run_session, sleep_interruptibly};
use sdk::rgb::{DeviceInfo, WootingRgb};
use signals::{
    AppAuraConfig, CommandPulseConfig, CommandPulseOutput, FixtureConfig, FocusConfig,
    GitHubCiConfig, MarketConfig, SignalKind, SoundwaveConfig, SportsConfig, build_signal,
};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

#[derive(Debug, Parser)]
#[command(author, version, about)]
struct Cli {
    /// Path to libwooting-rgb-sdk.dylib/.so or wooting-rgb-sdk.dll.
    #[arg(long, global = true)]
    sdk_path: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::large_enum_variant)]
enum Command {
    /// Run the per-user engine. Fresh state starts paused; opt in with control resume.
    Engine(engine::EngineOptions),
    /// Control a running engine over its user-private Unix socket; emits JSON.
    Control(engine::ControlOptions),
    /// Check SDK access and restoration (opens/resets RGB; opt in to an animation).
    Doctor(doctor::DoctorOptions),
    /// Play an interactive keyboard toy; stop to restore normal lighting.
    Toy {
        #[command(subcommand)]
        command: toys::ToyCommand,
    },
    /// Print connected keyboard metadata.
    Info,
    /// Print the inferred keyboard layout summary.
    LayoutInfo,
    /// Paint a short row test pattern, then reset.
    Test {
        /// Maximum RGB channel value.
        #[arg(long, default_value_t = 96)]
        brightness: u8,
        /// Seconds to keep the pattern visible.
        #[arg(long, default_value_t = 3)]
        seconds: u64,
    },
    /// Try the SDK direct single-key feature call, then reset.
    Direct {
        /// Matrix row to light.
        #[arg(long, default_value_t = 0)]
        row: u8,
        /// Matrix column to light.
        #[arg(long, default_value_t = 0)]
        column: u8,
        /// Maximum RGB channel value.
        #[arg(long, default_value_t = 96)]
        brightness: u8,
        /// Seconds to keep the key visible.
        #[arg(long, default_value_t = 3)]
        seconds: u64,
    },
    /// Run a device-bounded rainbow animation, then reset.
    Rainbow {
        /// Maximum RGB channel value.
        #[arg(long, default_value_t = 128)]
        brightness: u8,
        /// Seconds to run the animation.
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// Animation frames per second.
        #[arg(long, default_value_t = 30)]
        fps: u32,
    },
    /// Run any named RGB demo effect.
    Effect {
        /// Effect to run.
        #[arg(value_enum)]
        effect: EffectKind,
        /// Palette for palette-aware effects.
        #[arg(long, value_enum, default_value_t = PaletteName::Wooting)]
        palette: PaletteName,
        /// Maximum RGB channel value.
        #[arg(long, default_value_t = 128)]
        brightness: u8,
        /// Seconds to run the effect.
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// Animation frames per second.
        #[arg(long, default_value_t = 30)]
        fps: u32,
    },
    /// Run a signal directly.
    #[command(alias = "extension")]
    Signal {
        #[command(subcommand)]
        command: SignalCommand,
    },
    /// Preview deterministic frames without opening the keyboard SDK.
    Preview {
        #[command(subcommand)]
        command: PreviewCommand,
    },
    /// Run a TOML signals profile.
    Run {
        /// Config file path.
        #[arg(long)]
        config: PathBuf,
        /// Print resolved config without touching the keyboard.
        #[arg(long)]
        dry_run: bool,
        /// Render a preview without touching the keyboard.
        #[arg(long)]
        preview: bool,
        /// Preview output format.
        #[arg(long, value_enum, default_value_t = PreviewFormat::Ansi)]
        preview_format: PreviewFormat,
        /// Number of preview ticks to render.
        #[arg(long, default_value_t = 3)]
        preview_ticks: u32,
    },
}

#[derive(Debug, Subcommand)]
enum PreviewCommand {
    /// Preview a static effect.
    Effect {
        /// Effect to preview.
        #[arg(value_enum)]
        effect: EffectKind,
        /// Palette for palette-aware effects.
        #[arg(long, value_enum, default_value_t = PaletteName::Wooting)]
        palette: PaletteName,
        /// Maximum RGB channel value.
        #[arg(long, default_value_t = 128)]
        brightness: u8,
        /// Number of ticks to render.
        #[arg(long, default_value_t = 3)]
        ticks: u32,
        /// Preview output format.
        #[arg(long, value_enum, default_value_t = PreviewFormat::Ansi)]
        format: PreviewFormat,
    },
}

#[derive(Debug, Subcommand)]
enum SignalCommand {
    /// Run a named signal. Use `--` before command-pulse commands.
    Run {
        /// Signal to run.
        #[arg(value_enum)]
        signal: SignalKind,
        /// Static effect used by the static-effect signal.
        #[arg(long, value_enum, default_value_t = EffectKind::Comet)]
        effect: EffectKind,
        /// Palette for signal renderers.
        #[arg(long, value_enum, default_value_t = PaletteName::Wooting)]
        palette: PaletteName,
        /// Maximum RGB channel value.
        #[arg(long, default_value_t = 128)]
        brightness: u8,
        /// Animation frames per second.
        #[arg(long, default_value_t = 30)]
        fps: u32,
        /// Seconds to run static-effect or ripples.
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// Analog SDK distributable for ripples; otherwise use WOOTING_ANALOG_SDK_PATH.
        #[arg(long)]
        analog_sdk_path: Option<PathBuf>,
        /// Working directory for command-pulse.
        #[arg(long)]
        cwd: Option<PathBuf>,
        /// Environment override for command-pulse, in KEY=VALUE form. Repeatable.
        #[arg(long = "env", value_name = "KEY=VALUE")]
        env: Vec<String>,
        /// Command stdout/stderr policy for command-pulse.
        #[arg(long, value_enum, default_value_t = CommandPulseOutput::Inherit)]
        output: CommandPulseOutput,
        /// Print a command-pulse status summary when the command completes.
        #[arg(long)]
        summary: bool,
        /// Command timeout for command-pulse.
        #[arg(long, default_value_t = 600)]
        timeout_seconds: u64,
        /// Command-pulse success hold seconds.
        #[arg(long, default_value_t = 3)]
        success_hold_seconds: u64,
        /// Command-pulse failure/timeout hold seconds.
        #[arg(long, default_value_t = 6)]
        failure_hold_seconds: u64,
        /// Command-pulse interrupted hold seconds.
        #[arg(long, default_value_t = 2)]
        interrupted_hold_seconds: u64,
        /// GitHub repository for github-ci, in owner/repo form.
        #[arg(long)]
        repo: Option<String>,
        /// GitHub branch filter for github-ci.
        #[arg(long)]
        branch: Option<String>,
        /// GitHub pull request number for github-ci.
        #[arg(long)]
        pull_request: Option<u64>,
        /// Environment variable containing the GitHub token.
        #[arg(long, default_value = "GITHUB_TOKEN")]
        token_env: String,
        /// GitHub API base URL.
        #[arg(long, default_value = "https://api.github.com")]
        api_base: String,
        /// Provider API URL for market-pulse or sports-alerts.
        #[arg(long)]
        api_url: Option<String>,
        /// Watched market symbol. Repeatable.
        #[arg(long = "watchlist")]
        watchlist: Vec<String>,
        /// Market threshold percentage for alerts.
        #[arg(long, default_value_t = 2.0)]
        threshold_percent: f64,
        /// Favorite team/driver for sports alerts. Repeatable.
        #[arg(long = "favorite")]
        favorites: Vec<String>,
        /// External source polling interval.
        #[arg(long, default_value_t = 60)]
        poll_seconds: u64,
        /// Focus phase duration for focus-cockpit.
        #[arg(long, default_value_t = 25)]
        focus_minutes: u64,
        /// Break phase duration for focus-cockpit.
        #[arg(long, default_value_t = 5)]
        break_minutes: u64,
        /// Number of focus/break cycles before overtime.
        #[arg(long, default_value_t = 1)]
        cycles: u32,
        /// Start focus-cockpit in paused mode.
        #[arg(long)]
        start_paused: bool,
        /// Start focus-cockpit in meeting-safe dim mode.
        #[arg(long)]
        meeting_safe: bool,
        /// Dim focus-cockpit or app-aura output.
        #[arg(long)]
        dim: bool,
        /// Manual App Aura profile.
        #[arg(long, default_value = "manual")]
        profile: String,
        /// Permit future macOS Accessibility automation for app-aura.
        #[arg(long)]
        accessibility_enabled: bool,
        /// Enable soundwave rendering. Disabled by default.
        #[arg(long)]
        enabled: bool,
        /// Manual soundwave level for the opt-in prototype.
        #[arg(long, default_value_t = 0.0)]
        level: f32,
        /// Manual soundwave bass level for the opt-in prototype.
        #[arg(long, default_value_t = 0.0)]
        bass: f32,
        /// CPU limit target for future sound capture.
        #[arg(long, default_value_t = 10)]
        cpu_limit_percent: u8,
        /// Seconds before GitHub status is treated as stale.
        #[arg(long, default_value_t = 300)]
        stale_seconds: u64,
        /// Command and args for command-pulse.
        #[arg(last = true, allow_hyphen_values = true)]
        command: Vec<String>,
    },
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();
    let interrupted = install_ctrlc_handler()?;

    let needs_hardware = match &cli.command {
        Command::Engine(_) | Command::Control(_) | Command::Preview { .. } => false,
        Command::Run {
            dry_run, preview, ..
        } => !dry_run && !preview,
        Command::Toy {
            command: toys::ToyCommand::Ripples(options),
        } => !options.preview,
        _ => true,
    };
    let _hardware = if needs_hardware {
        Some(ownership::Lease::acquire("hardware")?)
    } else {
        None
    };

    match cli.command {
        Command::Engine(options) => engine::run(options, cli.sdk_path, &interrupted)?,
        Command::Control(options) => engine::control(options)?,
        Command::Doctor(options) => doctor::run(options, cli.sdk_path.as_deref(), &interrupted)?,
        Command::Toy { command } => toys::run(command, cli.sdk_path.as_deref(), &interrupted)?,
        Command::Info => {
            let keyboard = WootingRgb::open(cli.sdk_path.as_deref())?;
            print_info(keyboard.info());
        }
        Command::LayoutInfo => {
            let keyboard = WootingRgb::open(cli.sdk_path.as_deref())?;
            print_info(keyboard.info());
            let layout = KeyboardLayout::for_device(keyboard.info());
            println!("layout: {}", layout.summary());
        }
        Command::Test {
            brightness,
            seconds,
        } => {
            let options = RunOptions {
                effect: EffectKind::RowTest,
                brightness,
                seconds: Some(seconds),
                ..RunOptions::default()
            };
            run_keyboard(cli.sdk_path, &options, &interrupted)?;
        }
        Command::Direct {
            row,
            column,
            brightness,
            seconds,
        } => {
            let mut keyboard = WootingRgb::open(cli.sdk_path.as_deref())?;
            print_info(keyboard.info());
            keyboard.direct_set_key(row, column, Color::new(brightness, brightness, brightness))?;
            sleep_interruptibly(Duration::from_secs(seconds), &interrupted);
            close_best_effort(&mut keyboard, true);
        }
        Command::Rainbow {
            brightness,
            seconds,
            fps,
        } => {
            let options = RunOptions {
                effect: EffectKind::Rainbow,
                brightness,
                seconds: Some(seconds),
                fps,
                ..RunOptions::default()
            };
            run_keyboard(cli.sdk_path, &options, &interrupted)?;
        }
        Command::Effect {
            effect,
            palette,
            brightness,
            seconds,
            fps,
        } => {
            let options = RunOptions {
                effect,
                palette,
                brightness,
                seconds: Some(seconds),
                fps,
                continuous: false,
            };
            run_keyboard(cli.sdk_path, &options, &interrupted)?;
        }
        Command::Preview { command } => match command {
            PreviewCommand::Effect {
                effect,
                palette,
                brightness,
                ticks,
                format,
            } => {
                preview::print_effect_preview(effect, palette, brightness, ticks, format);
            }
        },
        Command::Signal { command } => match command {
            SignalCommand::Run {
                signal,
                effect,
                palette,
                brightness,
                fps,
                seconds,
                cwd,
                env,
                output,
                summary,
                timeout_seconds,
                success_hold_seconds,
                failure_hold_seconds,
                interrupted_hold_seconds,
                repo,
                branch,
                pull_request,
                token_env,
                api_base,
                api_url,
                watchlist,
                threshold_percent,
                favorites,
                poll_seconds,
                focus_minutes,
                break_minutes,
                cycles,
                start_paused,
                meeting_safe,
                dim,
                profile,
                accessibility_enabled,
                enabled,
                level,
                bass,
                cpu_limit_percent,
                stale_seconds,
                analog_sdk_path,
                command,
            } => {
                let env = parse_env_vars(env)?;
                let (config, options) = match signal {
                    SignalKind::Ripples => (
                        signals::SignalConfig::ripples(toys::RippleConfig {
                            analog_sdk_path,
                            ..Default::default()
                        }),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: Some(seconds),
                            continuous: false,
                        },
                    ),
                    SignalKind::StaticEffect => (
                        signals::SignalConfig::static_effect(effect),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: Some(seconds),
                            continuous: false,
                        },
                    ),
                    SignalKind::CommandPulse => (
                        signals::SignalConfig::command_pulse(CommandPulseConfig {
                            command,
                            cwd,
                            env,
                            output,
                            summary,
                            timeout_seconds,
                            success_hold_seconds,
                            failure_hold_seconds,
                            interrupted_hold_seconds,
                            ..CommandPulseConfig::default()
                        }),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: None,
                            continuous: true,
                        },
                    ),
                    SignalKind::GitHubCi => (
                        signals::SignalConfig::github_ci(GitHubCiConfig {
                            repo: repo.unwrap_or_default(),
                            branch,
                            pull_request,
                            token_env,
                            api_base,
                            poll_seconds,
                            stale_seconds,
                        }),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: None,
                            continuous: true,
                        },
                    ),
                    SignalKind::FocusCockpit => (
                        signals::SignalConfig::focus_cockpit(FocusConfig {
                            focus_minutes,
                            break_minutes,
                            cycles,
                            start_paused,
                            meeting_safe,
                            dim,
                        }),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: None,
                            continuous: true,
                        },
                    ),
                    SignalKind::MarketPulse => (
                        signals::SignalConfig::market_pulse(MarketConfig {
                            api_url: api_url.unwrap_or_default(),
                            token_env,
                            watchlist,
                            threshold_percent,
                            poll_seconds,
                            stale_seconds,
                        }),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: None,
                            continuous: true,
                        },
                    ),
                    SignalKind::SportsAlerts => (
                        signals::SignalConfig::sports_alerts(SportsConfig {
                            api_url: api_url.unwrap_or_default(),
                            token_env,
                            favorites,
                            poll_seconds,
                            stale_seconds,
                        }),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: None,
                            continuous: true,
                        },
                    ),
                    SignalKind::AppAura => (
                        signals::SignalConfig::app_aura(AppAuraConfig {
                            profile: parse_app_aura_profile(&profile)?,
                            accessibility_enabled,
                            dim,
                        }),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: None,
                            continuous: true,
                        },
                    ),
                    SignalKind::Soundwave => (
                        signals::SignalConfig::soundwave(SoundwaveConfig {
                            enabled,
                            level,
                            bass,
                            cpu_limit_percent,
                        }),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: None,
                            continuous: true,
                        },
                    ),
                    SignalKind::FixtureReplay => (
                        signals::SignalConfig::fixture_replay(FixtureConfig::default()),
                        SignalRunOptions {
                            palette,
                            brightness,
                            fps,
                            seconds: None,
                            continuous: true,
                        },
                    ),
                };
                let mut signal = build_signal(&config, effect)?;
                run_keyboard_signal(cli.sdk_path, &options, &mut *signal, true, &interrupted)?;
            }
        },
        Command::Run {
            config,
            dry_run,
            preview,
            preview_format,
            preview_ticks,
        } => {
            let config = AppConfig::load(&config)?;
            if preview {
                let mut signal = build_config_signal(&config)?;
                preview::print_signal_preview(
                    &mut *signal,
                    &config.signal_run_options(),
                    preview_ticks,
                    preview_format,
                );
            } else if !dry_run {
                let sdk_path = cli.sdk_path.or(config.sdk_path.clone());
                let mut signal = build_config_signal(&config)?;
                run_keyboard_signal(
                    sdk_path,
                    &config.signal_run_options(),
                    &mut *signal,
                    config.warn_on_close_error,
                    &interrupted,
                )?;
            } else {
                // Validate constructors without acquiring hardware or starting providers.
                let _ = build_config_signal(&config)?;
                print_config(&config);
            }
        }
    }

    Ok(())
}

fn build_config_signal(
    config: &AppConfig,
) -> Result<Box<dyn signals::SignalProgram>, Box<dyn std::error::Error>> {
    config.validate()?;
    let base: Box<dyn signals::SignalProgram> =
        if ProfileRuntimeSignal::is_profile_runtime_config(config) {
            Box::new(ProfileRuntimeSignal::new(config.clone())?)
        } else {
            build_signal(&config.signal_config(), config.effect)?
        };
    notifications::wrap(base, &config.notifications, config.effect)
}

fn run_keyboard(
    sdk_path: Option<PathBuf>,
    options: &RunOptions,
    interrupted: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    run_keyboard_with_close_policy(sdk_path, options, true, interrupted)
}

fn run_keyboard_with_close_policy(
    sdk_path: Option<PathBuf>,
    options: &RunOptions,
    warn_on_close_error: bool,
    interrupted: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut signal = signals::StaticEffectSignal::new(options.effect);
    run_session(
        sdk_path.as_deref(),
        &SignalRunOptions::from(options),
        &mut signal,
        warn_on_close_error,
        interrupted,
    )
}

fn run_keyboard_signal(
    sdk_path: Option<PathBuf>,
    options: &SignalRunOptions,
    signal: &mut dyn signals::SignalProgram,
    warn_on_close_error: bool,
    interrupted: &AtomicBool,
) -> Result<(), Box<dyn std::error::Error>> {
    run_session(
        sdk_path.as_deref(),
        options,
        signal,
        warn_on_close_error,
        interrupted,
    )
}

fn close_best_effort(keyboard: &mut WootingRgb, warn: bool) {
    if let Err(error) = keyboard.close()
        && warn
    {
        eprintln!(
            "warning: {error}; SDK close/reset was not acknowledged, but the handle was closed"
        );
    }
}

fn print_config(config: &AppConfig) {
    let signal = config.signal_config();
    println!("config:");
    println!("  schema_version: {}", config.schema_version);
    println!("  sdk_path: {}", path_display(config.sdk_path.as_ref()));
    println!("  signal: {:?}", signal.kind);
    println!("  effect: {}", config.effect);
    println!("  palette: {}", config.palette);
    println!("  brightness: {}", config.brightness);
    println!("  fps: {}", config.fps);
    println!("  seconds: {:?}", config.seconds);
    println!("  continuous: {}", config.continuous);
    println!("  warn_on_close_error: {}", config.warn_on_close_error);
    println!(
        "  runtime: {}",
        if ProfileRuntimeSignal::is_profile_runtime_config(config) {
            "profile-v2"
        } else {
            "single-signal"
        }
    );
    if !config.sources.is_empty() {
        println!("  sources: {}", config.sources.len());
        for source in &config.sources {
            println!("    {}: {:?}", source.id, source.kind);
        }
    }
    if !config.rules.is_empty() {
        println!("  rules: {}", config.rules.len());
        for rule in &config.rules {
            println!("    priority {} -> {}", rule.priority, rule.scene);
        }
    }
    if !config.scenes.is_empty() {
        println!("  scenes: {}", config.scenes.len());
        for name in config.scenes.keys() {
            println!("    {name}");
        }
    }
    if !config.sources.is_empty() && !config.rules.is_empty() {
        println!("  selected_scenes:");
        for source in &config.sources {
            for status in [
                "running",
                "success",
                "failure",
                "timeout",
                "interrupted",
                "focus",
                "break",
                "overtime",
                "paused",
                "meeting-safe",
                "alert",
                "positive",
                "negative",
                "stale",
                "manual",
                "ide",
                "terminal",
                "meeting",
                "recording",
            ] {
                if let Some(selected) = config.select_scene(&source.id, status) {
                    println!(
                        "    {}.{status}: {} (priority {}, effect {:?})",
                        source.id, selected.name, selected.rule.priority, selected.scene.effect
                    );
                }
            }
        }
    }
    match signal.kind {
        SignalKind::Ripples => {
            println!(
                "  analog_sdk_path: {}",
                path_display(signal.ripples.analog_sdk_path.as_ref())
            );
        }
        SignalKind::CommandPulse => {
            println!("  command: {:?}", signal.command_pulse.command);
            println!("  cwd: {}", path_display(signal.command_pulse.cwd.as_ref()));
            println!("  env_overrides: {}", signal.command_pulse.env.len());
            println!("  output: {:?}", signal.command_pulse.output);
            println!("  summary: {}", signal.command_pulse.summary);
            println!(
                "  timeout_seconds: {}",
                signal.command_pulse.timeout_seconds
            );
            println!(
                "  hold_seconds: success={}, failure={}, interrupted={}",
                signal.command_pulse.success_hold_seconds,
                signal.command_pulse.failure_hold_seconds,
                signal.command_pulse.interrupted_hold_seconds
            );
        }
        SignalKind::GitHubCi => {
            println!("  repo: {}", signal.github_ci.repo);
            println!("  branch: {:?}", signal.github_ci.branch);
            println!("  pull_request: {:?}", signal.github_ci.pull_request);
            println!("  token_env: {}", signal.github_ci.token_env);
            println!("  api_base: {}", signal.github_ci.api_base);
            println!("  poll_seconds: {}", signal.github_ci.poll_seconds);
            println!("  stale_seconds: {}", signal.github_ci.stale_seconds);
        }
        SignalKind::FocusCockpit => {
            println!("  focus_minutes: {}", signal.focus.focus_minutes);
            println!("  break_minutes: {}", signal.focus.break_minutes);
            println!("  cycles: {}", signal.focus.cycles);
            println!("  start_paused: {}", signal.focus.start_paused);
            println!("  meeting_safe: {}", signal.focus.meeting_safe);
            println!("  dim: {}", signal.focus.dim);
        }
        SignalKind::MarketPulse => {
            println!("  api_url: {}", redacted_url(&signal.market.api_url));
            println!("  token_env: {}", signal.market.token_env);
            println!("  watchlist: {:?}", signal.market.watchlist);
            println!("  threshold_percent: {}", signal.market.threshold_percent);
            println!("  poll_seconds: {}", signal.market.poll_seconds);
            println!("  stale_seconds: {}", signal.market.stale_seconds);
        }
        SignalKind::SportsAlerts => {
            println!("  api_url: {}", redacted_url(&signal.sports.api_url));
            println!("  token_env: {}", signal.sports.token_env);
            println!("  favorites: {:?}", signal.sports.favorites);
            println!("  poll_seconds: {}", signal.sports.poll_seconds);
            println!("  stale_seconds: {}", signal.sports.stale_seconds);
        }
        SignalKind::AppAura => {
            println!("  profile: {:?}", signal.app_aura.profile);
            println!(
                "  accessibility_enabled: {}",
                signal.app_aura.accessibility_enabled
            );
            println!("  dim: {}", signal.app_aura.dim);
        }
        SignalKind::Soundwave => {
            println!("  enabled: {}", signal.soundwave.enabled);
            println!("  level: {}", signal.soundwave.level);
            println!("  bass: {}", signal.soundwave.bass);
            println!(
                "  cpu_limit_percent: {}",
                signal.soundwave.cpu_limit_percent
            );
        }
        SignalKind::FixtureReplay => {
            println!("  fixture_steps: {}", signal.fixture.steps.len());
            println!("  loop_steps: {}", signal.fixture.loop_steps);
        }
        SignalKind::StaticEffect => {}
    }
}

fn parse_app_aura_profile(value: &str) -> Result<signals::app_aura::AppAuraProfile, String> {
    match value {
        "manual" => Ok(signals::app_aura::AppAuraProfile::Manual),
        "ide" => Ok(signals::app_aura::AppAuraProfile::Ide),
        "terminal" => Ok(signals::app_aura::AppAuraProfile::Terminal),
        "meeting" => Ok(signals::app_aura::AppAuraProfile::Meeting),
        "game" => Ok(signals::app_aura::AppAuraProfile::Game),
        "recording" => Ok(signals::app_aura::AppAuraProfile::Recording),
        "late-night" => Ok(signals::app_aura::AppAuraProfile::LateNight),
        _ => Err(format!(
            "unknown app-aura profile {value:?}; expected manual, ide, terminal, meeting, game, recording, or late-night"
        )),
    }
}

fn parse_env_vars(values: Vec<String>) -> Result<BTreeMap<String, String>, String> {
    let mut env = BTreeMap::new();
    for value in values {
        let (key, var_value) = value
            .split_once('=')
            .ok_or_else(|| format!("--env must use KEY=VALUE, got {value:?}"))?;
        if key.is_empty() {
            return Err(format!("--env key cannot be empty in {value:?}"));
        }
        env.insert(key.to_string(), var_value.to_string());
    }
    Ok(env)
}

fn redacted_url(url: &str) -> String {
    if url.is_empty() {
        return "<unset>".to_string();
    }
    let Some((base, _query)) = url.split_once('?') else {
        return url.to_string();
    };
    format!("{base}?<redacted>")
}

fn path_display(path: Option<&PathBuf>) -> String {
    path.map(|path| path.display().to_string())
        .unwrap_or_else(|| "<auto>".to_string())
}

fn print_info(info: &DeviceInfo) {
    println!("model: {}", info.model);
    println!("connected: {}", info.connected);
    println!(
        "matrix: {} rows x {} columns",
        info.max_rows, info.max_columns
    );
    println!("led_index_max: {}", info.led_index_max);
    println!("device_type: {:?}", info.device_type);
    println!("layout: {:?}", info.layout);
    println!("v2_interface: {}", info.v2_interface);
    println!("uses_small_packets: {}", info.uses_small_packets);
    println!("uses_multi_report: {}", info.uses_multi_report);
}

fn install_ctrlc_handler() -> Result<Arc<AtomicBool>, ctrlc::Error> {
    let interrupted = Arc::new(AtomicBool::new(false));
    let handler_flag = Arc::clone(&interrupted);
    ctrlc::set_handler(move || {
        handler_flag.store(true, Ordering::SeqCst);
    })?;
    Ok(interrupted)
}
