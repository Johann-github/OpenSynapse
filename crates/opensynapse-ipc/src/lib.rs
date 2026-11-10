//! Local IPC between the OpenSynapse agent and its clients (CLI, GUI).
//!
//! - [`msg`]: message types (pure data).
//! - [`codec`]: versioned binary encoding of those messages (pure, std only).
//! - [`frame`]: length-prefixed framing over `Read`/`Write`.
//! - [`transport`]: per-user local endpoint (Windows named pipe; Unix socket later).
//! - [`client`]: blocking client for control calls and event subscriptions.
//!
//! This crate does not touch HID; it reuses `opensynapse-protocol` value types only.

pub mod client;
pub mod codec;
pub mod frame;
pub mod msg;
pub mod transport;

pub use client::{Client, Subscription};
pub use codec::DecodeError;
