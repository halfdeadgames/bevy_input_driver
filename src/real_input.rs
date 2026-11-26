//! Keeping the user's real mouse and keyboard out of the game while the driver is in control,
//! and keeping script runs off the user's screen.

use std::path::Path;

use bevy::{
    ecs::{
        message::{MessageUpdateSystems, Messages},
        system::SystemParam,
    },
    input::{
        gestures::{DoubleTapGesture, PanGesture, PinchGesture, RotationGesture},
        keyboard::KeyboardInput,
        mouse::{MouseButtonInput, MouseMotion, MouseWheel},
        touch::TouchInput,
    },
    log::info,
    math::IVec2,
    prelude::*,
    window::{CursorMoved, CursorOptions, PrimaryWindow, WindowPosition},
};

use crate::{script_file::script_path_from_arguments, status::DriverStatus};

const ON_SCREEN_ARGUMENT: &str = "--on-screen";

// Inside the 16-bit coordinate range Windows accepts for window positions, far from any monitor
// layout, and not Windows' own parking spot for minimized windows, which is (-32000, -32000).
const OFF_SCREEN_POSITION: IVec2 = IVec2::new(-30_000, -30_000);

/// Whether real keyboard and pointer input is blocked.
///
/// Blocking starts when a launch script loads or when the first live command arrives, and lasts
/// until the game exits or a `real_input` command turns it off.
#[derive(Resource, Debug, Default)]
pub struct RealInputBlocking {
    blocked: bool,
    automatic_start_used: bool,
}

impl RealInputBlocking {
    /// True while real input is blocked.
    #[must_use]
    pub fn is_blocked(&self) -> bool {
        self.blocked
    }

    /// Turns blocking on or off, logging when the state changes. An explicit choice also means
    /// the automatic start no longer applies.
    pub fn set_blocked(&mut self, blocked: bool) {
        self.automatic_start_used = true;
        if self.blocked != blocked {
            self.blocked = blocked;
            let state = if blocked { "on" } else { "off" };
            info!("input driver: blocking real input is {state}");
        }
    }

    /// Turns blocking on the first time it is called, and does nothing afterward, so a later
    /// `real_input` command that turned it off stays in force.
    pub fn block_for_first_command(&mut self) {
        if !self.automatic_start_used {
            self.set_blocked(true);
        }
    }
}

/// Every typed input message the driver injects and bevy_input reads.
#[derive(SystemParam)]
struct TypedInputMessages<'w> {
    keyboard: ResMut<'w, Messages<KeyboardInput>>,
    mouse_button: ResMut<'w, Messages<MouseButtonInput>>,
    mouse_wheel: ResMut<'w, Messages<MouseWheel>>,
    cursor_moved: ResMut<'w, Messages<CursorMoved>>,
    mouse_motion: ResMut<'w, Messages<MouseMotion>>,
    touch: ResMut<'w, Messages<TouchInput>>,
    pinch: ResMut<'w, Messages<PinchGesture>>,
    rotation: ResMut<'w, Messages<RotationGesture>>,
    pan: ResMut<'w, Messages<PanGesture>>,
    double_tap: ResMut<'w, Messages<DoubleTapGesture>>,
}

impl TypedInputMessages<'_> {
    fn clear_all(&mut self) {
        self.keyboard.clear();
        self.mouse_button.clear();
        self.mouse_wheel.clear();
        self.cursor_moved.clear();
        self.mouse_motion.clear();
        self.touch.clear();
        self.pinch.clear();
        self.rotation.clear();
        self.pan.clear();
        self.double_tap.clear();
    }
}

fn real_input_is_blocked(blocking: Res<RealInputBlocking>) -> bool {
    blocking.is_blocked()
}

// Real messages are written before `First`, injected ones in `PreUpdate`, and every reader of
// them (bevy_input, the bevy_input_focus dispatch after `InputSystems`, bevy_picking's touch
// cleanup in `Last`) runs in the frame of the injection, so dropping everything at the start of
// the next frame removes only real input. `Messages::clear` keeps the message count, so reader
// cursors stay valid. It runs after the buffer swap so both buffers are emptied in one pass.
// `WindowEvent` is left alone because `bevy_picking` reads last frame's injected ones in `First`.
fn clear_real_input_messages(mut messages: TypedInputMessages) {
    messages.clear_all();
}

