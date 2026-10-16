//! Wire types for lull.
//!
//! This crate is the single definition of every shape that crosses the daemon socket:
//! the data model of `docs/contract.md` §4, the configuration of §6, the stopped-server
//! record of §7 and the JSON-RPC methods, notifications and browser commands of §8.
//! It contains no logic beyond (de)serialisation.
//!
//! The golden exchanges in `protocol/fixtures/*.ndjson` are decoded by the tests of this
//! crate, by the extension and by the macOS app, so the three implementations cannot
//! drift from each other.

pub mod config;
pub mod history;
pub mod rpc;
pub mod server;

pub use config::{AgentMarker, AutoStop, Config, ConfigPatch, ProjectRule, TabsOnStop};
pub use history::{RestartStatus, StopReason, StoppedServer};
pub use rpc::ProtocolVersion;
pub use server::{
    Activity, Agent, Framework, Origin, OriginKind, PinSource, RestartedFrom, Server, ServerState,
    TabRef,
};

/// Protocol version spoken by this crate (`hello.protocol`, contract §8).
///
/// A different `major` is rejected with [`rpc::PROTOCOL_MISMATCH`]; `minor` grows with
/// additive changes (contract §8, "Versioning").
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 1, minor: 3 };
