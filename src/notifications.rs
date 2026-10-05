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

    fn winner(&self) -> Option<&Notification> {
        self.notifications
            .iter()
            .enumerate()
            .filter(|(_, n)| n.expires.is_some_and(|expires| self.now < expires))
            .max_by(|(ai, a), (bi, b)| {
                a.config
                    .priority
                    .cmp(&b.config.priority)
                    .then_with(|| bi.cmp(ai))
            })
            .map(|(_, n)| n)
    }
}

impl SignalProgram for NotificationProgram {
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
        let mut frame = self.base.render(ctx);
        if let Some(notification) = self.winner() {
            let overlay = notification.signal.render(ctx);
            for key in ctx.layout.keys() {
                if notification.config.zones.iter().any(|zone| match zone {
                    SceneZone::Function => key.zone == Zone::Function,
                    SceneZone::Alpha => key.zone == Zone::Alpha,
                    SceneZone::Navigation => key.zone == Zone::Navigation,
                    SceneZone::Arrows => key.zone == Zone::Arrows,
                    SceneZone::System => key.zone == Zone::System,
                }) {
                    frame.set_coord(key.coord, overlay.get_coord(key.coord));
                }
            }
        }
        frame
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

    #[test]
    fn edges_expire_without_replay_and_ignore_progress() {
        let state = mock();
        let mut p = program(&mock(), vec![(config("test", 0, 3), state.clone())]);
        p.observe_at(Duration::ZERO);
        assert!(p.winner().is_none());
        event(&state, "success");
        p.observe_at(Duration::from_secs(1));
        assert!(p.winner().is_some());
        state.borrow_mut().snapshot.as_mut().unwrap().progress = Some(0.9);
        p.observe_at(Duration::from_secs(4));
        assert!(p.winner().is_none());
        p.observe_at(Duration::from_secs(100));
        assert!(p.winner().is_none());
        event(&state, "failure");
        p.observe_at(Duration::from_secs(101));
        assert!(p.winner().is_some());
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
        assert!(p.winner().is_none());

        source.borrow_mut().snapshot.as_mut().unwrap().message = "run-2".into();
        p.observe_at(Duration::from_secs(3));
        assert!(p.winner().is_some());
        event(&source, "running");
        p.observe_at(Duration::from_secs(4));
        // A nonmatching status does not refresh the previous alert's deadline.
        p.observe_at(Duration::from_secs(5));
        assert!(p.winner().is_none());
        event(&source, "passing");
        p.observe_at(Duration::from_secs(6));
        assert!(p.winner().is_some());
    }

    #[test]
    fn hidden_expired_events_are_not_queued_and_shutdown_clears_active_events() {
        let low = mock();
        let high = mock();
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
        assert_eq!(p.winner().unwrap().config.id, "high");
        p.observe_at(Duration::from_secs(5));
        assert!(p.winner().is_none());
        event(&high, "failure");
        p.observe_at(Duration::from_secs(6));
        assert!(p.winner().is_some());
        p.shutdown(true).unwrap();
        assert!(p.winner().is_none());
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
        assert_eq!(p.winner().unwrap().config.id, "high");
        p.observe_at(Duration::from_secs(3));
        assert_eq!(p.winner().unwrap().config.id, "low");
        p.observe_at(Duration::from_secs(10));
        assert!(p.winner().is_none());
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
        assert!(p.winner().is_none());
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
        assert!(p.winner().is_none());
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
