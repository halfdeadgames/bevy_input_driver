//! Game-provided targets: named systems that turn JSON arguments into a logical window position,
//! so a script can say where to point by meaning (a board cell) instead of by pixel.

use std::{collections::BTreeMap, fmt};

use bevy::{ecs::system::SystemId, prelude::*};
use serde_json::Value;

use crate::{
    commands::{DriverCommand, DriverTarget},
    status::DriverStatus,
    steps::{DriverQueue, expand_command},
};

/// One-shot system id of a target that receives the script's arguments and returns a logical
/// window position.
pub type TargetSystemId = SystemId<In<Value>, Result<Vec2, String>>;

/// The registered targets by name. Fill it with [`InputDriverAppExt::add_driver_target`].
///
/// [`InputDriverAppExt::add_driver_target`]: crate::InputDriverAppExt::add_driver_target
#[derive(Resource, Debug, Default)]
pub struct DriverTargets {
    systems: BTreeMap<String, TargetSystemId>,
}

impl DriverTargets {
    /// Registers `system` under `name`, replacing any target with that name, and returns the
    /// replaced system so the caller can unregister it.
    pub fn insert(&mut self, name: String, system: TargetSystemId) -> Option<TargetSystemId> {
        self.systems.insert(name, system)
    }

    /// The system registered under `name`.
    #[must_use]
    pub fn get(&self, name: &str) -> Option<TargetSystemId> {
        self.systems.get(name).copied()
    }

    /// Every registered name, sorted.
    #[must_use]
    pub fn names(&self) -> Vec<String> {
        self.systems.keys().cloned().collect()
    }
}

/// Why a target did not give a position.
#[derive(Debug, PartialEq)]
pub enum TargetError {
    /// No target has this name.
    Unknown {
        /// The requested name.
        name: String,
        /// Every registered name, sorted.
        registered: Vec<String>,
    },
    /// Bevy could not run the registered system.
    CannotRun {
        /// Name of the target.
        name: String,
        /// What Bevy reported.
        problem: String,
    },
    /// The target ran and returned an error.
    Failed {
        /// Name of the target.
        name: String,
        /// The message the target returned.
        problem: String,
    },
    /// The target returned a position with a NaN or infinite coordinate.
    NotFinite {
        /// Name of the target.
        name: String,
        /// The position it returned.
        position: Vec2,
    },
}

impl fmt::Display for TargetError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unknown { name, registered } if registered.is_empty() => write!(
                formatter,
                "no target named \"{name}\"; registered targets: none (register one with \
                 add_driver_target)"
            ),
            Self::Unknown { name, registered } => write!(
                formatter,
                "no target named \"{name}\"; registered targets: {}",
                registered.join(", ")
            ),
            Self::CannotRun { name, problem } => {
                write!(formatter, "target \"{name}\" cannot run: {problem}")
            }
            Self::Failed { name, problem } => write!(formatter, "target \"{name}\": {problem}"),
            Self::NotFinite { name, position } => write!(
                formatter,
                "target \"{name}\" returned a position that is not finite: {position}"
            ),
        }
    }
}

impl std::error::Error for TargetError {}

/// Runs the target system named by `target` with its arguments.
///
/// A missing [`DriverTargets`] resource counts as no target being registered.
///
/// # Errors
///
/// Returns [`TargetError`] when no target has that name, Bevy cannot run it, it returns an error,
/// or its position is not finite.
pub fn resolve_target(world: &mut World, target: &DriverTarget) -> Result<Vec2, TargetError> {
    let name = &target.target;
    let registered = world.get_resource::<DriverTargets>();
    let Some(system) = registered.and_then(|targets| targets.get(name)) else {
        return Err(TargetError::Unknown {
            name: name.clone(),
            registered: registered.map(DriverTargets::names).unwrap_or_default(),
        });
    };
    let arguments = Value::Object(target.arguments.clone());
    let position = world
        .run_system_with::<In<Value>, Result<Vec2, String>>(system, arguments)
        .map_err(|problem| TargetError::CannotRun {
            name: name.clone(),
            problem: problem.to_string(),
        })?
        .map_err(|problem| TargetError::Failed {
            name: name.clone(),
            problem,
        })?;
    if position.is_finite() {
        Ok(position)
    } else {
        Err(TargetError::NotFinite {
            name: name.clone(),
            position,
        })
    }
}

/// Replaces every target in `command` with the position its system returns.
///
/// # Errors
///
/// Returns the first [`TargetError`] from [`resolve_target`].
pub fn resolve_command(
    world: &mut World,
    command: DriverCommand,
) -> Result<DriverCommand, TargetError> {
    command.resolve_targets(|target| resolve_target(world, target))
}

