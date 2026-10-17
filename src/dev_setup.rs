//! Development-only Pi defaults, merged into the saved snapshot, never source files.
use std::collections::BTreeSet;

type Error = Box<dyn std::error::Error>;

pub(crate) fn config(original: &str) -> Result<String, Error> {
    let mut profile: toml::Value = toml::from_str(original)?;
    let root = profile.as_table_mut().ok_or("profile must be a table")?;
    let automation = root
        .entry("automation")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("automation must be a table")?;
    let existing_rules = automation.get("rules").and_then(toml::Value::as_array);
    // An explicit Pi setup belongs to the user, even if it uses different scenes.
    if existing_rules.is_some_and(|rules| {
        rules
            .iter()
            .any(|rule| rule.get("source").and_then(toml::Value::as_str) == Some("pi"))
    }) {
        return Ok(original.to_owned());
    }
    let ids: BTreeSet<_> = existing_rules
        .into_iter()
        .flatten()
        .filter_map(|rule| rule.get("id").and_then(toml::Value::as_str))
        .map(str::to_owned)
        .collect();
    let scene_names: BTreeSet<_> = automation
        .get("scenes")
        .and_then(toml::Value::as_table)
        .into_iter()
        .flat_map(|scenes| scenes.keys().cloned())
        .collect();
    let prefix = (0..=128)
        .map(|n| {
            if n == 0 {
                "dev-pi".to_owned()
            } else {
                format!("dev-pi-{n}")
            }
        })
        .find(|prefix| {
            ["present", "working", "finished"]
                .iter()
                .all(|suffix| !ids.contains(&format!("{prefix}-{suffix}")))
                && ["idle", "working", "finished"]
                    .iter()
                    .all(|suffix| !scene_names.contains(&format!("{prefix}-{suffix}")))
        })
        .ok_or("no unused development Pi scene namespace")?;
    let starter: toml::Value = toml::from_str(include_str!("../examples/matrix-pi.toml"))?;
    let defaults = starter
        .get("automation")
        .ok_or("missing bundled Pi defaults")?;
    let rules = automation
        .entry("rules")
        .or_insert_with(|| toml::Value::Array(Vec::new()))
        .as_array_mut()
        .ok_or("automation.rules must be an array")?;
    for original_rule in defaults
        .get("rules")
        .and_then(toml::Value::as_array)
        .ok_or("missing bundled Pi rules")?
    {
        let mut rule = original_rule.clone();
        for field in ["id", "scene"] {
            let suffix = rule
                .get(field)
                .and_then(toml::Value::as_str)
                .and_then(|name| name.strip_prefix("pi-"))
                .ok_or("invalid bundled Pi rule")?;
            let name = format!("{prefix}-{suffix}");
            rule.as_table_mut()
                .ok_or("invalid bundled Pi rule")?
                .insert(field.into(), toml::Value::String(name));
        }
        rules.push(rule);
    }
    let scenes = automation
        .entry("scenes")
        .or_insert_with(|| toml::Value::Table(toml::Table::new()))
        .as_table_mut()
        .ok_or("automation.scenes must be a table")?;
    for (name, scene) in defaults
        .get("scenes")
        .and_then(toml::Value::as_table)
        .ok_or("missing bundled Pi scenes")?
    {
        let suffix = name.strip_prefix("pi-").ok_or("invalid bundled Pi scene")?;
        scenes.insert(format!("{prefix}-{suffix}"), scene.clone());
    }
    Ok(toml::to_string(&profile)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AppConfig;

    #[test]
    fn keeps_existing_visuals_sources_and_unrelated_automation() {
        let original = "effect='matrix'\nbrightness=83\nspeed=65\nfps=45\n\
            [[automation.rules]]\nid='checks'\nsource='ci'\nmode='while-state'\nscene='warning'\n\
            [automation.scenes.warning]\ncolor=[255,0,0]\nzones=['navigation']\n\
            [automation.groups]\ncustom=['F1']\n";
        let before: toml::Value = toml::from_str(original).unwrap();
        let merged = config(original).unwrap();
        let after: toml::Value = toml::from_str(&merged).unwrap();
        for key in ["effect", "brightness", "speed", "fps"] {
            assert_eq!(before[key], after[key]);
        }
        assert_eq!(
            before["automation"]["rules"][0],
            after["automation"]["rules"][0]
        );
        assert_eq!(
            before["automation"]["scenes"]["warning"],
            after["automation"]["scenes"]["warning"]
        );
        assert_eq!(
            before["automation"]["groups"],
            after["automation"]["groups"]
        );
        assert_eq!(after["automation"]["rules"].as_array().unwrap().len(), 4);
        toml::from_str::<AppConfig>(&merged)
            .unwrap()
            .validate()
            .unwrap();
        assert_eq!(config(&merged).unwrap(), merged);
    }

    #[test]
    fn explicit_pi_rules_are_unchanged_byte_for_byte() {
        let original = include_str!("../examples/matrix-pi.toml");
        assert_eq!(config(original).unwrap(), original);
        let custom = original.replace("pi-present", "my-pi-present");
        assert_eq!(config(&custom).unwrap(), custom);
    }

    #[test]
    fn namespace_collisions_do_not_replace_user_entries() {
        let original = "effect='breath'\n\
            [[automation.rules]]\nid='dev-pi-present'\nsource='ci'\nmode='while-state'\nscene='dev-pi-idle'\n\
            [automation.scenes.dev-pi-idle]\ncolor=[1,2,3]\n";
        let merged = config(original).unwrap();
        let value: toml::Value = toml::from_str(&merged).unwrap();
        assert_eq!(
            value["automation"]["scenes"]["dev-pi-idle"]["color"]
                .as_array()
                .unwrap()[0]
                .as_integer(),
            Some(1)
        );
        assert!(value["automation"]["scenes"].get("dev-pi-1-idle").is_some());
        toml::from_str::<AppConfig>(&merged)
            .unwrap()
            .validate()
            .unwrap();
    }
}
