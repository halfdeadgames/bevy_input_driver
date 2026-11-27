//! Fake gamepads the driver connects, presses, and moves.
//!
//! Each driver gamepad is an entity carrying [`DriverGamepad`]. The driver writes the messages
//! `bevy_gilrs` writes for a real device, so `bevy_input` turns them into a `Gamepad` component
//! and the game cannot tell the difference.

use std::{
    collections::{HashMap, HashSet},
    fmt,
};

use bevy::{
    ecs::system::SystemParam,
    input::gamepad::{
        GamepadAxis, GamepadButton, GamepadConnection, GamepadConnectionEvent,
        RawGamepadAxisChangedEvent, RawGamepadButtonChangedEvent, RawGamepadEvent,
    },
    prelude::*,
};
use serde::{Deserialize, Deserializer, de::Error as _};

use crate::real_input::DriverGamepad;

const BUTTON_NAMES: [(&str, GamepadButton); 19] = [
    ("south", GamepadButton::South),
    ("east", GamepadButton::East),
    ("north", GamepadButton::North),
    ("west", GamepadButton::West),
    ("c", GamepadButton::C),
    ("z", GamepadButton::Z),
    ("left_trigger", GamepadButton::LeftTrigger),
    ("left_trigger2", GamepadButton::LeftTrigger2),
    ("right_trigger", GamepadButton::RightTrigger),
    ("right_trigger2", GamepadButton::RightTrigger2),
    ("select", GamepadButton::Select),
    ("start", GamepadButton::Start),
    ("mode", GamepadButton::Mode),
    ("left_thumb", GamepadButton::LeftThumb),
    ("right_thumb", GamepadButton::RightThumb),
    ("d_pad_up", GamepadButton::DPadUp),
    ("d_pad_down", GamepadButton::DPadDown),
    ("d_pad_left", GamepadButton::DPadLeft),
    ("d_pad_right", GamepadButton::DPadRight),
];

const AXIS_NAMES: [(&str, GamepadAxis); 6] = [
    ("left_stick_x", GamepadAxis::LeftStickX),
    ("left_stick_y", GamepadAxis::LeftStickY),
    ("left_z", GamepadAxis::LeftZ),
    ("right_stick_x", GamepadAxis::RightStickX),
    ("right_stick_y", GamepadAxis::RightStickY),
    ("right_z", GamepadAxis::RightZ),
];

/// Number a script uses to name one of its fake gamepads.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Deserialize)]
#[serde(transparent)]
pub struct GamepadSlot(pub u8);

impl fmt::Display for GamepadSlot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}", self.0)
    }
}

fn look_up_name<Item: Copy>(
    table: &[(&str, Item)],
    kind: &str,
    name: &str,
) -> Result<Item, String> {
    table
        .iter()
        .find(|(candidate, _)| *candidate == name)
        .map(|(_, value)| *value)
        .ok_or_else(|| {
            let allowed: Vec<&str> = table.iter().map(|(candidate, _)| *candidate).collect();
            format!(
                "unknown gamepad {kind} \"{name}\"; allowed: {}",
                allowed.join(", ")
            )
        })
}

/// Reads a gamepad button by its snake_case name, such as `south` or `d_pad_up`.
///
/// # Errors
///
/// Returns a deserializer error listing every allowed name when the name is unknown.
pub fn deserialize_gamepad_button<'de, DeserializerType>(
    deserializer: DeserializerType,
) -> Result<GamepadButton, DeserializerType::Error>
where
    DeserializerType: Deserializer<'de>,
{
    let name = String::deserialize(deserializer)?;
    look_up_name(&BUTTON_NAMES, "button", &name).map_err(DeserializerType::Error::custom)
}

/// Reads a gamepad axis by its snake_case name, such as `left_stick_x`.
///
/// # Errors
///
/// Returns a deserializer error listing every allowed name when the name is unknown.
pub fn deserialize_gamepad_axis<'de, DeserializerType>(
    deserializer: DeserializerType,
) -> Result<GamepadAxis, DeserializerType::Error>
where
    DeserializerType: Deserializer<'de>,
{
    let name = String::deserialize(deserializer)?;
    look_up_name(&AXIS_NAMES, "axis", &name).map_err(DeserializerType::Error::custom)
}

