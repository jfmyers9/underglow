use crate::effects::EffectKind;
use crate::render::PaletteName;
use crate::runner::{RunOptions, SignalRunOptions};
use crate::signals::{
    AppAuraConfig, CommandPulseConfig, FixtureConfig, FocusConfig, GitHubCiConfig, MarketConfig,
    SignalConfig, SoundwaveConfig, SportsConfig,
};
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct AppConfig {
    pub schema_version: u32,
    pub sdk_path: Option<PathBuf>,
    pub effect: EffectKind,
    pub palette: PaletteName,
    pub brightness: u8,
    pub fps: u32,
    pub speed: u32,
    pub seconds: Option<u64>,
    pub continuous: bool,
    pub warn_on_close_error: bool,
    #[serde(alias = "extension")]
    pub signal: Option<SignalConfig>,
    pub notifications: Vec<crate::notifications::NotificationConfig>,
    pub sources: Vec<SourceConfig>,
    pub rules: Vec<RuleConfig>,
    pub scenes: BTreeMap<String, SceneConfig>,
}

#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SourceConfig {
    pub id: String,
    #[serde(rename = "type")]
    pub kind: SourceKind,
    pub effect: Option<EffectKind>,
    pub ripples: crate::toys::RippleConfig,
    pub reactive: crate::signals::ReactiveConfig,
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

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SourceKind {
    #[default]
    StaticEffect,
    Ripples,
    Constellation,
    Heatmap,
    Afterimage,
    CommandPulse,
    #[serde(rename = "github-ci", alias = "git-hub-ci")]
    GithubCi,
    FocusCockpit,
    MarketPulse,
    SportsAlerts,
    AppAura,
    Soundwave,
    FixtureReplay,
    GithubActions,
    GithubPullRequests,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct RuleConfig {
    pub when: String,
    pub scene: String,
    pub priority: i32,
    pub hold_seconds: Option<u64>,
}

#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq)]
#[serde(default, deny_unknown_fields)]
pub struct SceneConfig {
    pub effect: Option<EffectKind>,
    pub palette: Option<PaletteName>,
    pub brightness: Option<u8>,
    pub zones: Vec<SceneZone>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq)]
#[serde(rename_all = "kebab-case")]
pub enum SceneZone {
    Function,
    Alpha,
    Navigation,
    Arrows,
    System,
}

#[derive(Clone, Copy, Debug)]
pub struct SelectedScene<'a> {
    pub name: &'a str,
    pub rule: &'a RuleConfig,
    pub scene: &'a SceneConfig,
}

impl SourceConfig {
    pub fn signal_config(&self, fallback_effect: EffectKind) -> Option<SignalConfig> {
        match self.kind {
            SourceKind::Constellation => Some(SignalConfig::reactive(
                underglow::reactive::ReactiveKind::Constellation,
                self.reactive.clone(),
            )),
            SourceKind::Heatmap => Some(SignalConfig::reactive(
                underglow::reactive::ReactiveKind::Heatmap,
                self.reactive.clone(),
            )),
            SourceKind::Afterimage => Some(SignalConfig::reactive(
                underglow::reactive::ReactiveKind::Afterimage,
                self.reactive.clone(),
            )),
            SourceKind::Ripples => Some(SignalConfig::ripples(self.ripples.clone())),
            SourceKind::StaticEffect => Some(SignalConfig::static_effect(
                self.effect.unwrap_or(fallback_effect),
            )),
            SourceKind::CommandPulse => {
                Some(SignalConfig::command_pulse(self.command_pulse.clone()))
            }
            SourceKind::GithubCi | SourceKind::GithubActions | SourceKind::GithubPullRequests => {
                Some(SignalConfig::github_ci(self.github_ci.clone()))
            }
            SourceKind::FocusCockpit => Some(SignalConfig::focus_cockpit(self.focus.clone())),
            SourceKind::MarketPulse => Some(SignalConfig::market_pulse(self.market.clone())),
            SourceKind::SportsAlerts => Some(SignalConfig::sports_alerts(self.sports.clone())),
            SourceKind::AppAura => Some(SignalConfig::app_aura(self.app_aura.clone())),
            SourceKind::Soundwave => Some(SignalConfig::soundwave(self.soundwave.clone())),
            SourceKind::FixtureReplay => Some(SignalConfig::fixture_replay(self.fixture.clone())),
        }
    }
}