// A clear between queueing and running means the script was abandoned, so the command must not
// come back to life.
fn resolve_in_world(world: &mut World, command: DriverCommand, generation: u64) {
    let mut queue = world.resource_mut::<DriverQueue>();
    queue.finish_target_resolution();
    if queue.generation() != generation {
        return;
    }
    let name = command.name();
    match resolve_command(world, command) {
        Ok(resolved) => world
            .resource_mut::<DriverQueue>()
            .push_steps_to_front(expand_command(&resolved)),
        Err(problem) => world
            .resource_mut::<DriverStatus>()
            .note_target_failure(format!("{name}: {problem}")),
    }
}

/// Queues a world command that resolves the command's targets and puts its steps at the front of
/// the queue, or records the failure in [`DriverStatus`] and sends no input.
///
/// `generation` is [`DriverQueue::generation`] when this was queued; if the queue was cleared
/// since, nothing happens.
pub fn queue_target_resolution(commands: &mut Commands, command: DriverCommand, generation: u64) {
    commands.queue(move |world: &mut World| resolve_in_world(world, command, generation));
}

/// Stores the target under `name`, creating [`DriverTargets`] if it does not exist yet.
pub fn register_driver_target<Marker>(
    app: &mut App,
    name: String,
    target_system: impl IntoSystem<In<Value>, Result<Vec2, String>, Marker> + 'static,
) {
    let system = app.register_system(target_system);
    let replaced_system = app
        .world_mut()
        .get_resource_or_insert_with(DriverTargets::default)
        .insert(name, system);
    if let Some(replaced_system) = replaced_system {
        // Already gone only if the game unregistered it itself, which leaves nothing to clean up.
        let _already_unregistered = app.world_mut().unregister_system(replaced_system);
    }
}

/// Creates [`DriverTargets`] when no target was registered before the plugin was added.
pub struct DriverTargetsPlugin;

impl Plugin for DriverTargetsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DriverTargets>();
    }
}

#[cfg(test)]
mod tests {
    use bevy::{
        input::{
            gestures::{DoubleTapGesture, PanGesture, PinchGesture, RotationGesture},
            keyboard::KeyboardInput,
            mouse::{MouseButtonInput, MouseMotion, MouseWheel},
            touch::TouchInput,
        },
        window::{CursorMoved, PrimaryWindow, WindowEvent},
    };

    use super::*;
    use crate::{
        commands::parse_command_text, executor::execute_driver_steps,
        gamepad_injection::DriverGamepadPlugin, injection::DriverWindowEventPlugin,
        plugin::DriverSettings, real_input::RealInputBlocking,
    };

    fn cell_position(In(arguments): In<Value>) -> Result<Vec2, String> {
        let coordinate = |key: &str| {
            arguments
                .get(key)
                .and_then(Value::as_f64)
                .ok_or_else(|| format!("missing {key}"))
        };
        Ok(Vec2::new(
            coordinate("column")? as f32 * 10.0,
            coordinate("row")? as f32 * 10.0,
        ))
    }

    fn always_fails(In(_arguments): In<Value>) -> Result<Vec2, String> {
        Err("the board is not shown".to_owned())
    }

    /// Every cursor move seen, kept because messages expire after two frames.
    #[derive(Resource, Default)]
    struct RecordedCursorPositions(Vec<Vec2>);

    fn record_cursor_positions(
        mut cursor_moves: MessageReader<CursorMoved>,
        mut recorded: ResMut<RecordedCursorPositions>,
    ) {
        recorded
            .0
            .extend(cursor_moves.read().map(|moved| moved.position));
    }

    fn driver_app() -> App {
        let mut app = App::new();
        app.add_message::<CursorMoved>()
            .add_message::<MouseMotion>()
            .add_message::<KeyboardInput>()
            .add_message::<MouseButtonInput>()
            .add_message::<MouseWheel>()
            .add_message::<TouchInput>()
            .add_message::<PinchGesture>()
            .add_message::<RotationGesture>()
            .add_message::<PanGesture>()
            .add_message::<DoubleTapGesture>()
            .add_message::<WindowEvent>()
            .add_plugins((DriverGamepadPlugin, DriverWindowEventPlugin))
            .insert_resource(DriverSettings {
                port: 0,
                enable_remote: false,
                script_path: None,
                update_window_cursor_position: false,
                off_screen_for_scripts: false,
            })
            .init_resource::<DriverQueue>()
            .init_resource::<DriverStatus>()
            .init_resource::<RealInputBlocking>()
            .init_resource::<RecordedCursorPositions>()
            .add_systems(
                Update,
                (execute_driver_steps, record_cursor_positions).chain(),
            );
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        register_driver_target(&mut app, "cell".to_owned(), cell_position);
        register_driver_target(&mut app, "broken".to_owned(), always_fails);
        app
    }

    fn run_script(app: &mut App, lines: &[&str]) {
        let commands: Vec<DriverCommand> = lines
            .iter()
            .map(|line| parse_command_text(line).expect("command should parse"))
            .collect();
        app.world_mut()
            .resource_mut::<DriverQueue>()
            .enqueue_commands(&commands);
        for _ in 0..60 {
            app.update();
        }
    }