/// One change to a fake gamepad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum GamepadAction {
    /// Connect the gamepad, creating its entity the first time.
    Connect,
    /// Disconnect the gamepad but keep its entity.
    Disconnect,
    /// Set a button's analog value.
    Button {
        /// Button to change.
        button: GamepadButton,
        /// Value from 0.0 (released) to 1.0 (pressed).
        value: f32,
    },
    /// Set an axis position.
    Axis {
        /// Axis to change.
        axis: GamepadAxis,
        /// Position from -1.0 to 1.0.
        value: f32,
    },
}

impl GamepadAction {
    /// The command's name as written in scripts, for messages.
    #[must_use]
    pub fn command_name(&self) -> &'static str {
        match self {
            Self::Connect => "gamepad_connect",
            Self::Disconnect => "gamepad_disconnect",
            Self::Button { .. } => "gamepad_button",
            Self::Axis { .. } => "gamepad_axis",
        }
    }
}

/// Why a gamepad change could not be injected.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GamepadInjectionError {
    /// The slot is already connected.
    AlreadyConnected(GamepadSlot),
    /// The slot was connected before but is disconnected now.
    NotConnected(GamepadSlot),
    /// No gamepad was ever connected in the slot.
    NeverConnected(GamepadSlot),
}

impl fmt::Display for GamepadInjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::AlreadyConnected(slot) => {
                write!(formatter, "gamepad {slot} is already connected")
            }
            Self::NotConnected(slot) => write!(formatter, "gamepad {slot} is not connected"),
            Self::NeverConnected(slot) => {
                write!(formatter, "gamepad {slot} was never connected")
            }
        }
    }
}

impl std::error::Error for GamepadInjectionError {}

#[derive(Debug, Clone)]
struct SlotState {
    entity: Entity,
    connected: bool,
    held_buttons: HashSet<GamepadButton>,
    held_axes: HashSet<GamepadAxis>,
}

impl SlotState {
    fn track_button(&mut self, button: GamepadButton, value: f32) {
        if value == 0.0 {
            self.held_buttons.remove(&button);
        } else {
            self.held_buttons.insert(button);
        }
    }

    fn track_axis(&mut self, axis: GamepadAxis, value: f32) {
        if value == 0.0 {
            self.held_axes.remove(&axis);
        } else {
            self.held_axes.insert(axis);
        }
    }
}

/// The nonzero buttons and axes of one connected fake gamepad.
struct HeldGamepadInput {
    entity: Entity,
    buttons: Vec<GamepadButton>,
    axes: Vec<GamepadAxis>,
}

/// The entity behind each fake gamepad slot, and whether the slot is connected.
///
/// An entity stays in the map after a disconnect, so a reconnect reuses it as Bevy does for real
/// devices.
#[derive(Resource, Debug, Default)]
pub struct DriverGamepads {
    slots: HashMap<GamepadSlot, SlotState>,
}

impl DriverGamepads {
    /// The entity of a connected slot, or none when the slot is unknown or disconnected.
    #[must_use]
    pub fn connected_entity(&self, slot: GamepadSlot) -> Option<Entity> {
        self.slots
            .get(&slot)
            .filter(|state| state.connected)
            .map(|state| state.entity)
    }

    fn require_connected(
        &mut self,
        slot: GamepadSlot,
    ) -> Result<&mut SlotState, GamepadInjectionError> {
        match self.slots.get_mut(&slot) {
            Some(state) if state.connected => Ok(state),
            Some(_) => Err(GamepadInjectionError::NotConnected(slot)),
            None => Err(GamepadInjectionError::NeverConnected(slot)),
        }
    }

    // A disconnect or reconnect starts with nothing held, because Bevy drops the `Gamepad`.
    fn set_connected(&mut self, slot: GamepadSlot, entity: Entity, connected: bool) {
        self.slots.insert(
            slot,
            SlotState {
                entity,
                connected,
                held_buttons: HashSet::new(),
                held_axes: HashSet::new(),
            },
        );
    }

    fn take_held_input(&mut self) -> Vec<HeldGamepadInput> {
        self.slots
            .values_mut()
            .filter(|state| !state.held_buttons.is_empty() || !state.held_axes.is_empty())
            .map(|state| HeldGamepadInput {
                entity: state.entity,
                buttons: state.held_buttons.drain().collect(),
                axes: state.held_axes.drain().collect(),
            })
            .collect()
    }
}

fn gamepad_name(slot: GamepadSlot) -> String {
    format!("Input driver gamepad {slot}")
}

