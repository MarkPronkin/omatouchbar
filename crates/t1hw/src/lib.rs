//! Client for the T1Bridge Touch Bar hardware IPC v1.
//!
//! A renderer runs as the logged-in user, negotiates the panel dimensions,
//! submits complete XRGB8888 frames through sealed memfd buffers, receives
//! bounded touch input, and requests only typed actions (key taps,
//! brightness, Touch ID cancellation). See T1Bridge `docs/interfaces.md`.

pub mod client;
pub mod transport;
pub mod wire;

pub use client::{Client, ConnectError, Event};
pub use transport::SOCKET_PATH;
pub use wire::{Contact, ErrorCode, Features, InputFrame, Rect, key};
