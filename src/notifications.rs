//! Bounded notifications composed into the existing single RGB frame stream.
use crate::config::SceneZone;
use crate::effects::EffectKind;
use crate::layout::Zone;
use crate::render::{Frame, RenderContext};
use crate::sdk::rgb::DeviceInfo;
use crate::signals::{
    ProgramResult, SignalConfig, SignalKind, SignalProgram, SignalSnapshot, build_signal,
};
use serde::Deserialize;
use std::collections::HashSet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};
use underglow::compositor::{KeyMask, Layer, compose};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NotificationConfig {
    pub id: String,
    #[serde(default)]
    pub priority: u8,
    pub duration_seconds: u64,
    pub zones: Vec<SceneZone>,
    /// Empty means any state change; otherwise exact provider status names.
    #[serde(default)]
    pub statuses: Vec<String>,
    pub signal: SignalConfig,
}

pub fn validate(configs: &[NotificationConfig]) -> Result<(), String> {
    if configs.len() > 16 {
        return Err("at most 16 notifications are supported".into());
    }
    let mut ids = HashSet::new();
    for config in configs {
        if config.id.trim().is_empty() || !ids.insert(&config.id) {
            return Err("notification ids must be nonempty and unique".into());
        }
        if !(1..=60).contains(&config.duration_seconds) {
            return Err("notification duration_seconds must be between 1 and 60".into());
        }
        if config.zones.is_empty() {
            return Err("notification zones must be explicitly nonempty".into());
        }
        if !matches!(
            config.signal.kind,
            SignalKind::CommandPulse | SignalKind::GitHubCi | SignalKind::FocusCockpit
        ) {
            return Err(
                "notifications support only command-pulse, github-ci, and focus-cockpit".into(),
            );
        }
    }
    Ok(())
}

struct Notification {
    config: NotificationConfig,
    signal: Box<dyn SignalProgram>,
    previous: Option<SignalSnapshot>,
    expires: Option<Duration>,
}

impl Notification {
    fn new(config: NotificationConfig, signal: Box<dyn SignalProgram>) -> Self {
        let previous = signal.notification_snapshot();
        Self {
            config,
            signal,
            previous,
            expires: None,
        }
    }

    fn observe(&mut self, now: Duration) {
        let snapshot = self.signal.notification_snapshot();
        // Progress/intensity are not event identity, even for custom providers.
        let changed = snapshot.as_ref().map(|s| (&s.status, &s.message))
            != self.previous.as_ref().map(|s| (&s.status, &s.message));
        if changed
            && let Some(snapshot) = &snapshot
            && (self.config.statuses.is_empty() || self.config.statuses.contains(&snapshot.status))
        {
            self.expires = Some(now + Duration::from_secs(self.config.duration_seconds));
        }
        self.previous = snapshot;
    }
}

pub fn wrap(
    base: Box<dyn SignalProgram>,
    configs: &[NotificationConfig],
    fallback: EffectKind,
) -> Result<Box<dyn SignalProgram>, Box<dyn std::error::Error>> {
    validate(configs)?;
    if configs.is_empty() {
        return Ok(base);
    }
    let notifications = configs
        .iter()
        .map(|config| {
            Ok(Notification::new(
                config.clone(),
                build_signal(&config.signal, fallback)?,
            ))
        })
        .collect::<Result<Vec<_>, Box<dyn std::error::Error>>>()?;
    Ok(Box::new(NotificationProgram {
        base,
        notifications,
        started: Instant::now(),
        now: Duration::ZERO,
    }))
}

struct NotificationProgram {
    base: Box<dyn SignalProgram>,
    notifications: Vec<Notification>,
    started: Instant,
    now: Duration,
}

impl NotificationProgram {
    fn tick_sources(&mut self, interrupted: &AtomicBool) -> ProgramResult {
        if interrupted.load(Ordering::SeqCst) {
            return Ok(());
        }
        self.base.tick(interrupted)?;
        for notification in &mut self.notifications {
            if interrupted.load(Ordering::SeqCst) {
                break;
            }
            notification.signal.tick(interrupted)?;
        }
        Ok(())
    }

