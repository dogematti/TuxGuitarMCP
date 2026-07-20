//! Client side of the TuxGuitar bridge protocol (and a simulator of the
//! Java plugin, used by integration tests and `tabmcp bridge-sim`).
//!
//! Transport: JSON-RPC 2.0, one message per newline-terminated UTF-8 line,
//! over loopback TCP. The plugin listens; we connect. Discovery and
//! authentication go through `~/.tuxguitar-mcp/bridge.json`.

pub mod client;
pub mod discovery;
pub mod error;
pub mod sim;

pub use client::BridgeClient;
pub use discovery::{default_discovery_path, read_discovery};
pub use error::BridgeError;
