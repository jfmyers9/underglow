//! Explicit one-time development starter; never a recurring runtime mode.
use crate::config::AppConfig;
use underglow::setup::{Target, TargetZone};

type Error = Box<dyn std::error::Error>;

pub(crate) fn config(original: &str) -> Result<String, Error> {
    let before: AppConfig = toml::from_str(original)?;
    // A canonical document is authoritative even if Pi (or every layer) was deleted.
    if before.setup.is_some() {
        return Ok(original.into());
    }
    // Recognize only the complete untouched bundled automation table. Even a
    // single extra field/rule, changed color/target, instance, or priority keeps
    // the user's independent rules. This happens once, before canonical data.
    let source: toml::Value = toml::from_str(original)?;
    let bundled: toml::Value = toml::from_str(include_str!("../examples/matrix-pi.toml"))?;
    let bundled_pi = source.get("automation") == bundled.get("automation");
    let migrated = crate::migration::migrate(original)?;
    if migrated.warning.is_some() || (!before.automation.rules.is_empty() && !bundled_pi) {
        // Existing rules retain their exact migration semantics. Don't guess a
        // single stack position across the old starter's 10/20/30 priorities.
        // Add the template explicitly in the GUI instead.
        return Ok(migrated.config);
    }
    let canonical: AppConfig = toml::from_str(&migrated.config)?;
    let mut document = canonical.setup.ok_or("migration produced no setup")?;
    if bundled_pi {
        // Exact matching proves these are the known trio and no custom rules.
        document.layers.truncate(1);
    }
    if document.layers.len() == underglow::setup::MAX_LAYERS {
        return Ok(migrated.config);
    }
    let id = (0..=128)
        .map(|n| format!("dev-pi-{n}"))
        .find(|id| document.layers.iter().all(|layer| &layer.id != id))
        .ok_or("no unused Pi starter namespace")?;
    let pi = underglow::templates::pi_status_layer(
        id,
        Target::Selection {
            zones: vec![TargetZone::Function],
            keys: vec![],
            groups: vec![],
        },
        None,
    );
    document.layers.push(pi);
    document.validate()?;
    let mut value: toml::Value = toml::from_str(&migrated.config)?;
    value
        .as_table_mut()
        .ok_or("configuration must be a table")?
        .insert("setup".into(), toml::Value::try_from(&document)?);
    Ok(toml::to_string(&value)?)
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
        assert_eq!(parsed.setup.as_ref().unwrap().layers.len(), 2);
        assert_eq!(parsed.setup.as_ref().unwrap().layers[1].variants.len(), 2);
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

    #[test]
    fn only_exact_bundled_trio_folds_once_and_customizations_are_not_guessed() {
        let bundled = include_str!("../examples/matrix-pi.toml");
        let merged = config(bundled).unwrap();
        let parsed: AppConfig = toml::from_str(&merged).unwrap();
        let doc = parsed.setup.unwrap();
        assert_eq!(doc.layers.len(), 2);
        assert_eq!(
            doc.layers[1],
            underglow::templates::pi_status_layer(
                "dev-pi-0".into(),
                Target::Selection {
                    zones: vec![TargetZone::Function],
                    keys: vec![],
                    groups: vec![]
                },
                None
            )
        );
        assert_eq!(config(&merged).unwrap(), merged);
        for customized in [
            bundled.replace("[0, 100, 255]", "[1, 100, 255]"),
            bundled.replace("priority = 20", "priority = 21"),
            bundled.replace("source = \"pi\"", "source = \"pi\"\ninstance = \"chosen\""),
            bundled.replace("[\"function\"]", "[\"arrows\"]"),
            format!("{bundled}\n[automation.scenes.extra]\ncolor = [1, 2, 3]\n"),
        ] {
            assert_eq!(
                config(&customized).unwrap(),
                crate::migration::migrate(&customized).unwrap().config
            );
            let parsed: AppConfig = toml::from_str(&config(&customized).unwrap()).unwrap();
            assert_eq!(parsed.setup.unwrap().layers.len(), 4);
        }
        // An already-canonical old bundled stack is not upgraded/re-injected.
        let old_canonical = crate::migration::migrate(bundled).unwrap().config;
        assert_eq!(config(&old_canonical).unwrap(), old_canonical);
    }

    #[test]
    fn deleted_appearance_and_empty_canonical_stack_are_authoritative() {
        let merged = config("effect='aurora'\n").unwrap();
        let mut value: toml::Value = toml::from_str(&merged).unwrap();
        value["setup"]["layers"][1]["variants"]
            .as_array_mut()
            .unwrap()
            .remove(1);
        let deleted = toml::to_string(&value).unwrap();
        assert_eq!(config(&deleted).unwrap(), deleted);
        assert_eq!(crate::migration::migrate(&deleted).unwrap().config, deleted);
        value["setup"]["layers"] = toml::Value::Array(vec![]);
        let blank = toml::to_string(&value).unwrap();
        assert_eq!(config(&blank).unwrap(), blank);
    }
}
