//! Game-provided save and load hooks, run by the `save_state` and `load_state` commands.

use std::{fmt, path::PathBuf};

use bevy::{ecs::system::SystemId, prelude::*, remote::BrpResult};
use serde_json::Value;

use crate::{
    checks::register_driver_check,
    remote::register_remote_method,
    status::{DriverStatus, record_error_in_world},
    targets::register_driver_target,
};

/// One-shot system id of a hook that receives the file path from the command.
pub type StateHandlerId = SystemId<In<PathBuf>, Result<(), String>>;

/// The registered save and load hooks. Fill it with
/// [`InputDriverAppExt::set_driver_state_handlers`].
#[derive(Resource, Debug, Default, Clone, Copy)]
pub struct DriverStateHandlers {
    /// Hook run by `save_state`.
    pub save: Option<StateHandlerId>,
    /// Hook run by `load_state`.
    pub load: Option<StateHandlerId>,
}

/// Which hook a state command runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StateOperation {
    /// Run the save hook.
    Save,
    /// Run the load hook.
    Load,
}

impl StateOperation {
    fn command_name(self) -> &'static str {
        match self {
            Self::Save => "save_state",
            Self::Load => "load_state",
        }
    }

    fn past_tense(self) -> &'static str {
        match self {
            Self::Save => "saved",
            Self::Load => "loaded",
        }
    }

    fn hook_name(self) -> &'static str {
        match self {
            Self::Save => "save",
            Self::Load => "load",
        }
    }
}

/// Why a state operation failed.
#[derive(Debug, PartialEq, Eq)]
pub enum StateOperationError {
    /// No hook of this kind is registered.
    NoHandler(StateOperation),
    /// Bevy could not run the registered system.
    CannotRun(String),
    /// The hook ran and returned an error.
    HandlerFailed(String),
}

impl fmt::Display for StateOperationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoHandler(operation) => write!(
                formatter,
                "no {} system registered: call set_driver_state_handlers",
                operation.hook_name()
            ),
            Self::CannotRun(problem) => write!(formatter, "cannot run handler: {problem}"),
            Self::HandlerFailed(problem) => write!(formatter, "handler failed: {problem}"),
        }
    }
}

impl std::error::Error for StateOperationError {}

/// Runs the registered hook for `operation` with `path`.
///
/// # Errors
///
/// Returns [`StateOperationError`] when no hook is registered, the hook cannot run, or the
/// hook returns an error.
pub fn run_state_operation(
    world: &mut World,
    operation: StateOperation,
    path: &str,
) -> Result<(), StateOperationError> {
    let handlers = world
        .get_resource::<DriverStateHandlers>()
        .copied()
        .unwrap_or_default();
    let handler = match operation {
        StateOperation::Save => handlers.save,
        StateOperation::Load => handlers.load,
    }
    .ok_or(StateOperationError::NoHandler(operation))?;
    world
        .run_system_with::<In<PathBuf>, Result<(), String>>(handler, PathBuf::from(path))
        .map_err(|problem| StateOperationError::CannotRun(problem.to_string()))?
        .map_err(StateOperationError::HandlerFailed)
}

/// Queues a world command that runs the hook and records the outcome in [`DriverStatus`].
pub fn queue_state_operation(commands: &mut Commands, operation: StateOperation, path: String) {
    commands.queue(
        move |world: &mut World| match run_state_operation(world, operation, &path) {
            Ok(()) => world
                .resource_mut::<DriverStatus>()
                .note_state_operation(format!("{} {path}", operation.past_tense())),
            Err(problem) => record_error_in_world(
                world,
                format!("{} {path}: {problem}", operation.command_name()),
            ),
        },
    );
}

/// Registers [`DriverStateHandlers`].
pub struct DriverStateHandlersPlugin;

impl Plugin for DriverStateHandlersPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DriverStateHandlers>();
    }
}

/// App methods for connecting a game's save and load code to the driver.
pub trait InputDriverAppExt {
    /// Registers the systems that `save_state` and `load_state` run.
    ///
    /// Each system receives the file path from the command as `In<PathBuf>` and returns
    /// `Ok(())` or a message describing why it failed. Both run as one-shot systems, so they
    /// may take any ordinary system parameters after the `In` input.
    fn set_driver_state_handlers<SaveMarker, LoadMarker>(
        &mut self,
        save_system: impl IntoSystem<In<PathBuf>, Result<(), String>, SaveMarker> + 'static,
        load_system: impl IntoSystem<In<PathBuf>, Result<(), String>, LoadMarker> + 'static,
    ) -> &mut Self;

    /// Registers a check the `expect` command can run by `name`.
    ///
    /// The system receives the command's `arguments` as `In<Value>` (`null` when absent) and returns
    /// the value it read, or an error message when the check fails by itself. Registering a name
    /// twice replaces the first check. Works before or after adding [`InputDriverPlugin`].
    ///
    /// A check should return `Err` rather than panic: a panic crashes the game with a non-zero
    /// exit code and is not recorded as a failed check.
    ///
    /// [`InputDriverPlugin`]: crate::InputDriverPlugin
    fn add_driver_check<Marker>(
        &mut self,
        name: impl Into<String>,
        check_system: impl IntoSystem<In<Value>, Result<Value, String>, Marker> + 'static,
    ) -> &mut Self;