    fn cursor_positions(app: &App) -> Vec<Vec2> {
        app.world().resource::<RecordedCursorPositions>().0.clone()
    }

    #[test]
    fn registering_looking_up_and_replacing_targets() {
        let mut app = App::new();
        register_driver_target(&mut app, "cell".to_owned(), cell_position);
        let first = app.world().resource::<DriverTargets>().get("cell");
        assert!(first.is_some());
        assert_eq!(app.world().resource::<DriverTargets>().get("other"), None);

        register_driver_target(&mut app, "cell".to_owned(), always_fails);
        let second = app.world().resource::<DriverTargets>().get("cell");
        assert_ne!(first, second);
        assert_eq!(
            app.world().resource::<DriverTargets>().names(),
            vec!["cell".to_owned()]
        );
        let replaced = first.expect("the first target was registered");
        assert!(
            app.world_mut()
                .run_system_with::<In<Value>, Result<Vec2, String>>(replaced, Value::Null)
                .is_err()
        );
    }

    #[test]
    fn a_click_moves_the_cursor_to_the_target_position() {
        let mut app = driver_app();
        run_script(
            &mut app,
            &[r#"{"command":"click","at":{"target":"cell","column":3,"row":5}}"#],
        );
        let status = app.world().resource::<DriverStatus>();
        assert_eq!(status.cursor_position, Some([30.0, 50.0]));
        assert_eq!(status.targets_failed, 0);
        assert!(status.errors.is_empty());
    }

    #[test]
    fn a_drag_starts_and_ends_at_its_target_positions() {
        let mut app = driver_app();
        run_script(
            &mut app,
            &[
                r#"{"command":"drag","from":{"target":"cell","column":1,"row":1},"to":{"target":"cell","column":4,"row":2},"frames":2}"#,
            ],
        );
        let positions = cursor_positions(&app);
        assert_eq!(positions.first(), Some(&Vec2::new(10.0, 10.0)));
        assert_eq!(positions.last(), Some(&Vec2::new(40.0, 20.0)));
        assert_eq!(app.world().resource::<DriverStatus>().targets_failed, 0);
    }

    #[test]
    fn an_unregistered_name_and_an_erroring_system_fail_without_input() {
        let mut app = driver_app();
        run_script(
            &mut app,
            &[
                r#"{"command":"click","at":{"target":"nowhere"}}"#,
                r#"{"command":"touch_drag","from":[0,0],"to":{"target":"broken"}}"#,
                r#"{"command":"mouse_move","x":3,"y":4}"#,
            ],
        );
        let status = app.world().resource::<DriverStatus>();
        assert_eq!(status.targets_failed, 2);
        assert_eq!(status.errors.len(), 2);
        assert!(
            status.errors[0].starts_with("click: no target named \"nowhere\""),
            "{:?}",
            status.errors
        );
        assert_eq!(
            status.errors[1],
            "touch_drag: target \"broken\": the board is not shown"
        );
        assert_eq!(status.cursor_position, Some([3.0, 4.0]));
        assert!(status.held_touches.is_empty());
        assert!(
            app.world()
                .resource::<Messages<MouseButtonInput>>()
                .is_empty()
        );
        assert!(app.world().resource::<Messages<TouchInput>>().is_empty());
    }

    #[test]
    fn a_missing_registry_counts_as_no_target() {
        let mut world = World::new();
        let target = DriverTarget {
            target: "cell".to_owned(),
            arguments: serde_json::Map::new(),
        };
        let problem = resolve_target(&mut world, &target).expect_err("nothing is registered");
        assert!(matches!(problem, TargetError::Unknown { .. }));
        assert!(problem.to_string().contains("add_driver_target"));
    }

    #[test]
    fn a_clear_between_queueing_and_resolving_does_not_resurrect_steps() {
        let mut world = World::new();
        world.init_resource::<DriverStatus>();
        world.init_resource::<DriverQueue>();
        let mut targets = DriverTargets::default();
        targets.insert("cell".to_owned(), world.register_system(cell_position));
        world.insert_resource(targets);
        let command =
            parse_command_text(r#"{"command":"click","at":{"target":"cell","column":3,"row":5}}"#)
                .expect("command should parse");

        let generation = world.resource::<DriverQueue>().generation();
        queue_target_resolution(&mut world.commands(), command.clone(), generation);
        world.resource_mut::<DriverQueue>().clear_and_release_all();
        world.flush();
        assert_eq!(world.resource::<DriverQueue>().step_count(), 2);

        let generation = world.resource::<DriverQueue>().generation();
        queue_target_resolution(&mut world.commands(), command, generation);
        world.flush();
        assert_eq!(world.resource::<DriverQueue>().step_count(), 2 + 7);
    }
}
