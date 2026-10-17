//! Inert, bounded setup sharing. This module has no engine or producer transport.
//! Files contain only strict SetupDocument v1 data, with neutral source roles.
use crate::binding_repair::{self, Binding, BindingKey, BindingRepair};
use crate::connection_registry::ConnectionRegistry;
use crate::layer_runtime::CompiledSetup;
use crate::setup::{Activation, MAX_DOCUMENT_BYTES, SetupDocument};
use crate::source_capabilities::CapabilityWarning;
use std::collections::BTreeMap;
use std::fs::{self, OpenOptions};
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

#[cfg(test)]
#[path = "setup_sharing_tests.rs"]
mod tests;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Toml,
    Json,
}

fn source_mut(activation: &mut Activation) -> Option<&mut String> {
    match activation {
        Activation::Always {} => None,
        Activation::WhileState { source, .. }
        | Activation::OnEvent { source, .. }
        | Activation::OnTransition { source, .. } => Some(source),
    }
}

/// Does not accept AppConfig or a connection registry: private credentials,
/// argv, paths, executable trust and engine enabled state cannot enter the file.
/// Source names become role-1, role-2, ...; fact/instance selectors stay intent.
pub fn export_document(document: &SetupDocument, format: Format) -> Result<String, String> {
    CompiledSetup::compile(document)?;
    let mut document = document.clone();
    document.revision = 0;
    let mut roles = BTreeMap::new();
    for layer in &mut document.layers {
        for activation in std::iter::once(&mut layer.activation)
            .chain(layer.variants.iter_mut().map(|v| &mut v.activation))
        {
            if let Some(source) = source_mut(activation) {
                let next = format!("role-{}", roles.len() + 1);
                *source = roles.entry(source.clone()).or_insert(next).clone();
            }
        }
    }
    let encoded = match format {
        Format::Toml => document.to_toml()?,
        Format::Json => serde_json::to_string_pretty(&document).map_err(|e| e.to_string())?,
    };
    if encoded.len() > MAX_DOCUMENT_BYTES {
        return Err("shared setup exceeds 256 KiB".into());
    }
    Ok(encoded)
}

/// Explicit destination, create-new only. Publish a fully written private file
/// atomically without truncating or replacing any existing destination (including
/// symlinks). A failed validation/write leaves existing files untouched.
pub fn export_file(path: &Path, document: &SetupDocument, format: Format) -> Result<(), String> {
    let encoded = export_document(document, format)?;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos();
    let temporary = parent.join(format!(
        ".underglow-share-{}-{nonce}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&temporary).map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(encoded.as_bytes())?;
        file.sync_all()?;
        // hard_link is an atomic create-new operation, unlike rename on Unix.
        fs::hard_link(&temporary, path)
    })();
    drop(file);
    let _ = fs::remove_file(&temporary);
    result.map_err(|e| format!("export not saved (choose a new destination): {e}"))
}

pub struct ImportPreview {
    document: SetupDocument,
}

#[derive(Clone, Debug)]
pub struct BindingMapping {
    pub key: BindingKey,
    pub source: String,
    /// Must be selected explicitly, even when keeping the exported exact ID.
    pub instance: Option<String>,
}

pub struct ImportCandidate {
    pub document: SetupDocument,
    pub warnings: Vec<CapabilityWarning>,
}

impl ImportPreview {
    pub fn parse(bytes: &[u8], format: Format) -> Result<Self, String> {
        if bytes.len() > MAX_DOCUMENT_BYTES {
            return Err("shared setup exceeds 256 KiB".into());
        }
        let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
        let mut document = match format {
            Format::Toml => SetupDocument::from_toml(text)?,
            Format::Json => {
                serde_json::from_str::<SetupDocument>(text).map_err(|e| e.to_string())?
            }
        };
        CompiledSetup::compile(&document)?;
        document.revision = 0;
        Ok(Self { document })
    }

    pub fn read(path: &Path, format: Format) -> Result<Self, String> {
        if !fs::metadata(path).map_err(|e| e.to_string())?.is_file() {
            return Err("import requires a regular local file".into());
        }
        let file = fs::File::open(path).map_err(|e| e.to_string())?;
        let mut bytes = Vec::new();
        file.take(MAX_DOCUMENT_BYTES as u64 + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        Self::parse(&bytes, format)
    }

    /// Preview-only: never submit this still-unmapped document to the engine.
    pub fn document(&self) -> &SetupDocument {
        &self.document
    }

    pub fn bindings(&self) -> Vec<Binding<'_>> {
        binding_repair::bindings(&self.document)
    }

    /// All bindings (including disabled appearances) require explicit approval.
    /// Matching local names grant nothing. Unknown facts are preserved, warned,
    /// never translated. This returns only a new saved-library candidate.
    pub fn prepare(
        &self,
        new_id: &str,
        mappings: &[BindingMapping],
        registry: &ConnectionRegistry,
        existing_ids: &[String],
    ) -> Result<ImportCandidate, String> {
        if new_id == self.document.id || existing_ids.iter().any(|id| id == new_id) {
            return Err(
                "choose a new local setup ID; import never replaces an existing setup".into(),
            );
        }
        registry.validate()?;
        let bindings = self.bindings();
        if mappings.len() != bindings.len()
            || mappings
                .iter()
                .any(|m| bindings.iter().filter(|b| b.key == m.key).count() != 1)
            || bindings
                .iter()
                .any(|b| mappings.iter().filter(|m| m.key == b.key).count() != 1)
        {
            return Err(
                "explicitly map every source binding once, including disabled variants".into(),
            );
        }
        let mut candidate = self.document.clone();
        let mut warnings = Vec::new();
        for binding in bindings {
            let mapping = mappings.iter().find(|m| m.key == binding.key).unwrap();
            let (updated, mut issues) = binding_repair::repair_binding(
                &candidate,
                registry,
                &BindingRepair {
                    setup_id: candidate.id.clone(),
                    key: binding.key,
                    expected_activation: binding.activation.clone(),
                    source: mapping.source.clone(),
                    instance: mapping.instance.clone(),
                },
            )?;
            candidate = updated;
            warnings.append(&mut issues);
        }
        candidate.id = new_id.into();
        candidate.revision = 0;
        CompiledSetup::compile(&candidate)?;
        // Rebinding can expand every selector. Preflight the destination's
        // bounded saved-document representation before offering Store.
        candidate.to_toml()?;
        Ok(ImportCandidate {
            document: candidate,
            warnings,
        })
    }
}