    fn observe_at(&mut self, now: Duration) {
        self.now = now;
        for notification in &mut self.notifications {
            notification.observe(now);
        }
    }

    fn active_notifications(&self) -> impl Iterator<Item = &Notification> {
        self.notifications
            .iter()
            .filter(|n| n.expires.is_some_and(|expires| self.now < expires))
    }
}

impl SignalProgram for NotificationProgram {
    fn setup_runtime(&mut self) -> Option<&mut underglow::layer_runtime::CompiledSetup> {
        self.base.setup_runtime()
    }

    fn set_ripple_colors(&mut self, base: Option<[u8; 3]>, ripple: Option<[u8; 3]>) {
        self.base.set_ripple_colors(base, ripple);
    }

    fn initialize(&mut self) -> ProgramResult {
        self.started = Instant::now();
        self.base.initialize()?;
        for notification in &mut self.notifications {
            notification.signal.initialize()?;
        }
        Ok(())
    }

    fn validate_device(&self, info: &DeviceInfo) -> ProgramResult {
        self.base.validate_device(info)?;
        for notification in &self.notifications {
            notification.signal.validate_device(info)?;
        }
        Ok(())
    }

    fn tick(&mut self, interrupted: &AtomicBool) -> ProgramResult {
        self.tick_sources(interrupted)?;
        self.observe_at(self.started.elapsed());
        Ok(())
    }

    fn preview_tick(&mut self, tick: u32) {
        // Preview base only: no real provider calls or wall-clock notification events.
        self.base.preview_tick(tick);
    }

    fn render(&self, ctx: &RenderContext<'_>) -> Frame {
        let (base, layers) = self.composition(ctx);
        compose(base, ctx.layout, &layers, self.now)
    }

    fn automation_diagnostics(
        &self,
        ctx: &RenderContext<'_>,
    ) -> Vec<underglow::automation::RuleDiagnostic> {
        self.base.automation_diagnostics(ctx)
    }

    fn composition(&self, ctx: &RenderContext<'_>) -> (Frame, Vec<Layer>) {
        // Render the current background even when every key is covered. Source
        // ticking remains independent of visibility, including hidden deadlines.
        let (base, mut layers) = self.base.composition(ctx);
        layers.extend(self.active_notifications().map(|notification| {
            Layer {
                frame: notification.signal.render(ctx),
                mask: KeyMask::Zones(
                    notification
                        .config
                        .zones
                        .iter()
                        .map(|zone| match zone {
                            SceneZone::Function => Zone::Function,
                            SceneZone::Alpha => Zone::Alpha,
                            SceneZone::Navigation => Zone::Navigation,
                            SceneZone::Arrows => Zone::Arrows,
                            SceneZone::System => Zone::System,
                        })
                        .collect(),
                ),
                priority: i32::from(notification.config.priority),
                expires: None,
            }
        }));
        (base, layers)
    }

    fn finished(&self) -> bool {
        self.base.finished()
    }

