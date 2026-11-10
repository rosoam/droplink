#![forbid(unsafe_code)]
//! DropLink networking: transfer engine, framing, discovery and transports.

pub mod discovery;
pub mod engine;
pub mod fs_util;
pub mod receiver;
pub mod relay_client;
pub mod sender;
pub mod stream;
pub mod transport;

pub use engine::{
    spawn, Bound, Command, EngineConfig, EngineHandle, Event, IncomingTransfer, ProgressSnapshot,
};
