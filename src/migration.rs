//! Pure, all-or-nothing legacy snapshot migration. Never edits source files.
use crate::config::AppConfig;
use crate::signals::SignalKind;
use underglow::automation::{RuleMode, SceneKind};
use underglow::catalog::{PaletteControl, RendererKind};
use underglow::setup::{Activation, Layer, Renderer, SetupDocument, Target, TargetZone};

type Error = Box<dyn std::error::Error>;

pub(crate) struct Outcome {
    pub config: String,
    pub warning: Option<String>,
}

/// Canonical presence is the migration marker, including intentionally empty stacks.
/// Unsupported profiles remain byte-for-byte in legacy compatibility mode.
pub(crate) fn migrate(original: &str) -> Result<Outcome, Error> {
    let config: AppConfig = toml::from_str(original)?;
    config.validate()?;
    if config.setup.is_some() {
        return Ok(Outcome {
            config: original.into(),
            warning: None,
        });
    }
    let document = match document(&config) {
        Ok(document) => document,
        Err(reason) => {
            return Ok(Outcome {
                config: original.into(),
                warning: Some(format!(
                    "legacy compatibility mode: {reason}; original configuration retained"
                )),
            });
        }
    };
    let mut value: toml::Value = toml::from_str(original)?;
    value
        .as_table_mut()
        .ok_or("configuration must be a table")?
        .insert("setup".into(), toml::Value::try_from(&document)?);
    // Keep the source declarations (including dormant fields) intact. Only the
    // migrated automation is removed, preventing two runtimes painting its rules.
    if let Some(automation) = value
        .get_mut("automation")
        .and_then(toml::Value::as_table_mut)
    {
        automation.remove("rules");
    }
    Ok(Outcome {
        config: toml::to_string(&value)?,
        warning: None,
    })
}

fn effect(
    effect: underglow::effects::EffectKind,
    palette: underglow::render::PaletteName,
    speed: u32,
) -> Result<Renderer, String> {
    let entry = underglow::catalog::all()
        .iter()
        .find(|entry| entry.renderer == RendererKind::Static(effect))
        .ok_or("effect has no canonical renderer")?;
    Ok(Renderer::Effect {
        effect,
        palette: (entry.palette != PaletteControl::Fixed).then_some(palette),
        speed: if entry.supports_speed { speed } else { 100 },
    })
}

