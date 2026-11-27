//! Drive a running Bevy game with injected keyboard, mouse, scroll, touch, and gesture input,
//! and take screenshots.
//!
//! Commands arrive live over the Bevy Remote Protocol (`drive/send`) or from a JSON Lines file
//! named at launch with `--input-script <path>`. Each command expands into per-frame steps that
//! write the same messages the windowing backend writes, so the game cannot tell injected input
//! from real input.
//!
//! Add [`InputDriverPlugin`] to the app, typically behind a cargo feature.
//!
//! A game can make a launch script check itself. Register a check with
//! [`InputDriverAppExt::add_driver_check`], then write `{"command":"expect","check":"name",
//! "arguments":<optional>,"equals":<optional>}` in the script or in `drive/send`. Each result lands in
//! the `checks` list of `drive/status`, every failure also adds an entry to `errors` and counts
//! in `checks_failed`, and a `quit` exits with code 1 when `checks_failed` is above 0.
//! [`InputDriverAppExt::add_driver_remote_method`] adds the game's own BRP methods.
//!
//! A script can also point by meaning instead of by pixel. Register a target with
//! [`InputDriverAppExt::add_driver_target`], a system that turns JSON arguments into a logical
//! window position, then write `{"command":"click","at":{"target":"name","column":3}}`. The
//! system receives every key of `at` except `target` as a JSON object. `click`, `mouse_move`,
//! and `touch` take `at` instead of `x` and `y`, and the ends of `drag` and `touch_drag` take
//! either `[x, y]` or a target object. A target resolves right before its command runs. If it is
//! unknown or returns an error, the command sends no input, the failure lands in `errors` and
//! counts in `targets_failed`, and a `quit` exits with code 1.

#![warn(missing_docs)]

pub mod checks;
pub mod commands;
pub mod executor;
pub mod injection;
pub mod plugin;
pub mod real_input;
pub mod remote;
pub mod render_readiness;
pub mod screenshot;
pub mod script_file;
pub mod seed;
pub mod state_handlers;
pub mod status;
pub mod steps;
pub mod targets;
pub mod time_control;
pub mod window_control;

pub use checks::{CheckResult, DriverChecks};
pub use commands::{CommandParseError, DriverCommand, DriverTarget, PointerPosition};
pub use plugin::InputDriverPlugin;
pub use render_readiness::{FailedPipeline, RenderReadiness};
pub use seed::DriverSeed;
pub use state_handlers::{DriverStateHandlers, InputDriverAppExt};
pub use status::DriverStatus;
pub use steps::{DriverQueue, DriverStep};
pub use targets::{DriverTargets, TargetError};
