//! Keeping the user's real mouse and keyboard out of the game while the driver is in control,
//! and keeping script runs off the user's screen.

use std::path::Path;

use bevy::{
    ecs::{
        message::{MessageUpdateSystems, Messages},
        system::{NonSendMarker, SystemParam},
    },
    input::{
        gamepad::{
            Gamepad, GamepadConnectionEvent, GamepadInput, RawGamepadAxisChangedEvent,
            RawGamepadButtonChangedEvent, RawGamepadEvent, gamepad_connection_system,
            gamepad_event_processing_system,
        },
        gestures::{DoubleTapGesture, PanGesture, PinchGesture, RotationGesture},
        keyboard::KeyboardInput,
        mouse::{MouseButtonInput, MouseMotion, MouseWheel},
        touch::TouchInput,
    },
    log::info,
    math::IVec2,
    prelude::*,
    window::{CursorMoved, CursorOptions, FileDragAndDrop, Ime, PrimaryWindow, WindowPosition},
};

use crate::{
    executor::winit_reports_focus, script_file::script_path_from_arguments, status::DriverStatus,
};

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

/// Typed input messages that only arrive while the operating system window has focus.
#[derive(SystemParam)]
struct FocusDependentMessages<'w> {
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

impl FocusDependentMessages<'_> {
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

/// Typed messages the operating system sends whatever the window focus, so a drag over the
/// window or an input method editor reaches the game even while it is unfocused.
#[derive(SystemParam)]
struct FocusIndependentMessages<'w> {
    ime: ResMut<'w, Messages<Ime>>,
    file_drag_and_drop: ResMut<'w, Messages<FileDragAndDrop>>,
}

impl FocusIndependentMessages<'_> {
    fn clear_all(&mut self) {
        self.ime.clear();
        self.file_drag_and_drop.clear();
    }
}

/// Marks a gamepad entity the driver owns, so blocking real input leaves its `Gamepad` alone.
#[derive(Component, Debug, Default, Clone, Copy)]
pub struct DriverGamepad;

// Holds the removed `Gamepad` itself, because its vendor and product ids cannot be set outside
// bevy_input. Restoring the same value also keeps the device's settings and state.
#[derive(Component)]
struct HiddenRealGamepad(Gamepad);

fn real_input_is_blocked(blocking: Res<RealInputBlocking>) -> bool {
    blocking.is_blocked()
}

/// Whether clearing is needed, given winit's answer for the primary window. With no answer
/// (tests, headless apps) clearing stays on.
fn should_clear_for_focus(operating_system_focus: Option<bool>) -> bool {
    operating_system_focus.unwrap_or(true)
}

// Asks winit instead of `Window::focused` so injected focus messages cannot switch clearing.
// `NonSendMarker` keeps this on the main thread, the only thread whose `WINIT_WINDOWS` is filled.
fn primary_window_has_operating_system_focus(
    primary_window: Query<Entity, With<PrimaryWindow>>,
    _main_thread: NonSendMarker,
) -> bool {
    let focus = primary_window.single().ok().and_then(winit_reports_focus);
    should_clear_for_focus(focus)
}

// `Messages` cannot drop single messages, so every typed message goes. Real keyboard input only
// arrives while the operating system window has focus and real pointer input is routed away by
// `hit_test`, so clearing those runs only then; that spares injected messages otherwise. Ime and
// file drag messages do not depend on keyboard focus, so `clear_focus_independent_messages`
// clears them whenever input is blocked. Real
// messages are written before `First`, injected ones in `PreUpdate`, and their readers run in
// the injection frame, so dropping everything at the start of the next frame removes only real
// input. `Messages::clear` keeps the message count, so reader cursors stay valid. It runs after
// the buffer swap so both buffers are emptied in one pass. `WindowEvent` is left alone because
// `bevy_picking` reads last frame's injected ones in `First`.
fn clear_real_input_messages(mut messages: FocusDependentMessages) {
    messages.clear_all();
}

