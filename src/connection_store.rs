//! Private engine-owned connection persistence. No hardware or producer acquisition here.
use crate::ownership::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;
use underglow::connection_registry::{ConnectionRegistry, MAX_REGISTRY_BYTES};

pub fn select_warning(text: &str) -> Option<&'static str> {
    let value: toml::Value = toml::from_str(text).ok()?;
    value.get("connectors").and_then(toml::Value::as_array)
        .filter(|items| !items.is_empty())
        .map(|_| "visual profile connector definitions were ignored; use control connections replace to explicitly configure managed producers")
}

pub fn read_json(input: impl Read) -> Result<ConnectionRegistry, Error> {
    let mut bytes = Vec::new();
    input
        .take(MAX_REGISTRY_BYTES as u64 + 1)
        .read_to_end(&mut bytes)?;
    Ok(ConnectionRegistry::from_json(&bytes)?)
}

/// Existing registry always wins, including an explicitly empty registry.
pub fn load_or_migrate(
    dir: &Path,
    legacy: &[crate::managed_connectors::ConnectorConfig],
) -> Result<ConnectionRegistry, Error> {
    let path = dir.join("connections.json");
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            if !metadata.is_file() || metadata.permissions().mode() & 0o077 != 0 {
                return Err("connections.json must be a private regular file (0600)".into());
            }
            read_json(File::open(path)?)
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            let registry = ConnectionRegistry::from_legacy(legacy)?;
            save(dir, &registry)?;
            Ok(registry)
        }
        Err(error) => Err(error.into()),
    }
}

/// Strip machine secrets from every saved visual profile, including legacy Select.
pub fn visual_config(text: &str) -> Result<String, Error> {
    let mut value: toml::Value = toml::from_str(text)?;
    let root = value
        .as_table_mut()
        .ok_or("configuration must be a table")?;
    if root.remove("connectors").is_none() {
        return Ok(text.to_string());
    }
    Ok(toml::to_string(&value)?)
}

pub fn save(dir: &Path, registry: &ConnectionRegistry) -> Result<(), Error> {
    registry.validate()?;
    let temp = dir.join(format!(".connections-{}.tmp", std::process::id()));
    // If create_new fails, the existing temporary entry isn't ours to remove.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&temp)?;
    let result = (|| -> Result<(), Error> {
        let directory = File::open(dir)?;
        serde_json::to_writer(&mut file, registry)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temp, dir.join("connections.json"))?;
        // Rename commits the edit. Directory sync failure affects durability,
        // not whether the in-memory registry/supervisor must adopt the new intent.
        if let Err(error) = directory.sync_all() {
            eprintln!("warning: connections committed, directory sync failed: {error}");
        }
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use underglow::connection_registry::ConnectorConfig;
    static NEXT: AtomicUsize = AtomicUsize::new(0);

    fn directory() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "ug-registry-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn migration_is_once_only_private_and_removed_connections_do_not_resurrect() {
        let dir = directory();
        let legacy = vec![ConnectorConfig {
            id: "test".into(),
            source: "private".into(),
            command: vec!["never-run".into(), "secret".into()],
            enabled: true,
            trusted: true,
            ..Default::default()
        }];
        let registry = load_or_migrate(&dir, &legacy).unwrap();
        assert_eq!(registry.managed_configs(), legacy);
        assert_eq!(
            fs::metadata(dir.join("connections.json"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        let mut empty = registry.clone();
        empty.connections.clear();
        let empty = registry.replace(0, &empty).unwrap();
        save(&dir, &empty).unwrap();
        assert_eq!(load_or_migrate(&dir, &legacy).unwrap(), empty);
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn failure_before_rename_preserves_durable_registry_and_rejects_bad_storage() {
        let dir = directory();
        let registry = load_or_migrate(&dir, &[]).unwrap();
        let before = fs::read(dir.join("connections.json")).unwrap();
        let temp = dir.join(format!(".connections-{}.tmp", std::process::id()));
        fs::write(&temp, "occupied").unwrap();
        assert!(save(&dir, &registry.replace(0, &registry).unwrap()).is_err());
        assert_eq!(fs::read(dir.join("connections.json")).unwrap(), before);
        assert_eq!(fs::read_to_string(&temp).unwrap(), "occupied");
        fs::remove_file(temp).unwrap();
        fs::set_permissions(
            dir.join("connections.json"),
            fs::Permissions::from_mode(0o644),
        )
        .unwrap();
        assert!(load_or_migrate(&dir, &[]).is_err());
        fs::remove_file(dir.join("connections.json")).unwrap();
        std::os::unix::fs::symlink("/not-a-registry", dir.join("connections.json")).unwrap();
        assert!(load_or_migrate(&dir, &[]).is_err());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn strip_machine_config_preserves_visual_fields_and_warns_on_select() {
        let text = "effect='matrix'\n[[connectors]]\nid='secret'\ncommand=['token']\n";
        let stripped = visual_config(text).unwrap();
        assert!(select_warning(text).is_some());
        assert!(select_warning(&stripped).is_none());
        assert!(!stripped.contains("secret"));
        let value: toml::Value = toml::from_str(&stripped).unwrap();
        assert_eq!(value["effect"].as_str(), Some("matrix"));
    }
}
