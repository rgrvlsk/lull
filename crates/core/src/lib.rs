//! Core logic of lull: discovery, activity, policy and stopping (contract §4–§7).
//!
//! Core never calls libproc, ntstat, signals, clocks or the filesystem directly; every
//! platform touch point goes through one of the traits in [`traits`], implemented in
//! [`platform`] and faked in [`testing`]. Shared domain types live in [`model`].

pub mod model;
pub mod traits;

pub mod activity;
pub mod browser;
pub mod discovery;
pub mod ipc;
pub mod lifecycle;
pub mod platform;
pub mod policy;
pub mod state;

pub mod testing;

pub use traits::{
    ActivityBackend, ActivityBackendKind, Clock, ConfigStore, Instant, ProcessSource, Signal,
    Signaler, SpawnRequest, Spawner, StateStore,
};
