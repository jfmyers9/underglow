pub mod app_aura;
pub mod command_pulse;
pub mod external;
pub mod fixture;
pub mod focus;
pub mod github;
pub mod market;
pub mod soundwave;
pub mod sports;
pub mod static_effect;

use crate::effects::EffectKind;
use crate::render::{Frame, RenderContext};
use crate::sdk::rgb::DeviceInfo;
use crate::toys::{RippleConfig, RippleSignal};
pub use app_aura::{AppAuraConfig, AppAuraSignal};
use clap::ValueEnum;
pub use command_pulse::{CommandPulseConfig, CommandPulseOutput, CommandPulseSignal};
pub use fixture::{FixtureConfig, FixtureSignal};
pub use focus::{FocusConfig, FocusSignal};
pub use github::{GitHubCiConfig, GitHubCiSignal};
pub use market::{MarketConfig, MarketSignal};
use serde::Deserialize;
pub use soundwave::{SoundwaveConfig, SoundwaveSignal};
pub use sports::{SportsConfig, SportsSignal};
pub use static_effect::StaticEffectSignal;
use std::sync::atomic::AtomicBool;

pub type ProgramResult = Result<(), Box<dyn std::error::Error>>;

pub trait SignalProgram {
    /// Acquire mode-specific resources only for a live run, never for previews.
    fn initialize(&mut self) -> ProgramResult {
        Ok(())
    }
    fn validate_device(&self, _info: &DeviceInfo) -> ProgramResult {
        Ok(())
    }
    fn tick(&mut self, interrupted: &AtomicBool) -> ProgramResult;
    /// Deterministic preview input; must not open hardware, poll APIs, or spawn commands.
    fn preview_tick(&mut self, _tick: u32) {}
    /// Stable notification identity. Exclude continuously changing progress/intensity.
    fn notification_snapshot(&self) -> Option<SignalSnapshot> {
        None
    }
    fn render(&self, ctx: &RenderContext<'_>) -> Frame;
    fn finished(&self) -> bool;
    fn shutdown(&mut self, interrupted: bool) -> ProgramResult;
}

#[derive(Clone, Debug, PartialEq)]
pub struct SignalSnapshot {
    pub source_id: String,
    pub status: String,
    pub message: String,
    pub progress: Option<f32>,
    pub intensity: Option<f32>,
}

impl SignalSnapshot {
    pub fn status(source_id: &str, status: impl Into<String>) -> Self {
        let status = status.into();
        Self {
            source_id: source_id.to_string(),
            message: status.clone(),
            status,
            progress: None,
            intensity: None,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[derive(Default)]
pub enum SignalKind {
    #[default]
    StaticEffect,
    Ripples,
    CommandPulse,
    #[serde(rename = "github-ci", alias = "git-hub-ci")]
    #[value(name = "github-ci", alias = "git-hub-ci")]
    GitHubCi,
    FocusCockpit,
    MarketPulse,
    SportsAlerts,
    AppAura,
    Soundwave,
    FixtureReplay,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(default)]
pub struct SignalConfig {
    pub kind: SignalKind,
    pub effect: Option<EffectKind>,
    pub ripples: RippleConfig,
    #[serde(flatten)]
    pub command_pulse: CommandPulseConfig,
    #[serde(flatten)]
    pub github_ci: GitHubCiConfig,
    #[serde(flatten)]
    pub focus: FocusConfig,
    #[serde(flatten)]
    pub market: MarketConfig,
    #[serde(flatten)]
    pub sports: SportsConfig,
    #[serde(flatten)]
    pub app_aura: AppAuraConfig,
    #[serde(flatten)]
    pub soundwave: SoundwaveConfig,
    #[serde(flatten)]
    pub fixture: FixtureConfig,
}

impl Default for SignalConfig {
    fn default() -> Self {
        Self {
            kind: SignalKind::StaticEffect,
            effect: Some(EffectKind::default()),
            ripples: RippleConfig::default(),
            command_pulse: CommandPulseConfig::default(),
            github_ci: GitHubCiConfig::default(),
            focus: FocusConfig::default(),
            market: MarketConfig::default(),
            sports: SportsConfig::default(),
            app_aura: AppAuraConfig::default(),
            soundwave: SoundwaveConfig::default(),
            fixture: FixtureConfig::default(),
        }
    }
}

impl SignalConfig {
    fn base(kind: SignalKind) -> Self {
        Self {
            kind,
            effect: None,
            ripples: RippleConfig::default(),
            command_pulse: CommandPulseConfig::default(),
            github_ci: GitHubCiConfig::default(),
            focus: FocusConfig::default(),
            market: MarketConfig::default(),
            sports: SportsConfig::default(),
            app_aura: AppAuraConfig::default(),
            soundwave: SoundwaveConfig::default(),
            fixture: FixtureConfig::default(),
        }
    }

    pub fn static_effect(effect: EffectKind) -> Self {
        Self {
            effect: Some(effect),
            ..Self::base(SignalKind::StaticEffect)
        }
    }

    pub fn ripples(ripples: RippleConfig) -> Self {
        Self {
            ripples,
            ..Self::base(SignalKind::Ripples)
        }
    }

    pub fn command_pulse(command_pulse: CommandPulseConfig) -> Self {
        Self {
            command_pulse,
            ..Self::base(SignalKind::CommandPulse)
        }
    }

    pub fn github_ci(github_ci: GitHubCiConfig) -> Self {
        Self {
            github_ci,
            ..Self::base(SignalKind::GitHubCi)
        }
    }

    pub fn focus_cockpit(focus: FocusConfig) -> Self {
        Self {
            focus,
            ..Self::base(SignalKind::FocusCockpit)
        }
    }

    pub fn market_pulse(market: MarketConfig) -> Self {
        Self {
            market,
            ..Self::base(SignalKind::MarketPulse)
        }
    }

    pub fn sports_alerts(sports: SportsConfig) -> Self {
        Self {
            sports,
            ..Self::base(SignalKind::SportsAlerts)
        }
    }

    pub fn app_aura(app_aura: AppAuraConfig) -> Self {
        Self {
            app_aura,
            ..Self::base(SignalKind::AppAura)
        }
    }

    pub fn soundwave(soundwave: SoundwaveConfig) -> Self {
        Self {
            soundwave,
            ..Self::base(SignalKind::Soundwave)
        }
    }

    pub fn fixture_replay(fixture: FixtureConfig) -> Self {
        Self {
            fixture,
            ..Self::base(SignalKind::FixtureReplay)
        }
    }
}

pub fn build_signal(
    config: &SignalConfig,
    fallback_effect: EffectKind,
) -> Result<Box<dyn SignalProgram>, Box<dyn std::error::Error>> {
    match config.kind {
        SignalKind::Ripples => Ok(Box::new(RippleSignal::new(config.ripples.clone()))),
        SignalKind::StaticEffect => Ok(Box::new(StaticEffectSignal::new(
            config.effect.unwrap_or(fallback_effect),
        ))),
        SignalKind::CommandPulse => Ok(Box::new(CommandPulseSignal::new(
            config.command_pulse.clone(),
        )?)),
        SignalKind::GitHubCi => Ok(Box::new(GitHubCiSignal::new(config.github_ci.clone())?)),
        SignalKind::FocusCockpit => Ok(Box::new(FocusSignal::new(config.focus.clone()))),
        SignalKind::MarketPulse => Ok(Box::new(MarketSignal::new(config.market.clone()))),
        SignalKind::SportsAlerts => Ok(Box::new(SportsSignal::new(config.sports.clone()))),
        SignalKind::AppAura => Ok(Box::new(AppAuraSignal::new(config.app_aura.clone()))),
        SignalKind::Soundwave => Ok(Box::new(SoundwaveSignal::new(config.soundwave.clone()))),
        SignalKind::FixtureReplay => Ok(Box::new(FixtureSignal::new(config.fixture.clone()))),
    }
}
