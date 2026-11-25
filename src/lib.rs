//! Drive a running Bevy game with injected keyboard, mouse, scroll, touch, and gesture input,
//! and take screenshots.
//!
//! Commands arrive live over the Bevy Remote Protocol (`drive/send`) or from a JSON Lines file
//! named at launch with `--input-script <path>`. Each command expands into per-frame steps that
//! write the same messages the windowing backend writes, so the game cannot tell injected input
//! from real input.
//!
//! Add [`InputDriverPlugin`] to the app, typically behind a cargo feature.

#![warn(missing_docs)]

pub mod commands;
pub mod executor;
pub mod injection;
pub mod plugin;
pub mod remote;
pub mod render_readiness;
pub mod screenshot;
pub mod script_file;
pub mod seed;
pub mod state_handlers;
pub mod status;
pub mod steps;
pub mod time_control;
pub mod window_control;

pub use commands::{CommandParseError, DriverCommand};
pub use plugin::InputDriverPlugin;
pub use render_readiness::RenderReadiness;
pub use seed::DriverSeed;
pub use state_handlers::{DriverStateHandlers, InputDriverAppExt};
pub use status::DriverStatus;
pub use steps::{DriverQueue, DriverStep};