/// System parameter that writes injected gamepad messages.
#[derive(SystemParam)]
pub struct GamepadInjector<'w> {
    gamepads: ResMut<'w, DriverGamepads>,
    raw_events: MessageWriter<'w, RawGamepadEvent>,
    connection_events: MessageWriter<'w, GamepadConnectionEvent>,
    raw_button_events: MessageWriter<'w, RawGamepadButtonChangedEvent>,
    raw_axis_events: MessageWriter<'w, RawGamepadAxisChangedEvent>,
}

impl GamepadInjector<'_> {
    /// Applies one change to the gamepad in `slot`.
    ///
    /// # Errors
    ///
    /// Returns [`GamepadInjectionError`] when the slot's connection state does not allow the
    /// change.
    pub fn apply(
        &mut self,
        slot: GamepadSlot,
        action: GamepadAction,
        commands: &mut Commands,
    ) -> Result<(), GamepadInjectionError> {
        match action {
            GamepadAction::Connect => self.connect(slot, commands),
            GamepadAction::Disconnect => self.disconnect(slot),
            GamepadAction::Button { button, value } => self.change_button(slot, button, value),
            GamepadAction::Axis { axis, value } => self.change_axis(slot, axis, value),
        }
    }

    fn connect(
        &mut self,
        slot: GamepadSlot,
        commands: &mut Commands,
    ) -> Result<(), GamepadInjectionError> {
        let entity = match self.gamepads.slots.get(&slot) {
            Some(state) if state.connected => {
                return Err(GamepadInjectionError::AlreadyConnected(slot));
            }
            Some(state) => state.entity,
            None => commands
                .spawn((Name::new(gamepad_name(slot)), DriverGamepad))
                .id(),
        };
        self.gamepads.set_connected(slot, entity, true);
        self.write_connection(
            entity,
            GamepadConnection::Connected {
                name: gamepad_name(slot),
                vendor_id: None,
                product_id: None,
            },
        );
        Ok(())
    }

    fn disconnect(&mut self, slot: GamepadSlot) -> Result<(), GamepadInjectionError> {
        let entity = self.gamepads.require_connected(slot)?.entity;
        self.gamepads.set_connected(slot, entity, false);
        self.write_connection(entity, GamepadConnection::Disconnected);
        Ok(())
    }

    fn change_button(
        &mut self,
        slot: GamepadSlot,
        button: GamepadButton,
        value: f32,
    ) -> Result<(), GamepadInjectionError> {
        let state = self.gamepads.require_connected(slot)?;
        state.track_button(button, value);
        let entity = state.entity;
        self.write_button(entity, button, value);
        Ok(())
    }

    fn change_axis(
        &mut self,
        slot: GamepadSlot,
        axis: GamepadAxis,
        value: f32,
    ) -> Result<(), GamepadInjectionError> {
        let state = self.gamepads.require_connected(slot)?;
        state.track_axis(axis, value);
        let entity = state.entity;
        self.write_axis(entity, axis, value);
        Ok(())
    }

    /// Sets every nonzero button and axis of every connected fake gamepad back to 0.0 and forgets
    /// them, so `release_all` leaves no fake input held.
    pub fn release_all(&mut self) {
        for held in self.gamepads.take_held_input() {
            for button in held.buttons {
                self.write_button(held.entity, button, 0.0);
            }
            for axis in held.axes {
                self.write_axis(held.entity, axis, 0.0);
            }
        }
    }

    fn write_button(&mut self, entity: Entity, button: GamepadButton, value: f32) {
        let event = RawGamepadButtonChangedEvent::new(entity, button, value);
        self.raw_events.write(event.into());
        self.raw_button_events.write(event);
    }

    fn write_axis(&mut self, entity: Entity, axis: GamepadAxis, value: f32) {
        let event = RawGamepadAxisChangedEvent::new(entity, axis, value);
        self.raw_events.write(event.into());
        self.raw_axis_events.write(event);
    }

    fn write_connection(&mut self, entity: Entity, connection: GamepadConnection) {
        let event = GamepadConnectionEvent::new(entity, connection);
        self.raw_events.write(event.clone().into());
        self.connection_events.write(event);
    }
}

/// Registers the slot map and the gamepad messages the injector writes.
pub struct DriverGamepadPlugin;

