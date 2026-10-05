//! Per-user process locks. Lock files are never unlinked (which would split locks).
use std::fs::{self, File, OpenOptions};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
pub type Error = Box<dyn std::error::Error>;

pub fn state_dir() -> Result<PathBuf, Error> {
    // Compatibility identity, not product branding: changing this also changes
    // the hardware lease namespace and could allow an older engine to coexist.
    if let Some(path) = std::env::var_os("WOOTING_STATE_DIR") {
        return Ok(path.into());
    }
    let home = std::env::var_os("HOME").ok_or("HOME is not set; set WOOTING_STATE_DIR")?;
    if cfg!(target_os = "macos") {
        Ok(PathBuf::from(home).join("Library/Application Support/wooting-signals/runtime"))
    } else {
        Ok(std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(home).join(".local/state"))
            .join("wooting-signals"))
    }
}
pub fn private_dir(path: &Path) -> Result<(), Error> {
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)?;
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err("state directory must be a real directory".into());
    }
    // Compare against a newly-created file owned by this process without libc.
    let probe_path = path.join(format!(".owner-{}", std::process::id()));
    let probe = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&probe_path)?;
    let uid = probe.metadata()?.uid();
    drop(probe);
    fs::remove_file(probe_path)?;
    if meta.uid() != uid {
        return Err("state directory belongs to another user".into());
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
pub struct Lease {
    _file: File,
}
impl Lease {
    pub fn acquire(kind: &str) -> Result<Self, Error> {
        let dir = state_dir()?;
        private_dir(&dir)?;
        let path = dir.join(format!("{kind}.lock"));
        if fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink() || !m.is_file()) {
            return Err("lock path must be a regular file".into());
        }
        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .mode(0o600)
            .open(&path)?;
        file.try_lock().map_err(|_| {
            format!("another Underglow or legacy Wooting Signals process holds the {kind} lock; stop it first")
        })?;
        Ok(Self { _file: file })
    }
}
