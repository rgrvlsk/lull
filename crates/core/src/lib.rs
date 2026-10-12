//! Core logic of lull: discovery, activity, policy and stopping (contract §4–§7).
//!
//! M0 ships trait definitions only. Core never calls libproc, ntstat, signals, clocks or
//! the filesystem directly; every platform touch point goes through one of the traits in
//! [`traits`], and every trait has a fake in [`testing`].

pub mod traits;

pub mod testing;

pub use traits::{
    ActivityBackend, ActivityBackendKind, ActivityEvent, Clock, Instant, ListenerSocket,
    ProcessInfo, ProcessSource, Signal, Signaler, Snapshot, SpawnRequest, Spawner, StateStore,
    TcpConnection,
};
