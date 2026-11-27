//! The public plugin.

use std::path::PathBuf;

use bevy::{prelude::*, remote::http::DEFAULT_PORT};

use crate::{
    checks::DriverChecksPlugin,
    clipboard_injection::DriverClipboardPlugin,
    executor::DriverExecutorPlugin,
    gamepad_injection::DriverGamepadPlugin,
    injection::DriverWindowEventPlugin,
    real_input::{DriverRealInputPlugin, place_window_off_screen_for_scripts},
    remote::DriverRemotePlugin,
    render_readiness::DriverRenderReadinessPlugin,
    script_file::{load_script_file, log_driver_startup},
    seed::DriverSeedPlugin,
    state_handlers::DriverStateHandlersPlugin,
    targets::DriverTargetsPlugin,
    time_control::DriverTimeControlPlugin,
};

/// Settings the startup systems read.
#[derive(Resource, Debug, Clone)]
pub struct DriverSettings {
    /// BRP HTTP port.
    pub port: u16,
    /// Whether the BRP methods are served.
    pub enable_remote: bool,
    /// Script to load at startup when `--input-script` is absent.
    pub script_path: Option<PathBuf>,
    /// Whether injected cursor moves also write `Window::cursor_position`.
    pub update_window_cursor_position: bool,
    /// Whether a script run starts with the primary window off every monitor.
    pub off_screen_for_scripts: bool,
}

/// Lets an automated tester inject input and take screenshots in a running game.
///
/// When [`InputDriverPlugin::enable_remote`] is true this plugin adds `RemotePlugin` and
/// `RemoteHttpPlugin` itself, so the game must not add `RemotePlugin`. Add this plugin after
/// `DefaultPlugins`.
#[derive(Debug, Clone)]
pub struct InputDriverPlugin {
    /// BRP HTTP port on 127.0.0.1. Defaults to bevy_remote's default, 15702.
    pub port: u16,
    /// Script of JSON Lines commands to run at startup. The `--input-script <path>` process
    /// argument overrides it.
    pub script_path: Option<PathBuf>,
    /// Serve the `drive/*` BRP methods. Defaults to true.
    pub enable_remote: bool,
    /// Seed stored in `DriverSeed`. The `--seed <u64>` process argument overrides it.
    pub seed: Option<u64>,
    /// Also write injected cursor positions to `Window::cursor_position`, for games that call
    /// `window.cursor_position()`. Defaults to false.
    ///
    /// The write bypasses change detection, but `bevy_winit` compares each changed window with
    /// a private cache, so any later change to the `Window` (a title, a cursor icon) warps the
    /// user's real OS cursor to the injected position. Turn it on only for unattended runs.
    pub update_window_cursor_position: bool,
    /// When a script is given (`--input-script` or [`InputDriverPlugin::script_path`]) and the
    /// process arguments lack `--on-screen`, start the primary window far off every monitor and
    /// unfocused, so it neither covers the user's screen nor steals focus. Defaults to true.
    pub off_screen_for_scripts: bool,
}

impl Default for InputDriverPlugin {
    fn default() -> Self {
        Self {
            port: DEFAULT_PORT,
            script_path: None,
            enable_remote: true,
            seed: None,
            update_window_cursor_position: false,
            off_screen_for_scripts: true,
        }
    }
}

impl Plugin for InputDriverPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(DriverSettings {
            port: self.port,
            enable_remote: self.enable_remote,
            script_path: self.script_path.clone(),
            update_window_cursor_position: self.update_window_cursor_position,
            off_screen_for_scripts: self.off_screen_for_scripts,
        })
        .add_plugins((
            DriverRealInputPlugin,
            DriverGamepadPlugin,
            DriverWindowEventPlugin,
            DriverClipboardPlugin,
            DriverExecutorPlugin,
            DriverSeedPlugin { seed: self.seed },
            DriverTimeControlPlugin,
            DriverStateHandlersPlugin,
            DriverChecksPlugin,
            DriverTargetsPlugin,
            DriverRenderReadinessPlugin,
        ))
        .add_systems(Startup, (load_script_file, log_driver_startup).chain());

        if self.enable_remote {
            app.add_plugins(DriverRemotePlugin { port: self.port });
        }
    }

    // After every plugin was built, so the primary window exists, and before the runner creates
    // the operating system window, so it never shows on screen first.
    fn finish(&self, app: &mut App) {
        place_window_off_screen_for_scripts(
            app.world_mut(),
            self.off_screen_for_scripts,
            self.script_path.as_deref(),
        );
    }
}
