use std::path::PathBuf;

use thiserror::Error;

#[derive(Debug, Error)]
pub enum BridgeError {
    #[error(
        "TuxGuitar bridge is not running (no discovery file at {0}). \
         Start TuxGuitar with the TabMCP plugin installed."
    )]
    NotRunning(PathBuf),

    #[error(
        "found a discovery file but could not connect to 127.0.0.1:{port}: {source}. \
         TuxGuitar may have exited without cleaning up; restart it."
    )]
    Unreachable {
        port: u16,
        #[source]
        source: std::io::Error,
    },

    #[error("bridge protocol version mismatch: we speak {ours}, the plugin speaks {theirs}")]
    VersionMismatch { ours: u32, theirs: u32 },

    #[error("bridge rejected the request: {code}: {message}")]
    Rejected { code: String, message: String },

    #[error("malformed message from bridge: {0}")]
    Malformed(String),

    #[error("bridge connection lost: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),
}