    /// Registers a target that `click`, `mouse_move`, `touch`, `drag`, and `touch_drag` can aim
    /// at by `name`, such as `{"command":"click","at":{"target":"cell","column":3,"row":5}}`.
    ///
    /// The system receives every key of the `at` object except `target` as an `In<Value>` JSON
    /// object, and returns a logical window position (origin top left), or an error message. It
    /// runs right before its command's input is sent, so it sees the layout as earlier commands
    /// left it. On an error, or an unknown name, the command sends no input and the failure is
    /// recorded in `errors` and `targets_failed` of `drive/status`. Registering a name twice
    /// replaces the first target. Works before or after adding [`InputDriverPlugin`].
    ///
    /// [`InputDriverPlugin`]: crate::InputDriverPlugin
    fn add_driver_target<Marker>(
        &mut self,
        name: impl Into<String>,
        target_system: impl IntoSystem<In<Value>, Result<Vec2, String>, Marker> + 'static,
    ) -> &mut Self;

    /// Registers a BRP method served next to the `drive/*` methods, such as `game/score`.
    ///
    /// The handler receives the request params. Works before or after adding
    /// [`InputDriverPlugin`]. The methods are served only while a `RemotePlugin` exists: the
    /// driver's, when `enable_remote` is true, or one the game adds itself when it is false.
    /// Methods registered before any `RemoteMethods` exists are added when the driver's
    /// `RemotePlugin` builds or in `Startup`; if no `RemotePlugin` exists by then, a warning
    /// names them.
    ///
    /// Names starting with `drive/` are reserved: such a method is refused with an error log and
    /// not registered. A name that replaces an existing method, built in or from an earlier
    /// call, logs a warning.
    ///
    /// [`InputDriverPlugin`]: crate::InputDriverPlugin
    fn add_driver_remote_method<Marker>(
        &mut self,
        name: impl Into<String>,
        handler: impl IntoSystem<In<Option<Value>>, BrpResult, Marker> + 'static,
    ) -> &mut Self;
}

impl InputDriverAppExt for App {
    fn add_driver_check<Marker>(
        &mut self,
        name: impl Into<String>,
        check_system: impl IntoSystem<In<Value>, Result<Value, String>, Marker> + 'static,
    ) -> &mut Self {
        register_driver_check(self, name.into(), check_system);
        self
    }

    fn add_driver_target<Marker>(
        &mut self,
        name: impl Into<String>,
        target_system: impl IntoSystem<In<Value>, Result<Vec2, String>, Marker> + 'static,
    ) -> &mut Self {
        register_driver_target(self, name.into(), target_system);
        self
    }

    fn add_driver_remote_method<Marker>(
        &mut self,
        name: impl Into<String>,
        handler: impl IntoSystem<In<Option<Value>>, BrpResult, Marker> + 'static,
    ) -> &mut Self {
        register_remote_method(self, name.into(), handler);
        self
    }

    fn set_driver_state_handlers<SaveMarker, LoadMarker>(
        &mut self,
        save_system: impl IntoSystem<In<PathBuf>, Result<(), String>, SaveMarker> + 'static,
        load_system: impl IntoSystem<In<PathBuf>, Result<(), String>, LoadMarker> + 'static,
    ) -> &mut Self {
        let save = self.register_system(save_system);
        let load = self.register_system(load_system);
        self.insert_resource(DriverStateHandlers {
            save: Some(save),
            load: Some(load),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Resource, Default)]
    struct LastPath(Option<PathBuf>);

    fn remember_path(In(path): In<PathBuf>, mut last: ResMut<LastPath>) -> Result<(), String> {
        last.0 = Some(path);
        Ok(())
    }

    fn always_fail(In(_path): In<PathBuf>) -> Result<(), String> {
        Err("disk full".to_owned())
    }

    #[test]
    fn missing_handlers_name_the_fix() {
        let mut world = World::new();
        let problem = run_state_operation(&mut world, StateOperation::Save, "a.txt")
            .expect_err("no handler is registered");
        assert_eq!(
            problem,
            StateOperationError::NoHandler(StateOperation::Save)
        );
        assert_eq!(
            problem.to_string(),
            "no save system registered: call set_driver_state_handlers"
        );
    }

    #[test]
    fn runs_the_matching_handler_with_the_path() {
        let mut world = World::new();
        world.init_resource::<LastPath>();
        let handlers = DriverStateHandlers {
            save: Some(world.register_system(remember_path)),
            load: Some(world.register_system(always_fail)),
        };
        world.insert_resource(handlers);

        run_state_operation(&mut world, StateOperation::Save, "slot.txt").expect("save works");
        assert_eq!(
            world.resource::<LastPath>().0.as_deref(),
            Some(std::path::Path::new("slot.txt"))
        );
        assert_eq!(
            run_state_operation(&mut world, StateOperation::Load, "slot.txt"),
            Err(StateOperationError::HandlerFailed("disk full".to_owned()))
        );
    }
}