    fn shutdown(&mut self, interrupted: bool) -> ProgramResult {
        // Never let one cleanup error skip another provider (notably a command child).
        let mut errors = Vec::new();
        if let Err(error) = self.base.shutdown(interrupted) {
            errors.push(error.to_string());
        }
        for notification in &mut self.notifications {
            notification.expires = None;
            if let Err(error) = notification.signal.shutdown(interrupted) {
                errors.push(format!("{}: {error}", notification.config.id));
            }
        }
        if errors.is_empty() {
            Ok(())
        } else {
            Err(errors.join("; ").into())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;
    use crate::layout::KeyboardLayout;
    use crate::render::{Color, PaletteName};
    use crate::sdk::rgb::{DeviceType, Layout};
    use std::cell::RefCell;
    use std::rc::Rc;

    #[derive(Default)]
    struct State {
        snapshot: Option<SignalSnapshot>,
        ticks: u8,
        stops: u8,
        fail_tick: bool,
        fail_stop: bool,
    }
    struct Mock(Rc<RefCell<State>>);
    impl SignalProgram for Mock {
        fn tick(&mut self, _: &AtomicBool) -> ProgramResult {
            let mut state = self.0.borrow_mut();
            state.ticks += 1;
            if state.fail_tick {
                Err("tick failed".into())
            } else {
                Ok(())
            }
        }
        fn notification_snapshot(&self) -> Option<SignalSnapshot> {
            self.0.borrow().snapshot.clone()
        }
        fn render(&self, ctx: &RenderContext<'_>) -> Frame {
            let mut frame = Frame::black();
            for key in ctx.layout.keys() {
                frame.set_coord(key.coord, Color::new(self.0.borrow().ticks, 0, 0));
            }
            frame
        }
        fn finished(&self) -> bool {
            false
        }
        fn shutdown(&mut self, _: bool) -> ProgramResult {
            let mut state = self.0.borrow_mut();
            state.stops += 1;
            if state.fail_stop {
                Err("stop failed".into())
            } else {
                Ok(())
            }
        }
    }
    fn config(id: &str, priority: u8, duration_seconds: u64) -> NotificationConfig {
        NotificationConfig {
            id: id.into(),
            priority,
            duration_seconds,
            zones: vec![SceneZone::Function],
            statuses: Vec::new(),
            signal: SignalConfig::focus_cockpit(Default::default()),
        }
    }
    fn mock() -> Rc<RefCell<State>> {
        Rc::new(RefCell::new(State {
            snapshot: Some(SignalSnapshot::status("mock", "idle")),
            ..Default::default()
        }))
    }
    fn event(state: &Rc<RefCell<State>>, status: &str) {
        state.borrow_mut().snapshot = Some(SignalSnapshot::status("mock", status));
    }
    fn program(
        base: &Rc<RefCell<State>>,
        entries: Vec<(NotificationConfig, Rc<RefCell<State>>)>,
    ) -> NotificationProgram {
        NotificationProgram {
            base: Box::new(Mock(base.clone())),
            notifications: entries
                .into_iter()
                .map(|(config, state)| Notification::new(config, Box::new(Mock(state))))
                .collect(),
            started: Instant::now(),
            now: Duration::ZERO,
        }
    }

    fn assert_zone_colors(p: &NotificationProgram, expected: impl Fn(Zone) -> u8) {
        let info = DeviceInfo {
            connected: true,
            model: "test".into(),
            max_rows: 6,
            max_columns: 17,
            led_index_max: 0,
            device_type: DeviceType::Keyboard80,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        };
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            animation_seconds: 0.0,
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        };
        let frame = p.render(&ctx);
        for key in layout.keys() {
            assert_eq!(
                frame.get_coord(key.coord),
                Color::new(expected(key.zone), 0, 0),
                "{:?} {:?}",
                key.zone,
                key.coord
            );
        }
    }

    #[test]
    fn legacy_and_external_layers_share_signed_priorities_ties_and_deadline_clocks() {
        use crate::ingress::SharedSources;
        use underglow::automation::AutomationConfig;
        use underglow::external::{Message, Payload};
        for priority in [-1, 5, 6] {
            let config: AutomationConfig = toml::from_str(&format!(
                r#"
[[rules]]
id='agent'
source='pi'
mode='while-state'
state='working'
scene='blue'
priority={priority}
[scenes.blue]
color=[0,0,255]
zones=['function']
"#
            ))
            .unwrap();
            let sources = SharedSources::default();
            sources
                .apply(Message {
                    schema_version: 1,
                    source: "pi".into(),
                    instance: "a".into(),
                    sequence: 1,
                    payload: Payload::State {
                        state: "working".into(),
                        ttl_ms: 15000,
                    },
                })
                .unwrap();
            let state = mock();
            let mut legacy = program(&mock(), vec![(self::config("legacy", 5, 2), state.clone())]);
            // Legacy clock intentionally far ahead of the external registry.
            state.borrow_mut().ticks = 90;
            event(&state, "success");
            legacy.observe_at(Duration::from_secs(100));
            let mut p =
                crate::external_rules::wrap(Box::new(legacy), &config, Some(sources)).unwrap();
            p.initialize().unwrap();
            let info = crate::preview::preview_device();
            let layout = KeyboardLayout::for_device(&info);
            let ctx = RenderContext {
                info: &info,
                layout: &layout,
                brightness: 96,
                palette: PaletteName::Wooting,
                tick: 0,
                animation_seconds: 0.0,
            };
            let function = layout
                .keys()
                .iter()
                .find(|key| key.zone == Zone::Function)
                .unwrap();
            let expected = if priority > 5 {
                Color::new(0, 0, 96)
            } else {
                Color::new(90, 0, 0)
            };
            assert_eq!(p.render(&ctx).get_coord(function.coord), expected);
            let diagnostic = p.automation_diagnostics(&ctx);
            assert_eq!(diagnostic[0].visible_keys > 0, priority > 5);
            assert!(
                p.composition(&ctx)
                    .1
                    .iter()
                    .all(|layer| layer.expires.is_none()),
                "each producer resolves its own clock before sharing layers"
            );
            p.shutdown(true).unwrap();
        }
    }

    #[test]
    fn edges_expire_without_replay_and_ignore_progress() {
        let state = mock();
        let mut p = program(&mock(), vec![(config("test", 0, 3), state.clone())]);
        p.observe_at(Duration::ZERO);
        assert!(p.active_notifications().next().is_none());
        event(&state, "success");
        p.observe_at(Duration::from_secs(1));
        assert!(p.active_notifications().next().is_some());
        state.borrow_mut().snapshot.as_mut().unwrap().progress = Some(0.9);
        p.observe_at(Duration::from_secs(4));
        assert!(p.active_notifications().next().is_none());
        p.observe_at(Duration::from_secs(100));
        assert!(p.active_notifications().next().is_none());
        event(&state, "failure");
        p.observe_at(Duration::from_secs(101));
        assert!(p.active_notifications().next().is_some());
    }

    #[test]
    fn new_event_identity_retriggers_same_status_but_filtered_changes_do_not() {
        let source = mock();
        let mut cfg = config("ci", 10, 2);
        cfg.statuses = vec!["passing".into()];
        let mut p = program(&mock(), vec![(cfg, source.clone())]);
        event(&source, "passing");
        source.borrow_mut().snapshot.as_mut().unwrap().message = "run-1".into();
        p.observe_at(Duration::ZERO);
        p.observe_at(Duration::from_secs(2));
        assert!(p.active_notifications().next().is_none());

        source.borrow_mut().snapshot.as_mut().unwrap().message = "run-2".into();
        p.observe_at(Duration::from_secs(3));
        assert!(p.active_notifications().next().is_some());
        event(&source, "running");
        p.observe_at(Duration::from_secs(4));
        // A nonmatching status does not refresh the previous alert's deadline.
        p.observe_at(Duration::from_secs(5));
        assert!(p.active_notifications().next().is_none());
        event(&source, "passing");
        p.observe_at(Duration::from_secs(6));
        assert!(p.active_notifications().next().is_some());
    }

    #[test]
    fn hidden_expired_events_are_not_queued_and_shutdown_clears_active_events() {
        let low = mock();
        let high = mock();
        low.borrow_mut().ticks = 10;
        high.borrow_mut().ticks = 90;
        let mut p = program(
            &mock(),
            vec![
                (config("low", 1, 2), low.clone()),
                (config("high", 9, 5), high.clone()),
            ],
        );
        event(&low, "success");
        event(&high, "success");
        p.observe_at(Duration::ZERO);
        assert_zone_colors(&p, |zone| if zone == Zone::Function { 90 } else { 0 });
        p.observe_at(Duration::from_secs(5));
        assert!(p.active_notifications().next().is_none());
        event(&high, "failure");
        p.observe_at(Duration::from_secs(6));
        assert!(p.active_notifications().next().is_some());
        p.shutdown(true).unwrap();
        assert!(p.active_notifications().next().is_none());
        assert_eq!(low.borrow().stops, 1);
        assert_eq!(high.borrow().stops, 1);
    }

    #[test]
    fn interruption_before_tick_does_not_start_sources_and_empty_wrap_is_passthrough() {
        let base = mock();
        let source = mock();
        let mut p = program(&base, vec![(config("test", 0, 2), source.clone())]);
        p.tick(&AtomicBool::new(true)).unwrap();
        assert_eq!(base.borrow().ticks, 0);
        assert_eq!(source.borrow().ticks, 0);

        let original_snapshot = base.borrow().snapshot.clone();
        let mut wrapped = wrap(Box::new(Mock(base.clone())), &[], EffectKind::default()).unwrap();
        // The default wrapper snapshot would be None; retaining this proves the
        // empty configuration returns the original program rather than wrapping it.
        assert_eq!(wrapped.notification_snapshot(), original_snapshot);
        wrapped.tick(&AtomicBool::new(false)).unwrap();
        assert_eq!(base.borrow().ticks, 1);
        wrapped.shutdown(true).unwrap();
        assert_eq!(base.borrow().stops, 1);
    }

    #[test]
    fn priority_ties_preemption_and_hidden_deadlines() {
        let low = mock();
        let high = mock();
        let tie = mock();
        low.borrow_mut().ticks = 10;
        high.borrow_mut().ticks = 90;
        tie.borrow_mut().ticks = 99;
        let mut p = program(
            &mock(),
            vec![
                (config("low", 1, 10), low.clone()),
                (config("high", 9, 3), high.clone()),
                (config("tie", 9, 3), tie.clone()),
            ],
        );
        for state in [&low, &high, &tie] {
            event(state, "success");
        }
        p.observe_at(Duration::ZERO);
        assert_zone_colors(&p, |zone| if zone == Zone::Function { 90 } else { 0 });
        p.observe_at(Duration::from_secs(3));
        assert_zone_colors(&p, |zone| if zone == Zone::Function { 10 } else { 0 });
        p.observe_at(Duration::from_secs(10));
        assert!(p.active_notifications().next().is_none());
        assert_zone_colors(&p, |_| 0);
    }

    #[test]
    fn simultaneous_zones_and_partial_overlap_have_independent_winners_and_expiry() {
        let base = mock();
        let function = mock();
        let navigation = mock();
        let high = mock();
        let tie = mock();
        function.borrow_mut().ticks = 10;
        navigation.borrow_mut().ticks = 20;
        high.borrow_mut().ticks = 90;
        tie.borrow_mut().ticks = 99;
        let mut navigation_config = config("navigation", 2, 10);
        navigation_config.zones = vec![SceneZone::Navigation];
        let mut high_config = config("high", 9, 2);
        high_config.zones = vec![SceneZone::Function, SceneZone::Navigation];
        let mut tie_config = config("tie", 9, 4);
        tie_config.zones = vec![SceneZone::Function, SceneZone::Arrows];
        let mut p = program(
            &base,
            vec![
                (config("function", 1, 10), function.clone()),
                (navigation_config, navigation.clone()),
                (high_config, high.clone()),
                (tie_config, tie.clone()),
            ],
        );
        event(&function, "success");
        event(&navigation, "failure");
        p.observe_at(Duration::ZERO);
        assert_zone_colors(&p, |zone| match zone {
            Zone::Function => 10,
            Zone::Navigation => 20,
            _ => 0,
        });
        event(&high, "success");
        event(&tie, "success");
        p.observe_at(Duration::from_secs(1));
        assert_zone_colors(&p, |zone| match zone {
            Zone::Function | Zone::Navigation => 90,
            Zone::Arrows => 99,
            _ => 0,
        });
        p.observe_at(Duration::from_secs(3));
        assert_zone_colors(&p, |zone| match zone {
            Zone::Function | Zone::Arrows => 99,
            Zone::Navigation => 20,
            _ => 0,
        });
        p.observe_at(Duration::from_secs(5));
        assert_zone_colors(&p, |zone| match zone {
            Zone::Function => 10,
            Zone::Navigation => 20,
            _ => 0,
        });
        p.tick_sources(&AtomicBool::new(false)).unwrap();
        p.observe_at(Duration::from_secs(10));
        assert_zone_colors(&p, |_| 1);
        p.observe_at(Duration::from_secs(100));
        assert_zone_colors(&p, |_| 1);
    }

    #[test]
    fn zones_restore_current_base_and_base_keeps_ticking() {
        let base = mock();
        let source = mock();
        source.borrow_mut().ticks = 50;
        let mut p = program(&base, vec![(config("test", 0, 2), source.clone())]);
        let info = DeviceInfo {
            connected: true,
            model: "test".into(),
            max_rows: 6,
            max_columns: 17,
            led_index_max: 0,
            device_type: DeviceType::Keyboard80,
            layout: Layout::Ansi,
            v2_interface: true,
            uses_small_packets: false,
            uses_multi_report: false,
        };
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            animation_seconds: 0.0,
            info: &info,
            layout: &layout,
            brightness: 96,
            palette: PaletteName::Wooting,
            tick: 0,
        };
        event(&source, "success");
        p.tick_sources(&AtomicBool::new(false)).unwrap();
        p.observe_at(Duration::ZERO);
        let frame = p.render(&ctx);
        for key in layout.keys() {
            assert_eq!(
                frame.get_coord(key.coord).red,
                if key.zone == Zone::Function { 51 } else { 1 }
            );
        }
        p.tick_sources(&AtomicBool::new(false)).unwrap();
        p.observe_at(Duration::from_secs(2));
        assert_eq!(p.render(&ctx), p.base.render(&ctx));
        assert_eq!(base.borrow().ticks, 2);
    }

