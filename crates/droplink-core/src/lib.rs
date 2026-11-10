#![forbid(unsafe_code)]
//! DropLink core: protocol, peers, frames, hashing, paths, registry, config and crypto.
//!
//! This crate is pure: no I/O, no async, no runtime.

pub mod config;
pub mod crypto;
pub mod error;
pub mod frame;
pub mod hash;
pub mod path;
pub mod peer;
pub mod protocol;
pub mod registry;
pub mod relay;

pub use error::{Error, Result};