impl RuleConfig {
    pub fn matches_status(&self, source_id: &str, status: &str) -> bool {
        let compact = self
            .when
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        compact == format!("{source_id}.status=='{status}'")
            || compact == format!("{source_id}.status==\"{status}\"")
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        let run = RunOptions::default();
        Self {
            schema_version: 1,
            sdk_path: None,
            effect: run.effect,
            palette: run.palette,
            brightness: run.brightness,
            fps: run.fps,
            speed: run.speed,
            seconds: run.seconds,
            continuous: false,
            warn_on_close_error: true,
            signal: None,
            notifications: Vec::new(),
            sources: Vec::new(),
            rules: Vec::new(),
            scenes: BTreeMap::new(),
        }
    }
}

impl AppConfig {
    pub fn load(path: &Path) -> Result<Self, ConfigError> {
        let content = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        let config: Self = toml::from_str(&content).map_err(ConfigError::Parse)?;
        config.validate()?;
        Ok(config)
    }

    pub fn validate(&self) -> Result<(), ConfigError> {
        crate::notifications::validate(&self.notifications).map_err(ConfigError::Invalid)?;
        if self.schema_version != 1 {
            return Err(ConfigError::Invalid(
                "unsupported schema_version; expected 1".into(),
            ));
        }
        if !(1..=120).contains(&self.fps) {
            return Err(ConfigError::Invalid("fps must be between 1 and 120".into()));
        }
        if !(10..=400).contains(&self.speed) {
            return Err(ConfigError::Invalid(
                "speed must be between 10 and 400 percent".into(),
            ));
        }
        // The analog SDK is process-global. Never initialize multiple instances
        // through a multi-source profile, even if a scene temporarily hides one.
        if self.signal.is_none()
            && self
                .sources
                .iter()
                .filter(|source| {
                    matches!(
                        source.kind,
                        SourceKind::Ripples
                            | SourceKind::Constellation
                            | SourceKind::Heatmap
                            | SourceKind::Afterimage
                    )
                })
                .count()
                > 1
        {
            return Err(ConfigError::Invalid(
                "at most one analog-input source (ripples, constellation, heatmap, afterimage) is supported".into(),
            ));
        }
        Ok(())
    }

    pub fn signal_run_options(&self) -> SignalRunOptions {
        SignalRunOptions {
            palette: self.palette,
            brightness: self.brightness,
            fps: self.fps,
            speed: self.speed,
            seconds: self.seconds,
            continuous: self.continuous,
        }
    }

    /// Only effective sources count: an explicit signal overrides legacy sources.
    pub fn runs_commands(&self) -> bool {
        use crate::signals::SignalKind;
        let base = if crate::profile::ProfileRuntimeSignal::is_profile_runtime_config(self) {
            self.sources
                .iter()
                .filter_map(|source| source.signal_config(self.effect))
                .any(|signal| signal.kind == SignalKind::CommandPulse)
        } else {
            self.signal_config().kind == SignalKind::CommandPulse
        };
        base || self
            .notifications
            .iter()
            .any(|notification| notification.signal.kind == SignalKind::CommandPulse)
    }

    pub fn signal_config(&self) -> SignalConfig {
        if let Some(signal) = &self.signal {
            return signal.clone();
        }

        self.sources
            .iter()
            .find_map(|source| source.signal_config(self.effect))
            .unwrap_or_else(|| SignalConfig::static_effect(self.effect))
    }