fn clear_focus_independent_messages(mut messages: FocusIndependentMessages) {
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

// bevy_gilrs writes real gamepad events in `PreUpdate` whatever the window focus, and bevy_input
// ignores events for entities without `Gamepad`. Hiding runs between connection handling and
// event processing, so a pad that connects while blocked is hidden the same frame. Exclusive
// access applies the change immediately, so no sync point is needed.
type RealGamepadFilter = (With<Gamepad>, Without<DriverGamepad>);

fn hide_real_gamepads_while_blocked(
    world: &mut World,
    mut real_gamepads: Local<QueryState<Entity, RealGamepadFilter>>,
) {
    let entities: Vec<Entity> = real_gamepads.iter(world).collect();
    for entity in &entities {
        let mut entity_mut = world.entity_mut(*entity);
        if let Some(gamepad) = entity_mut.take::<Gamepad>() {
            entity_mut.insert(HiddenRealGamepad(gamepad));
        }
    }
    if !entities.is_empty() {
        info!("input driver: hid {} real gamepad(s)", entities.len());
    }
}

// The device may have been moved while hidden, and the stored `Gamepad` still holds the values
// from before, so every nonzero input is zeroed. The device reports fresh values on its next
// change.
fn zero_events_for_held_inputs(entity: Entity, gamepad: &Gamepad) -> Vec<RawGamepadEvent> {
    gamepad
        .get_analog_axes()
        .filter(|input| {
            gamepad
                .get_unclamped(**input)
                .is_some_and(|value| value != 0.0)
        })
        .map(|input| match input {
            GamepadInput::Button(button) => {
                RawGamepadEvent::Button(RawGamepadButtonChangedEvent::new(entity, *button, 0.0))
            }
            GamepadInput::Axis(axis) => {
                RawGamepadEvent::Axis(RawGamepadAxisChangedEvent::new(entity, *axis, 0.0))
            }
        })
        .collect()
}

fn restore_gamepad(world: &mut World, entity: Entity) -> Vec<RawGamepadEvent> {
    let mut entity_mut = world.entity_mut(entity);
    let Some(HiddenRealGamepad(gamepad)) = entity_mut.take::<HiddenRealGamepad>() else {
        return Vec::new();
    };
    let events = zero_events_for_held_inputs(entity, &gamepad);
    entity_mut.insert(gamepad);
    events
}

fn restore_real_gamepads_when_unblocked(
    world: &mut World,
    mut hidden_gamepads: Local<QueryState<Entity, With<HiddenRealGamepad>>>,
) {
    let entities: Vec<Entity> = hidden_gamepads.iter(world).collect();
    for entity in &entities {
        let events = restore_gamepad(world, *entity);
        let mut messages = world.resource_mut::<Messages<RawGamepadEvent>>();
        for event in events {
            messages.write(event);
        }
    }
    if !entities.is_empty() {
        info!("input driver: restored {} real gamepad(s)", entities.len());
    }
}

// A device that disconnected while hidden must not come back.
fn forget_disconnected_hidden_gamepads(
    mut connection_events: MessageReader<GamepadConnectionEvent>,
    mut commands: Commands,
) {
    for event in connection_events.read() {
        if event.disconnected() {
            commands
                .entity(event.gamepad)
                .try_remove::<HiddenRealGamepad>();
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
            .add_message::<Ime>()
            .add_message::<FileDragAndDrop>()
            .add_message::<GamepadConnectionEvent>()
            .add_message::<RawGamepadEvent>()
            .add_systems(
                First,
                (
                    clear_real_input_messages.run_if(primary_window_has_operating_system_focus),
                    clear_focus_independent_messages,
                )
                    .after(MessageUpdateSystems)
                    .run_if(real_input_is_blocked),
            )
            .add_systems(
                PreUpdate,
                (
                    forget_disconnected_hidden_gamepads,
                    hide_real_gamepads_while_blocked
                        .run_if(real_input_is_blocked)
                        .after(forget_disconnected_hidden_gamepads)
                        .after(gamepad_connection_system)
                        .before(gamepad_event_processing_system),
                    restore_real_gamepads_when_unblocked
                        .run_if(not(real_input_is_blocked))
                        .after(forget_disconnected_hidden_gamepads)
                        .before(gamepad_event_processing_system),
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
        gamepad::{
            GamepadButton, GamepadButtonStateChangedEvent, GamepadConnection,
            RawGamepadButtonChangedEvent, RawGamepadEvent,
        },
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

    #[test]
    fn clearing_follows_the_operating_system_focus() {
        assert!(should_clear_for_focus(None));
        assert!(should_clear_for_focus(Some(true)));
        assert!(!should_clear_for_focus(Some(false)));
    }

    fn app_with_gamepad_input() -> App {
        let mut app = App::new();
        app.add_plugins((bevy::input::InputPlugin, DriverRealInputPlugin));
        app
    }

    fn connect_gamepad(app: &mut App) -> Entity {
        let entity = app.world_mut().spawn_empty().id();
        app.world_mut().write_message(GamepadConnectionEvent::new(
            entity,
            GamepadConnection::Connected {
                name: "test pad".into(),
                vendor_id: Some(1),
                product_id: Some(2),
            },
        ));
        app.update();
        entity
    }

    fn press_south(app: &mut App, entity: Entity) {
        app.world_mut()
            .write_message(RawGamepadEvent::Button(RawGamepadButtonChangedEvent::new(
                entity,
                GamepadButton::South,
                1.0,
            )));
    }

    fn state_change_count(app: &App) -> usize {
        app.world()
            .resource::<Messages<GamepadButtonStateChangedEvent>>()
            .len()
    }

    #[test]
    fn real_gamepads_are_hidden_while_blocked_and_restored_after() {
        let mut app = app_with_gamepad_input();
        let entity = connect_gamepad(&mut app);
        assert!(app.world().get::<Gamepad>(entity).is_some());

        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(true);
        press_south(&mut app, entity);
        app.update();
        assert!(app.world().get::<Gamepad>(entity).is_none());
        assert_eq!(state_change_count(&app), 0);

        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(false);
        app.update();
        let restored = app.world().get::<Gamepad>(entity);
        assert_eq!(
            restored.map(|gamepad| (gamepad.vendor_id(), gamepad.product_id())),
            Some((Some(1), Some(2)))
        );
    }

    #[test]
    fn a_button_held_while_hiding_reads_released_after_restore() {
        let mut app = app_with_gamepad_input();
        let entity = connect_gamepad(&mut app);
        press_south(&mut app, entity);
        app.update();
        let pressed = |app: &App| {
            app.world()
                .get::<Gamepad>(entity)
                .map(|gamepad| gamepad.pressed(GamepadButton::South))
        };
        assert_eq!(pressed(&app), Some(true));

        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(true);
        app.update();
        assert!(app.world().get::<Gamepad>(entity).is_none());

        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(false);
        app.update();
        assert_eq!(pressed(&app), Some(false));
    }

    #[test]
    fn ime_and_file_messages_are_cleared_while_blocked() {
        let mut app = app_with_real_input();
        app.world_mut().write_message(Ime::Disabled {
            window: Entity::PLACEHOLDER,
        });
        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(true);
        app.update();
        assert_eq!(app.world().resource::<Messages<Ime>>().len(), 0);
    }

    #[test]
    fn a_gamepad_that_disconnects_while_hidden_is_not_restored() {
        let mut app = app_with_gamepad_input();
        let entity = connect_gamepad(&mut app);
        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(true);
        app.update();
        app.world_mut().write_message(GamepadConnectionEvent::new(
            entity,
            GamepadConnection::Disconnected,
        ));
        app.update();
        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(false);
        app.update();
        assert!(app.world().get::<Gamepad>(entity).is_none());
    }

    #[test]
    fn driver_gamepads_keep_their_gamepad_while_blocked() {
        let mut app = app_with_gamepad_input();
        let entity = app
            .world_mut()
            .spawn((Gamepad::default(), DriverGamepad))
            .id();
        app.world_mut()
            .resource_mut::<RealInputBlocking>()
            .set_blocked(true);
        press_south(&mut app, entity);
        app.update();
        assert!(app.world().get::<Gamepad>(entity).is_some());
        assert_eq!(state_change_count(&app), 1);
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
