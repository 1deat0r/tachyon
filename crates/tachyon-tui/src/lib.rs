//! Tachyon Tui.
//!
//! A pure gateway client (AD-014): three independent paths — an input task,
//! a gateway-event reader task, and a render loop — speaking only the
//! protocol over [`tachyon_gateway::transport`]. No database, model or tool
//! ownership lives here.

#![warn(unsafe_code)]

mod app;
mod command;
mod input;
mod poll;
mod reader;
mod render;
mod state;

pub use app::{AttachOptions, TuiError, run};
pub use command::{CallError, CommandClient};
pub use input::{Key, Outbound, handle_key};
pub use reader::{AttachConfig, ClientEvent, Subscription, backoff_delay};
pub use render::draw;
pub use state::{AppState, Pane};

/// Marker proving the crate is wired into the workspace scaffold.
pub const CRATE_NAME: &str = "tachyon-tui";