// `CursorOptions::hit_test` is false so the operating system sends real pointer events to the
// window below. Writing only on a difference keeps `Changed<CursorOptions>` quiet otherwise.
fn apply_cursor_hit_test(
    blocking: Res<RealInputBlocking>,
    mut cursor_options: Query<&mut CursorOptions, With<PrimaryWindow>>,
) {
    let hit_test = !blocking.is_blocked();
    for mut options in &mut cursor_options {
        if options.hit_test != hit_test {
            options.hit_test = hit_test;
        }
    }
}

fn copy_blocking_into_status(blocking: Res<RealInputBlocking>, mut status: ResMut<DriverStatus>) {
    status.real_input_blocked = blocking.is_blocked();
}

/// Decides whether the primary window should start off every monitor.
///
/// True when the plugin allows it, a script path is given, and the process arguments do not
/// contain `--on-screen`.
#[must_use]
pub fn should_place_off_screen(
    enabled: bool,
    has_script: bool,
    arguments: impl IntoIterator<Item = String>,
) -> bool {
    enabled
        && has_script
        && !arguments
            .into_iter()
            .any(|argument| argument == ON_SCREEN_ARGUMENT)
}

/// Moves the primary window far off every monitor and marks it unfocused. Returns false when
/// there is no primary window, as in a headless app.
///
/// Call it before the operating system window exists so the window never flashes on screen.
/// The window is not minimized, because a minimized window has a 0 by 0 surface.
pub fn place_primary_window_off_screen(world: &mut World) -> bool {
    let mut primary_windows = world.query_filtered::<&mut Window, With<PrimaryWindow>>();
    let Ok(mut window) = primary_windows.single_mut(world) else {
        return false;
    };
    window.position = WindowPosition::At(OFF_SCREEN_POSITION);
    window.focused = false;
    true
}

/// Applies the off-screen start for script runs, as decided by [`should_place_off_screen`].
pub fn place_window_off_screen_for_scripts(
    world: &mut World,
    enabled: bool,
    plugin_script_path: Option<&Path>,
) {
    // A malformed `--input-script` is recorded as an error at startup, so here it counts as none.
    let argument_script = script_path_from_arguments(std::env::args().skip(1))
        .ok()
        .flatten();
    let has_script = argument_script.is_some() || plugin_script_path.is_some();
    if should_place_off_screen(enabled, has_script, std::env::args().skip(1))
        && place_primary_window_off_screen(world)
    {
        info!(
            "input driver: primary window placed off-screen at {OFF_SCREEN_POSITION} and \
             unfocused; pass {ON_SCREEN_ARGUMENT} to keep it on screen"
        );
    }
}

/// Registers the blocking state, the message clearing, and the pointer pass-through.
pub struct DriverRealInputPlugin;