impl Plugin for DriverGamepadPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DriverGamepads>()
            .add_message::<RawGamepadEvent>()
            .add_message::<GamepadConnectionEvent>()
            .add_message::<RawGamepadButtonChangedEvent>()
            .add_message::<RawGamepadAxisChangedEvent>();
    }
}

#[cfg(test)]
mod tests {
    use bevy::{
        input::{InputPlugin, gamepad::Gamepad},
        window::{PrimaryWindow, WindowEvent},
    };

    use super::*;
    use crate::{
        commands::parse_command_text, executor::DriverExecutorPlugin,
        injection::DriverWindowEventPlugin, plugin::DriverSettings,
        real_input::DriverRealInputPlugin, status::DriverStatus, steps::DriverQueue,
    };

    const SLOT: GamepadSlot = GamepadSlot(0);

    fn gamepad_app() -> App {
        let mut app = App::new();
        app.add_plugins((
            InputPlugin,
            DriverRealInputPlugin,
            DriverExecutorPlugin,
            DriverGamepadPlugin,
            DriverWindowEventPlugin,
        ))
        .add_message::<WindowEvent>()
        .insert_resource(DriverSettings {
            port: 0,
            enable_remote: false,
            script_path: None,
            update_window_cursor_position: false,
            off_screen_for_scripts: false,
        });
        app.world_mut().spawn((Window::default(), PrimaryWindow));
        app
    }

    // Every command ends its frame, so one update runs exactly one command.
    fn run_command(app: &mut App, text: &str) {
        let command = parse_command_text(text).expect("command should parse");
        app.world_mut()
            .resource_mut::<DriverQueue>()
            .enqueue_commands(&[command]);
        app.update();
    }