fn document(config: &AppConfig) -> Result<SetupDocument, String> {
    if !config.notifications.is_empty() {
        return Err("notification providers require the legacy runtime".into());
    }
    if crate::profile::ProfileRuntimeSignal::is_profile_runtime_config(config) {
        return Err("multi-source status/scene profiles require the legacy runtime".into());
    }
    let signal = config.signal_config();
    let renderer = match signal.kind {
        SignalKind::StaticEffect => effect(
            signal.effect.unwrap_or(config.effect),
            config.palette,
            config.speed,
        )?,
        SignalKind::FocusCockpit => Renderer::Focus {
            focus_minutes: signal.focus.focus_minutes,
            break_minutes: signal.focus.break_minutes,
            cycles: signal.focus.cycles,
            start_paused: signal.focus.start_paused,
            meeting_safe: signal.focus.meeting_safe,
            dim: signal.focus.dim,
        },
        SignalKind::Ripples => {
            if signal.ripples.analog_sdk_path.is_some() {
                return Err("custom analog SDK path requires the legacy runtime".into());
            }
            Renderer::Ripples {
                palette: signal
                    .ripples
                    .base_color
                    .is_none()
                    .then_some(config.palette),
                base_color: signal.ripples.base_color,
                ripple_color: signal.ripples.ripple_color,
            }
        }
        kind if kind.reactive_kind().is_some() => {
            if signal.reactive.analog_sdk_path.is_some() {
                return Err("custom analog SDK path requires the legacy runtime".into());
            }
            Renderer::Reactive {
                effect: kind.reactive_kind().unwrap(),
                palette: Some(config.palette),
            }
        }
        _ => return Err("selected provider requires the legacy runtime".into()),
    };
    let mut layers = vec![layer(
        "legacy-base".into(),
        "Base".into(),
        renderer,
        Target::Whole {},
        Activation::Always {},
    )?];
    let mut rules: Vec<_> = config.automation.rules.iter().enumerate().collect();
    // Legacy ties: first declaration wins. Canonical ties: top (later) wins.
    rules.sort_by(|(ai, a), (bi, b)| a.priority.cmp(&b.priority).then_with(|| bi.cmp(ai)));
    for (index, rule) in rules {
        let scene = &config.automation.scenes[&rule.scene];
        let renderer = match scene.kind {
            SceneKind::Solid => Renderer::Solid { color: scene.color },
            SceneKind::Effect => effect(
                scene.effect.unwrap(),
                scene.palette.unwrap_or(config.palette),
                100,
            )?,
            SceneKind::Pulse => Renderer::Pulse {
                color: scene.color,
                period_ms: 2000,
            },
            SceneKind::Sweep => {
                let info = underglow::device::DeviceInfo::synthetic_80he();
                let layout = underglow::layout::KeyboardLayout::for_device(&info);
                let mask = config.automation.scene_mask(scene)?;
                let count = layout
                    .keys()
                    .iter()
                    .filter(|key| mask.contains(key))
                    .count();
                Renderer::Sweep {
                    color: scene.color,
                    period_ms: count.max(1) as u64 * 100,
                }
            }
        };
        // Legacy scene caps brightness (min), canonical multiplies. Only migrate
        // when the two operations are exactly equivalent, including rounding.
        let brightness = match scene.brightness {
            Some(value) if config.brightness == 255 => value,
            Some(value) if value < config.brightness => {
                return Err("scene brightness cap requires the legacy runtime".into());
            }
            _ => 255,
        };
        let target = if scene.zones.is_empty() && scene.keys.is_empty() && scene.groups.is_empty() {
            Target::Whole {}
        } else {
            Target::Selection {
                zones: scene
                    .zones
                    .iter()
                    .map(|zone| match zone {
                        underglow::automation::SceneZone::Function => TargetZone::Function,
                        underglow::automation::SceneZone::Alpha => TargetZone::Alpha,
                        underglow::automation::SceneZone::Navigation => TargetZone::Navigation,
                        underglow::automation::SceneZone::Arrows => TargetZone::Arrows,
                        underglow::automation::SceneZone::System => TargetZone::System,
                    })
                    .collect(),
                keys: scene.keys.clone(),
                groups: scene.groups.clone(),
            }
        };
        let activation = match rule.mode {
            RuleMode::WhileState => Activation::WhileState {
                source: rule.source.clone(),
                instance: rule.instance.clone(),
                state: rule.state.clone(),
            },
            RuleMode::OnEvent => Activation::OnEvent {
                source: rule.source.clone(),
                instance: rule.instance.clone(),
                event: rule.event.clone().unwrap(),
                duration_ms: rule.duration_ms.unwrap(),
            },
            RuleMode::OnTransition => Activation::OnTransition {
                source: rule.source.clone(),
                instance: rule.instance.clone(),
                from_state: rule.from_state.clone(),
                state: rule.state.clone().unwrap(),
                duration_ms: rule.duration_ms.unwrap(),
            },
        };
        let mut migrated = layer(
            format!("legacy-rule-{index}"),
            rule.id.clone(),
            renderer,
            target,
            activation,
        )?;
        migrated.brightness = brightness;
        layers.push(migrated);
    }
    let document = SetupDocument {
        schema_version: 1,
        revision: 0,
        id: "legacy-setup".into(),
        name: "Migrated setup".into(),
        layers,
        groups: config.automation.groups.clone(),
    };
    document.validate()?;
    underglow::layer_runtime::CompiledSetup::compile(&document)?;
    Ok(document)
}