impl Plugin for DriverRealInputPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RealInputBlocking>()
            .init_resource::<DriverStatus>()
            .add_message::<KeyboardInput>()
            .add_message::<MouseButtonInput>()
            .add_message::<MouseWheel>()
            .add_message::<CursorMoved>()
            .add_message::<MouseMotion>()
            .add_message::<TouchInput>()
            .add_message::<PinchGesture>()
            .add_message::<RotationGesture>()
            .add_message::<PanGesture>()
            .add_message::<DoubleTapGesture>()
            .add_systems(
                First,
                clear_real_input_messages
                    .after(MessageUpdateSystems)
                    .run_if(real_input_is_blocked),
            )
            .add_systems(
                PreUpdate,
                (
                    copy_blocking_into_status,
                    apply_cursor_hit_test.run_if(resource_changed::<RealInputBlocking>),
                ),
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::input::{
        ButtonState,
        keyboard::{Key, KeyCode},
    };

    fn app_with_real_input() -> App {
        let mut app = App::new();
        app.add_plugins(DriverRealInputPlugin);
        app
    }

    fn write_keyboard_and_motion(app: &mut App) {
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::KeyA,
            logical_key: Key::Character("a".into()),
            state: ButtonState::Pressed,
            text: None,
            repeat: false,
            window: Entity::PLACEHOLDER,
        });
        app.world_mut()
            .write_message(MouseMotion { delta: Vec2::X });
    }

    fn pending_counts(app: &App) -> (usize, usize) {
        (
            app.world().resource::<Messages<KeyboardInput>>().len(),
            app.world().resource::<Messages<MouseMotion>>().len(),
        )
    }

    #[test]
    fn messages_are_cleared_only_while_blocked() {
        let mut app = app_with_real_input();
        write_keyboard_and_motion(&mut app);
        app.update();
        assert_eq!(pending_counts(&app), (1, 1));

        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(true);
        app.update();
        assert_eq!(pending_counts(&app), (0, 0));

        write_keyboard_and_motion(&mut app);
        app.update();
        assert_eq!(pending_counts(&app), (0, 0));
    }

    fn hit_test_of(app: &App, window: Entity) -> Option<bool> {
        app.world()
            .get::<CursorOptions>(window)
            .map(|options| options.hit_test)
    }

    #[test]
    fn hit_test_follows_the_blocked_state_and_is_not_rewritten_while_steady() {
        let mut app = app_with_real_input();
        let window = app
            .world_mut()
            .spawn((Window::default(), PrimaryWindow))
            .id();
        app.update();
        assert_eq!(hit_test_of(&app, window), Some(true));

        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(true);
        app.update();
        assert_eq!(hit_test_of(&app, window), Some(false));

        if let Some(mut options) = app.world_mut().get_mut::<CursorOptions>(window) {
            options.hit_test = true;
        }
        app.update();
        assert_eq!(
            hit_test_of(&app, window),
            Some(true),
            "a steady state must not be written again"
        );

        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(false);
        app.update();
        assert_eq!(hit_test_of(&app, window), Some(true));
    }

    #[test]
    fn status_reports_the_blocked_state() {
        let mut app = app_with_real_input();
        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(true);
        app.update();
        assert!(app.world().resource::<DriverStatus>().real_input_blocked);
    }

    #[test]
    fn the_automatic_start_applies_once() {
        let mut blocking = RealInputBlocking::default();
        blocking.block_for_first_command();
        assert!(blocking.is_blocked());
        blocking.set_blocked(false);
        blocking.block_for_first_command();
        assert!(!blocking.is_blocked());
    }

    #[test]
    fn an_explicit_choice_beats_the_automatic_start() {
        let mut blocking = RealInputBlocking::default();
        blocking.set_blocked(false);
        blocking.block_for_first_command();
        assert!(!blocking.is_blocked());
    }

    fn arguments(values: &[&str]) -> Vec<String> {
        values.iter().map(|value| (*value).to_owned()).collect()
    }

    #[test]
    fn off_screen_needs_a_script_and_no_on_screen_argument() {
        assert!(should_place_off_screen(true, true, arguments(&[])));
        assert!(should_place_off_screen(
            true,
            true,
            arguments(&["--input-script", "a.jsonl"])
        ));
        assert!(!should_place_off_screen(true, false, arguments(&[])));
        assert!(!should_place_off_screen(false, true, arguments(&[])));
        assert!(!should_place_off_screen(
            true,
            true,
            arguments(&["--on-screen", "--input-script", "a.jsonl"])
        ));
    }

    #[test]
    fn placing_off_screen_moves_and_unfocuses_the_primary_window() {
        let mut world = World::new();
        assert!(!place_primary_window_off_screen(&mut world));
        let window = world.spawn((Window::default(), PrimaryWindow)).id();
        assert!(place_primary_window_off_screen(&mut world));
        let placed = world
            .get::<Window>(window)
            .map(|window| (window.position, window.focused));
        assert_eq!(
            placed,
            Some((WindowPosition::At(OFF_SCREEN_POSITION), false))
        );
    }
}
