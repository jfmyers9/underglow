//! Inert custom producer metadata. This is NOT a connection-registry import:
//! argv, environment, directories, trust and enable flags are never accepted.
use crate::connection_registry::{Connection, ConnectionRegistry};
use crate::source_capabilities::{MAX_DESCRIPTOR_BYTES, SourceDescriptor};
use std::io::Read;
use std::path::Path;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ManifestFormat {
    Toml,
    Json,
}

/// A manifest is exactly one strict v1 SourceDescriptor (64 KiB maximum).
pub fn parse(bytes: &[u8], format: ManifestFormat) -> Result<SourceDescriptor, String> {
    match format {
        ManifestFormat::Json => SourceDescriptor::from_json(bytes),
        ManifestFormat::Toml => {
            if bytes.len() > MAX_DESCRIPTOR_BYTES {
                return Err("source descriptor exceeds 64 KiB".into());
            }
            SourceDescriptor::from_toml(
                std::str::from_utf8(bytes).map_err(|_| "manifest must be UTF-8")?,
            )
        }
    }
}

/// Read only a bounded regular file; FIFOs/devices are not manifest inputs.
/// File paths are local UI choices, never provided by a setup or manifest.
pub fn read_file(path: &Path, format: ManifestFormat) -> Result<SourceDescriptor, String> {
    if !std::fs::symlink_metadata(path)
        .map_err(|_| "cannot inspect manifest file")?
        .file_type()
        .is_file()
    {
        return Err("manifest must be a regular file (not a symlink or device)".into());
    }
    let file = std::fs::File::open(path).map_err(|_| "cannot open manifest file")?;
    if !file
        .metadata()
        .map_err(|_| "cannot inspect manifest file")?
        .is_file()
    {
        return Err("manifest must be a regular file".into());
    }
    let mut bytes = Vec::new();
    file.take((MAX_DESCRIPTOR_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|_| "cannot read manifest file")?;
    parse(&bytes, format)
}

/// Build an atomic candidate for the existing engine CAS transport. Revision is
/// unchanged until persisted by the engine. An existing producer is copied
/// verbatim, including its trust, disabled intent, private env and working dir.
pub fn import_descriptor(
    registry: &ConnectionRegistry,
    expected_revision: u64,
    descriptor: SourceDescriptor,
    replace_metadata: bool,
) -> Result<ConnectionRegistry, String> {
    registry.validate()?;
    if registry.revision != expected_revision {
        return Err("connection registry revision conflict; refresh and review again".into());
    }
    descriptor.validate()?;
    let mut candidate = registry.clone();
    if let Some(connection) = candidate
        .connections
        .iter_mut()
        .find(|c| c.descriptor.source == descriptor.source)
    {
        if !replace_metadata {
            return Err(
                "source ID already exists; explicitly confirm metadata-only replacement".into(),
            );
        }
        connection.descriptor = descriptor;
    } else {
        candidate.connections.push(Connection {
            descriptor,
            producer: None,
        });
    }
    candidate.validate()?;
    Ok(candidate)
}