    fn connect_slot_zero(app: &mut App) -> Entity {
        run_command(app, r#"{"command":"gamepad_connect","gamepad":0}"#);
        app.world()
            .resource::<DriverGamepads>()
            .connected_entity(SLOT)
            .expect("slot 0 should be connected")
    }

    fn recorded_errors(app: &App) -> Vec<String> {
        app.world().resource::<DriverStatus>().errors.clone()
    }

    #[test]
    fn connecting_creates_a_named_gamepad_entity() {
        let mut app = gamepad_app();
        let entity = connect_slot_zero(&mut app);
        assert!(app.world().get::<Gamepad>(entity).is_some());
        assert!(app.world().get::<DriverGamepad>(entity).is_some());
        assert_eq!(
            app.world().get::<Name>(entity).map(Name::as_str),
            Some("Input driver gamepad 0")
        );
    }

    #[test]
    fn button_presses_and_releases() {
        let mut app = gamepad_app();
        let entity = connect_slot_zero(&mut app);
        run_command(
            &mut app,
            r#"{"command":"gamepad_button","gamepad":0,"button":"south","value":1.0}"#,
        );
        let pressed = app
            .world()
            .get::<Gamepad>(entity)
            .map(|gamepad| gamepad.pressed(GamepadButton::South));
        assert_eq!(pressed, Some(true));

        run_command(
            &mut app,
            r#"{"command":"gamepad_button","gamepad":0,"button":"south","value":0.0}"#,
        );
        let pressed = app
            .world()
            .get::<Gamepad>(entity)
            .map(|gamepad| gamepad.pressed(GamepadButton::South));
        assert_eq!(pressed, Some(false));
        assert!(recorded_errors(&app).is_empty());
    }

    // -0.5 is outside the default dead zone of 0.05 and moves more than the default change
    // threshold, so the default filter keeps it and `Gamepad::get` returns it unscaled.
    #[test]
    fn axis_moves_past_the_default_filter() {
        let mut app = gamepad_app();
        let entity = connect_slot_zero(&mut app);
        run_command(
            &mut app,
            r#"{"command":"gamepad_axis","gamepad":0,"axis":"left_stick_x","value":-0.5}"#,
        );
        let position = app
            .world()
            .get::<Gamepad>(entity)
            .and_then(|gamepad| gamepad.get(GamepadAxis::LeftStickX))
            .expect("the axis should have a value");
        assert!((position + 0.5).abs() < 0.01, "{position}");
    }

    #[test]
    fn release_all_zeroes_held_gamepad_buttons_and_axes() {
        let mut app = gamepad_app();
        let entity = connect_slot_zero(&mut app);
        run_command(
            &mut app,
            r#"{"command":"gamepad_button","gamepad":0,"button":"south","value":1.0}"#,
        );
        run_command(
            &mut app,
            r#"{"command":"gamepad_axis","gamepad":0,"axis":"left_stick_x","value":-0.5}"#,
        );
        run_command(&mut app, r#"{"command":"release_all"}"#);

        let gamepad = app.world().get::<Gamepad>(entity).expect("gamepad exists");
        assert!(!gamepad.pressed(GamepadButton::South));
        let position = gamepad.get(GamepadAxis::LeftStickX).unwrap_or_default();
        assert!(position.abs() < 0.01, "{position}");
        assert!(
            app.world_mut()
                .resource_mut::<DriverGamepads>()
                .take_held_input()
                .is_empty()
        );
        assert!(recorded_errors(&app).is_empty());
    }

    #[test]
    fn a_button_set_back_to_zero_is_no_longer_held() {
        let mut app = gamepad_app();
        connect_slot_zero(&mut app);
        for value in ["1.0", "0.0"] {
            let text = format!(
                r#"{{"command":"gamepad_button","gamepad":0,"button":"south","value":{value}}}"#
            );
            run_command(&mut app, &text);
        }
        assert!(
            app.world_mut()
                .resource_mut::<DriverGamepads>()
                .take_held_input()
                .is_empty()
        );
    }

    #[test]
    fn disconnect_removes_gamepad_and_reconnect_reuses_the_entity() {
        let mut app = gamepad_app();
        let entity = connect_slot_zero(&mut app);
        run_command(&mut app, r#"{"command":"gamepad_disconnect","gamepad":0}"#);
        assert!(app.world().get::<Gamepad>(entity).is_none());
        assert!(app.world().get_entity(entity).is_ok());
        assert_eq!(
            app.world()
                .resource::<DriverGamepads>()
                .connected_entity(SLOT),
            None
        );

        assert_eq!(connect_slot_zero(&mut app), entity);
        assert!(app.world().get::<Gamepad>(entity).is_some());
        assert!(recorded_errors(&app).is_empty());
    }

    #[test]
    fn the_driver_gamepad_works_while_real_input_is_blocked() {
        let mut app = gamepad_app();
        run_command(&mut app, r#"{"command":"real_input","blocked":true}"#);
        let entity = connect_slot_zero(&mut app);
        run_command(
            &mut app,
            r#"{"command":"gamepad_button","gamepad":0,"button":"east","value":1.0}"#,
        );
        let pressed = app
            .world()
            .get::<Gamepad>(entity)
            .map(|gamepad| gamepad.pressed(GamepadButton::East));
        assert_eq!(pressed, Some(true));
    }

    #[test]
    fn state_errors_are_recorded_exactly() {
        let mut app = gamepad_app();
        run_command(
            &mut app,
            r#"{"command":"gamepad_button","gamepad":0,"button":"south","value":1}"#,
        );
        run_command(&mut app, r#"{"command":"gamepad_disconnect","gamepad":0}"#);
        connect_slot_zero(&mut app);
        run_command(&mut app, r#"{"command":"gamepad_connect","gamepad":0}"#);
        run_command(&mut app, r#"{"command":"gamepad_disconnect","gamepad":0}"#);
        run_command(&mut app, r#"{"command":"gamepad_disconnect","gamepad":0}"#);
        assert_eq!(
            recorded_errors(&app),
            vec![
                "gamepad_button: gamepad 0 was never connected".to_owned(),
                "gamepad_disconnect: gamepad 0 was never connected".to_owned(),
                "gamepad_connect: gamepad 0 is already connected".to_owned(),
                "gamepad_disconnect: gamepad 0 is not connected".to_owned(),
            ]
        );
    }

    #[test]
    fn unknown_names_list_the_allowed_ones() {
        let message = parse_command_text(
            r#"{"command":"gamepad_button","gamepad":0,"button":"cross","value":1}"#,
        )
        .expect_err("an unknown button should fail")
        .to_string();
        assert!(
            message.contains("cross") && message.contains("d_pad_up"),
            "{message}"
        );
        let message =
            parse_command_text(r#"{"command":"gamepad_axis","gamepad":0,"axis":"tilt","value":0}"#)
                .expect_err("an unknown axis should fail")
                .to_string();
        assert!(
            message.contains("tilt") && message.contains("right_z"),
            "{message}"
        );
    }
}