    #[test]
    fn filters_preview_and_cleanup_even_after_errors() {
        let base = mock();
        let source = mock();
        let other = mock();
        let mut cfg = config("test", 0, 2);
        cfg.statuses = vec!["failure".into()];
        let mut p = program(
            &base,
            vec![
                (cfg, source.clone()),
                (config("other", 0, 2), other.clone()),
            ],
        );
        event(&source, "success");
        p.observe_at(Duration::ZERO);
        assert!(p.active_notifications().next().is_none());
        p.preview_tick(100);
        assert_eq!(source.borrow().ticks, 0);
        source.borrow_mut().fail_tick = true;
        assert!(p.tick(&AtomicBool::new(false)).is_err());
        base.borrow_mut().fail_stop = true;
        source.borrow_mut().fail_stop = true;
        assert!(p.shutdown(true).is_err());
        for state in [&base, &source, &other] {
            assert_eq!(state.borrow().stops, 1);
        }
        assert!(p.active_notifications().next().is_none());
    }

    #[test]
    fn config_validation_and_command_detection_are_offline() {
        let example: AppConfig =
            toml::from_str(include_str!("../examples/notifications.toml")).unwrap();
        example.validate().unwrap();
        assert!(!example.runs_commands());
        let mut cfg = config("test", 0, 1);
        for duration in [0, 61, u64::MAX] {
            cfg.duration_seconds = duration;
            assert!(validate(&[cfg.clone()]).is_err());
        }
        cfg.duration_seconds = 1;
        assert!(validate(&[cfg.clone(), cfg.clone()]).is_err());
        cfg.zones.clear();
        assert!(validate(&[cfg.clone()]).is_err());
        cfg.zones.push(SceneZone::Alpha);
        cfg.signal = SignalConfig::default();
        assert!(validate(&[cfg.clone()]).is_err());
        cfg.signal = SignalConfig::command_pulse(crate::signals::CommandPulseConfig {
            command: vec!["never-execute-this".into()],
            ..Default::default()
        });
        let app = AppConfig {
            notifications: vec![cfg],
            ..Default::default()
        };
        assert!(app.runs_commands());
        app.validate().unwrap();
        let base = Box::new(Mock(mock()));
        let mut wrapped = wrap(base, &app.notifications, app.effect).unwrap();
        wrapped.preview_tick(30); // Construction/preview must not execute the command.
    }
}