// Deserialize optional/default fields so compatible model additions keep working.
pub(crate) fn layer(
    id: String,
    name: String,
    renderer: Renderer,
    target: Target,
    activation: Activation,
) -> Result<Layer, String> {
    serde_json::from_value(serde_json::json!({
        "id": id, "name": name, "enabled": true, "brightness": 255,
        "renderer": renderer, "target": target, "activation": activation,
    }))
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use underglow::device::DeviceInfo;
    use underglow::external::{Identity, Snapshot, StateSnapshot};
    use underglow::layer_runtime::CompiledSetup;
    use underglow::layout::KeyboardLayout;
    use underglow::render::RenderContext;

    #[test]
    fn static_visuals_phase_palette_and_brightness_survive_restart() {
        for entry in underglow::catalog::all() {
            let RendererKind::Static(effect) = entry.renderer else {
                continue;
            };
            let original =
                format!("effect='{effect}'\npalette='ocean'\nbrightness=83\nspeed=65\nfps=45\n");
            let migrated = migrate(&original).unwrap();
            assert!(migrated.warning.is_none(), "{:?}", migrated.warning);
            assert_eq!(migrate(&migrated.config).unwrap().config, migrated.config);
            let config: AppConfig = toml::from_str(&migrated.config).unwrap();
            assert_eq!((config.brightness, config.speed, config.fps), (83, 65, 45));
            let runtime = CompiledSetup::compile(config.setup.as_ref().unwrap()).unwrap();
            let info = DeviceInfo::synthetic_80he();
            let layout = KeyboardLayout::for_device(&info);
            for millis in [0, 333, 1250, 7999] {
                let elapsed = Duration::from_millis(millis);
                let ctx = RenderContext {
                    info: &info,
                    layout: &layout,
                    brightness: 83,
                    palette: config.palette,
                    tick: 9,
                    animation_seconds: elapsed.as_secs_f64()
                        * if entry.supports_speed { 0.65 } else { 1.0 },
                };
                let legacy = effect.render(&ctx);
                let canonical = runtime.render(&ctx, elapsed);
                for key in layout.keys() {
                    assert_eq!(
                        legacy.get_coord(key.coord),
                        canonical.get_coord(key.coord),
                        "{effect} at {millis}"
                    );
                }
            }
        }
    }

    #[test]
    fn focus_and_input_settings_are_retained_or_explicitly_compatible() {
        for original in [
            "palette='heat'\n[signal]\nkind='focus-cockpit'\nfocus_minutes=13\nbreak_minutes=2\ncycles=3\nstart_paused=true\nmeeting_safe=true\ndim=true\n",
            "palette='ocean'\n[signal]\nkind='heatmap'\n",
            "[signal]\nkind='ripples'\n[signal.ripples]\nbase_color=[1,2,3]\nripple_color=[4,5,6]\n",
        ] {
            let migrated = migrate(original).unwrap();
            assert!(migrated.warning.is_none(), "{:?}", migrated.warning);
            let parsed: AppConfig = toml::from_str(&migrated.config).unwrap();
            let document = parsed.setup.unwrap();
            match &document.layers[0].renderer {
                Renderer::Focus {
                    focus_minutes,
                    break_minutes,
                    cycles,
                    start_paused,
                    meeting_safe,
                    dim,
                } => assert_eq!(
                    (
                        *focus_minutes,
                        *break_minutes,
                        *cycles,
                        *start_paused,
                        *meeting_safe,
                        *dim
                    ),
                    (13, 2, 3, true, true, true)
                ),
                Renderer::Reactive { effect, palette } => {
                    assert_eq!(*effect, underglow::reactive::ReactiveKind::Heatmap);
                    assert!(palette.is_some());
                }
                Renderer::Ripples {
                    base_color,
                    ripple_color,
                    palette,
                } => assert_eq!(
                    (*base_color, *ripple_color, *palette),
                    (Some([1, 2, 3]), Some([4, 5, 6]), None)
                ),
                other => panic!("unexpected {other:?}"),
            }
        }
    }

    #[test]
    fn groups_targets_conditions_and_first_declared_priority_ties_survive() {
        let original = "effect='matrix'\nbrightness=83\n\
            [[automation.rules]]\nid='first'\nsource='pi'\nmode='while-state'\nstate='working'\nscene='first'\npriority=-7\n\
            [[automation.rules]]\nid='second'\nsource='pi'\nmode='while-state'\nscene='second'\npriority=-7\n\
            [automation.scenes.first]\ncolor=[1,2,255]\ngroups=['custom']\n\
            [automation.scenes.second]\ncolor=[255,0,0]\nkeys=['F1']\n\
            [automation.groups]\ncustom=['F1']\n";
        let before: AppConfig = toml::from_str(original).unwrap();
        let migrated = migrate(original).unwrap();
        assert!(migrated.warning.is_none(), "{:?}", migrated.warning);
        let after: AppConfig = toml::from_str(&migrated.config).unwrap();
        assert!(after.automation.rules.is_empty());
        let doc = after.setup.unwrap();
        assert_eq!(doc.layers[1].name, "second");
        assert_eq!(doc.layers[2].name, "first");
        let snapshot = Snapshot {
            revision: 1,
            event_cursor: 0,
            states: vec![StateSnapshot {
                identity: Identity {
                    source: "pi".into(),
                    instance: "session".into(),
                },
                sequence: 1,
                state: "working".into(),
                updated_at: Duration::ZERO,
                expires_at: Duration::from_secs(60),
                revision: 1,
            }],
        };
        let info = DeviceInfo::synthetic_80he();
        let layout = KeyboardLayout::for_device(&info);
        let ctx = RenderContext {
            info: &info,
            layout: &layout,
            brightness: 83,
            palette: before.palette,
            tick: 0,
            animation_seconds: 1.5,
        };
        for _restart in 0..2 {
            let mut legacy =
                underglow::automation::AutomationRuntime::new(before.automation.clone()).unwrap();
            legacy.baseline(&snapshot, Duration::ZERO);
            let expected = underglow::compositor::compose(
                before.effect.render(&ctx),
                &layout,
                &legacy.layers(&ctx),
                Duration::ZERO,
            );
            let mut canonical = CompiledSetup::compile(&doc).unwrap();
            canonical.baseline(&snapshot, Duration::ZERO);
            assert_eq!(
                canonical.render(&ctx, Duration::from_millis(1500)),
                expected
            );
            let diagnostics = canonical.diagnostics(&ctx);
            let first = diagnostics
                .iter()
                .find(|layer| layer.rule_id == "legacy-rule-0")
                .unwrap();
            let second = diagnostics
                .iter()
                .find(|layer| layer.rule_id == "legacy-rule-1")
                .unwrap();
            assert!(first.active && first.matched);
            assert_eq!((first.targeted_keys, first.visible_keys), (1, 1));
            assert_eq!((second.targeted_keys, second.visible_keys), (1, 0));
            canonical.baseline(
                &Snapshot {
                    revision: 2,
                    event_cursor: 0,
                    states: vec![],
                },
                Duration::from_secs(2),
            );
            let diagnostics = canonical.diagnostics(&ctx);
            let first = diagnostics
                .iter()
                .find(|layer| layer.rule_id == "legacy-rule-0")
                .unwrap();
            assert!(!first.active && !first.matched);
            assert_eq!((first.targeted_keys, first.visible_keys), (1, 0));
        }
    }

    #[test]
    fn unsupported_behavior_is_never_partially_migrated() {
        for original in [
            "# retain executable trust\n[signal]\nkind='command-pulse'\ncommand=['never-run']\n",
            "[signal]\nkind='ripples'\n[signal.ripples]\nanalog_sdk_path='/private/custom-sdk'\n",
            "[signal]\nkind='heatmap'\n[signal.reactive]\nanalog_sdk_path='/private/custom-sdk'\n",
            "[[notifications]]\nid='notify'\nduration_seconds=2\nzones=['function']\n[notifications.signal]\nkind='focus-cockpit'\n",
            "brightness=83\n[[automation.rules]]\nid='low'\nsource='pi'\nmode='while-state'\nscene='low'\n[automation.scenes.low]\nbrightness=32\n",
            "[[sources]]\nid='status'\ntype='command-pulse'\n[scenes.base]\neffect='matrix'\n",
        ] {
            let outcome = migrate(original).unwrap();
            assert_eq!(outcome.config, original);
            assert!(
                outcome
                    .warning
                    .as_deref()
                    .unwrap()
                    .contains("compatibility mode")
            );
            assert_eq!(migrate(&outcome.config).unwrap().config, original);
        }
    }

    #[test]
    fn legacy_pulse_sweep_and_conditional_effect_phases_match_after_activation() {
        for scene in [
            "kind='pulse'\ncolor=[0,100,255]\nzones=['function']\n",
            "kind='sweep'\ncolor=[0,100,255]\nzones=['function']\n",
            "kind='effect'\neffect='breath'\npalette='heat'\nzones=['function']\n",
        ] {
            let original = format!(
                "effect='matrix'\nbrightness=180\nspeed=65\n\
                [[automation.rules]]\nid='test'\nsource='pi'\nmode='while-state'\nstate='working'\nscene='test'\n\
                [automation.scenes.test]\n{scene}"
            );
            let before: AppConfig = toml::from_str(&original).unwrap();
            let migrated = migrate(&original).unwrap();
            assert!(migrated.warning.is_none(), "{:?}", migrated.warning);
            let after: AppConfig = toml::from_str(&migrated.config).unwrap();
            let mut canonical = CompiledSetup::compile(after.setup.as_ref().unwrap()).unwrap();
            let mut legacy =
                underglow::automation::AutomationRuntime::new(before.automation.clone()).unwrap();
            let mut snapshot = Snapshot {
                revision: 0,
                event_cursor: 0,
                states: vec![],
            };
            let events = underglow::external::EventBatch {
                cursor: 0,
                gap: false,
                events: vec![],
            };
            legacy.baseline(&snapshot, Duration::ZERO);
            canonical.baseline(&snapshot, Duration::ZERO);
            snapshot.states.push(StateSnapshot {
                identity: Identity {
                    source: "pi".into(),
                    instance: "session".into(),
                },
                sequence: 1,
                state: "working".into(),
                updated_at: Duration::from_secs(5),
                expires_at: Duration::from_secs(60),
                revision: 1,
            });
            let info = DeviceInfo::synthetic_80he();
            let layout = KeyboardLayout::for_device(&info);
            for age in [0, 33, 100, 300, 700, 1234, 1999, 2000, 3000] {
                let now = Duration::from_millis(5000 + age);
                legacy.observe(&snapshot, &events, now);
                canonical.observe(&snapshot, &events, now);
                let ctx = RenderContext {
                    info: &info,
                    layout: &layout,
                    brightness: 180,
                    palette: before.palette,
                    tick: 9,
                    animation_seconds: now.as_secs_f64() * 0.65,
                };
                let expected = underglow::compositor::compose(
                    before.effect.render(&ctx),
                    &layout,
                    &legacy.layers(&ctx),
                    Duration::ZERO,
                );
                assert_eq!(canonical.render(&ctx, now), expected, "{scene} at {age}");
            }
        }
    }
}
