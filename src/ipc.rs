#![allow(dead_code)]

//! Reserved boundary for a future background daemon.
//!
//! The initial release talks to `rbw` directly. Anything that later needs a
//! long-lived process should be implemented behind this trait so the frontend
//! does not care whether the backend is in-process or a daemon.

use crate::backend::VaultProvider;

pub trait DaemonProvider: VaultProvider {}

pub type DaemonHandle = std::sync::Arc<dyn DaemonProvider>;
