//! Locating and parsing the bridge discovery file.

use std::path::{Path, PathBuf};

use tabmcp_model::DiscoveryInfo;

use crate::error::BridgeError;

/// `~/.tuxguitar-mcp/bridge.json` (or `%USERPROFILE%` on Windows).
pub fn default_discovery_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .unwrap_or_default();
    PathBuf::from(home)
        .join(".tuxguitar-mcp")
        .join("bridge.json")
}

pub fn read_discovery(path: &Path) -> Result<DiscoveryInfo, BridgeError> {
    let bytes = std::fs::read(path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            BridgeError::NotRunning(path.to_path_buf())
        } else {
            BridgeError::Io(e)
        }
    })?;
    Ok(serde_json::from_slice(&bytes)?)
}