    pub fn select_scene(&self, source_id: &str, status: &str) -> Option<SelectedScene<'_>> {
        self.rules
            .iter()
            .enumerate()
            .filter(|(_, rule)| rule.matches_status(source_id, status))
            .filter_map(|(index, rule)| {
                self.scenes
                    .get_key_value(&rule.scene)
                    .map(|(name, scene)| (index, name, rule, scene))
            })
            .max_by(|left, right| {
                left.2
                    .priority
                    .cmp(&right.2.priority)
                    .then_with(|| right.0.cmp(&left.0))
            })
            .map(|(_, name, rule, scene)| SelectedScene {
                name: name.as_str(),
                rule,
                scene,
            })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("invalid config: {0}")]
    Invalid(String),
    #[error("failed to read config {path}: {source}")]
    Read {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("failed to parse config: {0}")]
    Parse(#[from] toml::de::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signals::SignalKind;

    #[test]
    fn command_detection_follows_effective_source_selection() {
        for (text, expected) in [
            ("", false),
            (
                "[signal]\nkind = 'command-pulse'\ncommand = ['never-run']",
                true,
            ),
            (
                "[[sources]]\ntype = 'static-effect'\n[[sources]]\ntype = 'command-pulse'",
                false,
            ),
            (
                "[[sources]]\ntype = 'static-effect'\n[[sources]]\ntype = 'command-pulse'\n[scenes.base]\neffect = 'comet'",
                true,
            ),
            (
                "[signal]\nkind = 'static-effect'\n[[sources]]\ntype = 'command-pulse'\n[scenes.base]\neffect = 'comet'",
                false,
            ),
        ] {
            let config: AppConfig = toml::from_str(text).unwrap();
            assert_eq!(config.runs_commands(), expected, "{text}");
        }
    }

    #[test]
    fn command_detection_uses_signal_kind_not_dormant_command_fields() {
        let notification = "\n[[notifications]]\nid = 'overlay'\nduration_seconds = 2\nzones = ['function']\n[notifications.signal]\n";
        for (text, expected) in [
            (
                "[signal]\nkind = 'static-effect'\ncommand = ['never-run']".to_string(),
                false,
            ),
            (
                format!(
                    "[signal]\nkind = 'static-effect'{notification}kind = 'command-pulse'\ncommand = ['never-run']"
                ),
                true,
            ),
            (
                format!(
                    "[signal]\nkind = 'static-effect'\n[[sources]]\ntype = 'command-pulse'{notification}kind = 'focus-cockpit'\ncommand = ['never-run']"
                ),
                false,
            ),
        ] {
            let config: AppConfig = toml::from_str(&text).unwrap();
            config.validate().unwrap();
            assert_eq!(config.runs_commands(), expected, "{text}");
        }
    }

    #[test]
    fn ripples_config_and_legacy_version_defaults() {
        let config: AppConfig = toml::from_str(include_str!("../examples/ripples.toml")).unwrap();
        config.validate().unwrap();
        assert_eq!(config.schema_version, 1);
        assert_eq!(config.signal_config().kind, SignalKind::Ripples);
        assert_eq!(config.brightness, 255);
        assert!(config.continuous);
        let legacy: AppConfig = toml::from_str("effect = 'comet'").unwrap();
        legacy.validate().unwrap();
        assert_eq!(legacy.schema_version, 1);
        assert_eq!(legacy.brightness, 255);
        assert_eq!(legacy.speed, 100);
        for brightness in [0, 42, 96, 180, 255] {
            let explicit: AppConfig =
                toml::from_str(&format!("brightness = {brightness}")).unwrap();
            assert_eq!(explicit.brightness, brightness);
        }
    }

    #[test]
    fn reject_unsupported_versions_rates_and_duplicate_analog_sources() {
        for text in [
            "schema_version = 2",
            "fps = 0",
            "fps = 121",
            "speed = 0",
            "speed = 9",
            "speed = 401",
            "[[sources]]\ntype = 'ripples'\n[[sources]]\ntype = 'ripples'",
        ] {
            let config: AppConfig = toml::from_str(text).unwrap();
            assert!(config.validate().is_err(), "{text}");
        }
        assert!(
            toml::from_str::<AppConfig>(
                "[signal]\nkind = 'ripples'\n[signal.ripples]\nunknown_setting = 1"
            )
            .is_err()
        );
    }

    #[test]
    fn reactive_example_profiles_are_valid_and_preview_safe() {
        for (name, text) in [
            (
                "constellation",
                include_str!("../examples/constellation.toml"),
            ),
            ("heatmap", include_str!("../examples/heatmap.toml")),
            ("afterimage", include_str!("../examples/afterimage.toml")),
        ] {
            let config: AppConfig = toml::from_str(text).unwrap();
            config.validate().unwrap();
            assert_eq!(
                config
                    .signal_config()
                    .kind
                    .reactive_kind()
                    .unwrap()
                    .to_string(),
                name
            );
            assert!(!config.runs_commands());
            assert_eq!(config.brightness, 255);
            assert!(config.signal_config().reactive.analog_sdk_path.is_none());
        }
    }

    #[test]
    fn reactive_sources_share_the_single_analog_owner_limit() {
        let modes = ["ripples", "constellation", "heatmap", "afterimage"];
        for first in modes {
            for second in modes {
                let text = format!("[[sources]]\ntype='{first}'\n[[sources]]\ntype='{second}'\n");
                let config: AppConfig = toml::from_str(&text).unwrap();
                assert!(
                    config
                        .validate()
                        .unwrap_err()
                        .to_string()
                        .contains("at most one analog-input")
                );
                // An explicit signal still overrides unused legacy source declarations.
                let config: AppConfig =
                    toml::from_str(&format!("{text}\n[signal]\nkind='static-effect'\n")).unwrap();
                config.validate().unwrap();
            }
        }
        for kind in ["constellation", "heatmap", "afterimage"] {
            let config: AppConfig = toml::from_str(&format!("[[sources]]\ntype='{kind}'\n[sources.reactive]\nanalog_sdk_path='/synthetic-analog'\n")).unwrap();
            config.validate().unwrap();
            let signal = config.signal_config();
            assert_eq!(signal.kind.reactive_kind().unwrap().to_string(), kind);
            assert_eq!(
                signal.reactive.analog_sdk_path,
                Some(PathBuf::from("/synthetic-analog"))
            );
            assert!(!config.runs_commands());
            assert!(
                toml::from_str::<AppConfig>(&format!(
                    "[signal]\nkind='{kind}'\n[signal.reactive]\nrecord_keys=true\n"
                ))
                .is_err()
            );
        }
    }

    #[test]
    fn config_defaults_to_static_effect_signal() {
        let config = AppConfig::default();

        assert_eq!(config.brightness, 255);
        assert_eq!(config.fps, 30);
        assert_eq!(config.seconds, Some(10));
        assert!(!config.continuous);
        assert!(config.warn_on_close_error);
        assert_eq!(config.signal_config().kind, SignalKind::StaticEffect);
        assert!(config.sources.is_empty());
        assert!(config.rules.is_empty());
        assert!(config.scenes.is_empty());
    }

    #[test]
    fn signal_run_options_reflect_config_values() {
        let config: AppConfig = toml::from_str(
            r#"
palette = "terminal"
brightness = 42
fps = 12
seconds = 0
continuous = true
"#,
        )
        .unwrap();

        let options = config.signal_run_options();
        assert_eq!(options.palette, PaletteName::Terminal);
        assert_eq!(options.brightness, 42);
        assert_eq!(options.fps, 12);
        assert_eq!(options.seconds, Some(0));
        assert!(options.continuous);
    }

    #[test]
    fn config_accepts_legacy_extension_alias() {
        let config: AppConfig = toml::from_str(
            r#"
[extension]
kind = "command-pulse"
command = ["true"]
"#,
        )
        .unwrap();

        let signal = config.signal_config();
        assert_eq!(signal.kind, SignalKind::CommandPulse);
        assert_eq!(signal.command_pulse.command, ["true"]);
    }

    #[test]
    fn explicit_signal_takes_precedence_over_profile_sources() {
        let config: AppConfig = toml::from_str(
            r#"
[signal]
kind = "static-effect"
effect = "matrix"

[[sources]]
id = "tests"
type = "command-pulse"
command = ["false"]
"#,
        )
        .unwrap();

        let signal = config.signal_config();
        assert_eq!(signal.kind, SignalKind::StaticEffect);
        assert_eq!(signal.effect, Some(EffectKind::Matrix));
    }

    #[test]
    fn profile_source_builds_command_pulse_signal_config() {
        let config: AppConfig = toml::from_str(
            r#"
[[sources]]
id = "tests"
type = "command-pulse"
command = ["cargo", "test"]
"#,
        )
        .unwrap();

        let signal = config.signal_config();
        assert_eq!(signal.kind, SignalKind::CommandPulse);
        assert_eq!(signal.command_pulse.command, ["cargo", "test"]);
    }

    #[test]
    fn profile_source_builds_static_effect_signal_config() {
        let config: AppConfig = toml::from_str(
            r#"
effect = "rainbow"

[[sources]]
id = "ambient"
type = "static-effect"
effect = "breath"
"#,
        )
        .unwrap();

        let signal = config.signal_config();
        assert_eq!(signal.kind, SignalKind::StaticEffect);
        assert_eq!(signal.effect, Some(EffectKind::Breath));
    }

    #[test]
    fn profile_source_builds_github_ci_signal_config() {
        let config: AppConfig = toml::from_str(
            r#"
[[sources]]
id = "ci"
type = "github-ci"
repo = "owner/repo"
branch = "main"
token_env = "GH_TOKEN"
"#,
        )
        .unwrap();

        let signal = config.signal_config();
        assert_eq!(signal.kind, SignalKind::GitHubCi);
        assert_eq!(signal.github_ci.repo, "owner/repo");
        assert_eq!(signal.github_ci.branch.as_deref(), Some("main"));
        assert_eq!(signal.github_ci.token_env, "GH_TOKEN");
    }

    #[test]
    fn profile_source_builds_focus_cockpit_signal_config() {
        let config: AppConfig = toml::from_str(
            r#"
[[sources]]
id = "focus"
type = "focus-cockpit"
focus_minutes = 45
break_minutes = 10
cycles = 2
meeting_safe = true
"#,
        )
        .unwrap();

        let signal = config.signal_config();
        assert_eq!(signal.kind, SignalKind::FocusCockpit);
        assert_eq!(signal.focus.focus_minutes, 45);
        assert_eq!(signal.focus.break_minutes, 10);
        assert_eq!(signal.focus.cycles, 2);
        assert!(signal.focus.meeting_safe);
    }

    #[test]
    fn profile_source_builds_fixture_replay_signal_config() {
        let config: AppConfig = toml::from_str(
            r#"
[[sources]]
id = "demo"
type = "fixture-replay"
loop_steps = false

[[sources.steps]]
status = "running"
message = "demo running"
hold_ticks = 1
"#,
        )
        .unwrap();

        let signal = config.signal_config();
        assert_eq!(signal.kind, SignalKind::FixtureReplay);
        assert_eq!(signal.fixture.steps.len(), 1);
        assert_eq!(signal.fixture.steps[0].status, "running");
        assert!(!signal.fixture.loop_steps);
    }

    #[test]
    fn config_rejects_unknown_top_level_fields() {
        let error = toml::from_str::<AppConfig>("unexpected = true").unwrap_err();

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn config_accepts_profile_v2_sections() {
        let config: AppConfig = toml::from_str(
            r#"
[[sources]]
id = "tests"
type = "command-pulse"
command = ["cargo", "test"]
timeout_seconds = 120

[[rules]]
when = "tests.status == 'failure'"
scene = "red-alert"
priority = 100
hold_seconds = 6

[scenes.red-alert]
effect = "breath"
palette = "heat"
brightness = 96
zones = ["function", "navigation"]
"#,
        )
        .unwrap();

        assert_eq!(config.sources.len(), 1);
        assert_eq!(config.sources[0].id, "tests");
        assert_eq!(config.sources[0].kind, SourceKind::CommandPulse);
        assert_eq!(config.sources[0].command_pulse.command, ["cargo", "test"]);
        assert_eq!(config.rules.len(), 1);
        assert_eq!(config.rules[0].priority, 100);
        assert_eq!(config.scenes.keys().collect::<Vec<_>>(), vec!["red-alert"]);
        assert_eq!(
            config.scenes,
            BTreeMap::from([(
                "red-alert".to_string(),
                SceneConfig {
                    effect: Some(EffectKind::Breath),
                    palette: Some(PaletteName::Heat),
                    brightness: Some(96),
                    zones: vec![SceneZone::Function, SceneZone::Navigation],
                },
            )])
        );
    }

    #[test]
    fn rule_matches_status_expressions() {
        let rule = RuleConfig {
            when: "tests.status == 'failure'".to_string(),
            scene: "red-alert".to_string(),
            ..RuleConfig::default()
        };

        assert!(rule.matches_status("tests", "failure"));
        assert!(!rule.matches_status("tests", "success"));
        assert!(!rule.matches_status("ci", "failure"));
    }

    #[test]
    fn scene_selection_uses_highest_priority_matching_rule() {
        let config: AppConfig = toml::from_str(
            r#"
[[rules]]
when = "ci.status == 'failure'"
scene = "ambient"
priority = 0

[[rules]]
when = "ci.status == 'failure'"
scene = "red-alert"
priority = 100

[scenes.ambient]
effect = "breath"
palette = "ocean"
zones = ["alpha"]

[scenes.red-alert]
effect = "breath"
palette = "heat"
zones = ["function", "navigation"]
"#,
        )
        .unwrap();

        let selected = config.select_scene("ci", "failure").unwrap();

        assert_eq!(selected.name, "red-alert");
        assert_eq!(selected.rule.priority, 100);
        assert_eq!(selected.scene.palette, Some(PaletteName::Heat));
    }

    #[test]
    fn scene_selection_uses_declaration_order_for_priority_ties() {
        let config: AppConfig = toml::from_str(
            r#"
[[rules]]
when = "ci.status == 'running'"
scene = "first"
priority = 10

[[rules]]
when = "ci.status == 'running'"
scene = "second"
priority = 10

[scenes.first]
effect = "comet"

[scenes.second]
effect = "matrix"
"#,
        )
        .unwrap();

        let selected = config.select_scene("ci", "running").unwrap();

        assert_eq!(selected.name, "first");
        assert_eq!(selected.scene.effect, Some(EffectKind::Comet));
    }
}
