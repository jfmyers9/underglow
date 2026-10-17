//! Explicit one-time development starter; never a recurring runtime mode.
use crate::config::AppConfig;
use std::collections::BTreeSet;

type Error = Box<dyn std::error::Error>;

pub(crate) fn config(original: &str) -> Result<String, Error> {
    let before: AppConfig = toml::from_str(original)?;
    // A canonical document is authoritative even if Pi (or every layer) was deleted.
    if before.setup.is_some() {
        return Ok(original.into());
    }
    let migrated = crate::migration::migrate(original)?;
    if migrated.warning.is_some()
        || before
            .automation
            .rules
            .iter()
            .any(|rule| rule.source == "pi")
    {
        return Ok(migrated.config);
    }
    let canonical: AppConfig = toml::from_str(&migrated.config)?;
    if canonical
        .setup
        .as_ref()
        .is_some_and(|setup| setup.layers.len() + 3 > underglow::setup::MAX_LAYERS)
    {
        // Optional starter layers must never displace a user's existing rules.
        return Ok(migrated.config);
    }
    // Add the explicit template BEFORE migration: priority ties and custom rules
    // must retain the same arbitration as the old one-time starter.
    let mut value: toml::Value = toml::from_str(original)?;
    let automation = value
        .as_table_mut()
        .ok_or("configuration must be a table")?
        .entry("automation")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or("automation must be a table")?;
    let ids: BTreeSet<_> = before
        .automation
        .rules
        .iter()
        .map(|rule| rule.id.as_str())
        .collect();
    let prefix = (0..=128)
        .map(|n| format!("dev-pi-{n}"))
        .find(|prefix| {
            ["present", "working", "finished"]
                .iter()
                .all(|suffix| !ids.contains(format!("{prefix}-{suffix}").as_str()))
                && ["idle", "working", "finished"].iter().all(|suffix| {
                    !before
                        .automation
                        .scenes
                        .contains_key(&format!("{prefix}-{suffix}"))
                })
        })
        .ok_or("no unused Pi starter namespace")?;
    let starter: toml::Value = toml::from_str(include_str!("../examples/matrix-pi.toml"))?;
    let defaults = &starter["automation"];
    let rules = automation
        .entry("rules")
        .or_insert_with(|| toml::Value::Array(vec![]))
        .as_array_mut()
        .ok_or("automation.rules must be an array")?;
    for rule in defaults["rules"]
        .as_array()
        .ok_or("missing bundled Pi rules")?
    {
        let mut rule = rule.clone();
        for field in ["id", "scene"] {
            let suffix = rule[field]
                .as_str()
                .and_then(|name| name.strip_prefix("pi-"))
                .ok_or("invalid bundled Pi rule")?
                .to_owned();
            rule.as_table_mut()
                .ok_or("invalid bundled Pi rule")?
                .insert(
                    field.into(),
                    toml::Value::String(format!("{prefix}-{suffix}")),
                );
        }
        rules.push(rule);
    }
    let scenes = automation
        .entry("scenes")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .ok_or("automation.scenes must be a table")?;
    for (name, scene) in defaults["scenes"]
        .as_table()
        .ok_or("missing bundled Pi scenes")?
    {
        let suffix = name.strip_prefix("pi-").ok_or("invalid bundled Pi scene")?;
        scenes.insert(format!("{prefix}-{suffix}"), scene.clone());
    }
    Ok(crate::migration::migrate(&toml::to_string(&value)?)?.config)
}

#[cfg(test)]
mod tests {
    use super::*;
    use underglow::setup::Renderer;

    #[test]
    fn starter_preserves_settings_and_deletion_is_authoritative() {
        let original = "effect='breath'\nbrightness=73\nspeed=65\nfps=45\n";
        let merged = config(original).unwrap();
        let parsed: AppConfig = toml::from_str(&merged).unwrap();
        assert_eq!((parsed.brightness, parsed.speed, parsed.fps), (73, 65, 45));
        assert_eq!(parsed.setup.as_ref().unwrap().layers.len(), 4);
        assert_eq!(config(&merged).unwrap(), merged);
        let mut value: toml::Value = toml::from_str(&merged).unwrap();
        value["setup"]["layers"].as_array_mut().unwrap().truncate(1);
        let deleted = toml::to_string(&value).unwrap();
        assert_eq!(config(&deleted).unwrap(), deleted);
        assert_eq!(crate::migration::migrate(&deleted).unwrap().config, deleted);
    }

    #[test]
    fn compatibility_profiles_and_custom_pi_rules_are_not_replaced() {
        let legacy = "[signal]\nkind='command-pulse'\ncommand=['never-run']\n";
        assert_eq!(config(legacy).unwrap(), legacy);
        let custom = "effect='matrix'\n[[automation.rules]]\nid='mine'\nsource='pi'\nmode='while-state'\nscene='mine'\n[automation.scenes.mine]\ncolor=[1,2,3]\n";
        let merged = config(custom).unwrap();
        let parsed: AppConfig = toml::from_str(&merged).unwrap();
        let layers = &parsed.setup.unwrap().layers;
        assert_eq!(layers.len(), 2);
        assert_eq!(layers[1].name, "mine");
        assert_eq!(layers[1].renderer, Renderer::Solid { color: [1, 2, 3] });
    }
}
