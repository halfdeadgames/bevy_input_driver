//! Writing injected input as the messages the windowing backend would write.
//!
//! Every input is written twice: as its specific message, which `bevy_input` reads, and as the
//! matching `WindowEvent`, which `bevy_picking` reads. This mirrors `bevy_winit`.

use std::fmt;

use bevy::{
    ecs::{change_detection::DetectChangesMut, system::SystemParam},
    input::{
        ButtonState,
        gestures::{DoubleTapGesture, PanGesture, PinchGesture, RotationGesture},
        keyboard::{Key, KeyCode, KeyboardInput, NativeKey},
        mouse::{MouseButton, MouseButtonInput, MouseMotion, MouseScrollUnit, MouseWheel},
        touch::{TouchInput, TouchPhase},
    },
    math::Vec2,
    prelude::*,
    window::{CursorMoved, PrimaryWindow, WindowEvent},
};

use crate::steps::character_for_key_code;

/// Why an input could not be injected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InjectionError {
    /// The app has no `PrimaryWindow` entity to target.
    NoPrimaryWindow,
}

impl fmt::Display for InjectionError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoPrimaryWindow => {
                write!(
                    formatter,
                    "no primary window exists, so input was not injected"
                )
            }
        }
    }
}

impl std::error::Error for InjectionError {}

/// Derives the logical key a physical key produces.
///
/// Named keys map to their named variant. Letters and digits become `Key::Character` of `text`,
/// or of the unshifted character when `text` is absent. Any other key with `text` becomes
/// `Key::Character`, and the rest are unidentified.
#[must_use]
pub fn logical_key_for(key_code: KeyCode, text: Option<&str>) -> Key {
    if let Some(named) = named_logical_key(key_code) {
        return named;
    }
    if let Some(text) = text {
        return Key::Character(text.into());
    }
    match character_for_key_code(key_code) {
        Some(character) => Key::Character(character.to_string().as_str().into()),
        None => Key::Unidentified(NativeKey::Unidentified),
    }
}

fn named_logical_key(key_code: KeyCode) -> Option<Key> {
    let key = match key_code {
        KeyCode::Enter => Key::Enter,
        KeyCode::Space => Key::Space,
        KeyCode::Tab => Key::Tab,
        KeyCode::Escape => Key::Escape,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Delete => Key::Delete,
        KeyCode::ArrowUp => Key::ArrowUp,
        KeyCode::ArrowDown => Key::ArrowDown,
        KeyCode::ArrowLeft => Key::ArrowLeft,
        KeyCode::ArrowRight => Key::ArrowRight,
        KeyCode::Home => Key::Home,
        KeyCode::End => Key::End,
        KeyCode::PageUp => Key::PageUp,
        KeyCode::PageDown => Key::PageDown,
        KeyCode::ShiftLeft | KeyCode::ShiftRight => Key::Shift,
        KeyCode::ControlLeft | KeyCode::ControlRight => Key::Control,
        KeyCode::AltLeft | KeyCode::AltRight => Key::Alt,
        KeyCode::SuperLeft | KeyCode::SuperRight => Key::Super,
        KeyCode::F1 => Key::F1,
        KeyCode::F2 => Key::F2,
        KeyCode::F3 => Key::F3,
        KeyCode::F4 => Key::F4,
        KeyCode::F5 => Key::F5,
        KeyCode::F6 => Key::F6,
        KeyCode::F7 => Key::F7,
        KeyCode::F8 => Key::F8,
        KeyCode::F9 => Key::F9,
        KeyCode::F10 => Key::F10,
        KeyCode::F11 => Key::F11,
        KeyCode::F12 => Key::F12,
        _ => return None,
    };
    Some(key)
}

/// System parameter that writes injected input for the primary window.
#[derive(SystemParam)]
pub struct InputInjector<'w, 's> {
    primary_window: Query<'w, 's, (Entity, &'static mut Window), With<PrimaryWindow>>,
    cursor_moved: MessageWriter<'w, CursorMoved>,
    mouse_motion: MessageWriter<'w, MouseMotion>,
    keyboard_input: MessageWriter<'w, KeyboardInput>,
    mouse_button_input: MessageWriter<'w, MouseButtonInput>,
    mouse_wheel: MessageWriter<'w, MouseWheel>,
    touch_input: MessageWriter<'w, TouchInput>,
    pinch_gesture: MessageWriter<'w, PinchGesture>,
    rotation_gesture: MessageWriter<'w, RotationGesture>,
    pan_gesture: MessageWriter<'w, PanGesture>,
    double_tap_gesture: MessageWriter<'w, DoubleTapGesture>,
    window_events: MessageWriter<'w, WindowEvent>,
}

impl InputInjector<'_, '_> {
    fn primary_window_entity(&self) -> Result<Entity, InjectionError> {
        self.primary_window
            .single()
            .map(|(entity, _)| entity)
            .map_err(|_| InjectionError::NoPrimaryWindow)
    }

    /// Moves the cursor from `previous` to `position`.
    ///
    /// The window's `cursor_position` is written only when `update_window_cursor_position` is
    /// true, and then without flagging the window as changed. Even so, `bevy_winit` compares a
    /// changed window with a private cache, so a later change to the `Window` warps the real OS
    /// cursor. That is why the write is off by default.
    ///
    /// # Errors
    ///
    /// Returns [`InjectionError::NoPrimaryWindow`] when no primary window exists.
    pub fn move_cursor(
        &mut self,
        position: Vec2,
        previous: Option<Vec2>,
        update_window_cursor_position: bool,
    ) -> Result<(), InjectionError> {
        let (window, mut window_component) = self
            .primary_window
            .single_mut()
            .map_err(|_| InjectionError::NoPrimaryWindow)?;
        if update_window_cursor_position {
            window_component
                .bypass_change_detection()
                .set_cursor_position(Some(position));
        }

        let delta = previous.map(|previous| position - previous);
        let moved = CursorMoved {
            window,
            position,
            delta,
        };
        let motion = MouseMotion {
            delta: delta.unwrap_or(Vec2::ZERO),
        };
        self.window_events
            .write(WindowEvent::CursorMoved(moved.clone()));
        self.cursor_moved.write(moved);
        self.window_events.write(WindowEvent::MouseMotion(motion));
        self.mouse_motion.write(motion);
        Ok(())
    }

    /// Writes a key press or release.
    ///
    /// # Errors
    ///
    /// Returns [`InjectionError::NoPrimaryWindow`] when no primary window exists.
    pub fn key(
        &mut self,
        key_code: KeyCode,
        text: Option<&str>,
        state: ButtonState,
    ) -> Result<(), InjectionError> {
        let window = self.primary_window_entity()?;
        let input = KeyboardInput {
            key_code,
            logical_key: logical_key_for(key_code, text),
            state,
            text: text.map(Into::into),
            repeat: false,
            window,
        };
        self.window_events
            .write(WindowEvent::KeyboardInput(input.clone()));
        self.keyboard_input.write(input);
        Ok(())
    }

    /// Writes a mouse button press or release.
    ///
    /// # Errors
    ///
    /// Returns [`InjectionError::NoPrimaryWindow`] when no primary window exists.
    pub fn mouse_button(
        &mut self,
        button: MouseButton,
        state: ButtonState,
    ) -> Result<(), InjectionError> {
        let window = self.primary_window_entity()?;
        let input = MouseButtonInput {
            button,
            state,
            window,
        };
        self.window_events
            .write(WindowEvent::MouseButtonInput(input));
        self.mouse_button_input.write(input);
        Ok(())
    }

    /// Writes a scroll wheel movement.
    ///
    /// # Errors
    ///
    /// Returns [`InjectionError::NoPrimaryWindow`] when no primary window exists.
    pub fn scroll(&mut self, unit: MouseScrollUnit, x: f32, y: f32) -> Result<(), InjectionError> {
        let window = self.primary_window_entity()?;
        let wheel = MouseWheel {
            unit,
            x,
            y,
            window,
            phase: TouchPhase::Moved,
        };
        self.window_events.write(WindowEvent::MouseWheel(wheel));
        self.mouse_wheel.write(wheel);
        Ok(())
    }

    /// Writes a touch event.
    ///
    /// # Errors
    ///
    /// Returns [`InjectionError::NoPrimaryWindow`] when no primary window exists.
    pub fn touch(
        &mut self,
        phase: TouchPhase,
        position: Vec2,
        id: u64,
    ) -> Result<(), InjectionError> {
        let window = self.primary_window_entity()?;
        let input = TouchInput {
            phase,
            position,
            window,
            force: None,
            id,
        };
        self.window_events.write(WindowEvent::TouchInput(input));
        self.touch_input.write(input);
        Ok(())
    }

    /// Writes a pinch gesture.
    pub fn pinch(&mut self, delta: f32) {
        self.window_events
            .write(WindowEvent::PinchGesture(PinchGesture(delta)));
        self.pinch_gesture.write(PinchGesture(delta));
    }

    /// Writes a rotation gesture.
    pub fn rotate(&mut self, delta: f32) {
        self.window_events
            .write(WindowEvent::RotationGesture(RotationGesture(delta)));
        self.rotation_gesture.write(RotationGesture(delta));
    }

    /// Writes a pan gesture.
    pub fn pan(&mut self, delta: Vec2) {
        self.window_events
            .write(WindowEvent::PanGesture(PanGesture(delta)));
        self.pan_gesture.write(PanGesture(delta));
    }

    /// Writes a double tap gesture.
    pub fn double_tap(&mut self) {
        self.window_events
            .write(WindowEvent::DoubleTapGesture(DoubleTapGesture));
        self.double_tap_gesture.write(DoubleTapGesture);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letters_use_text_or_lowercase_letter() {
        assert_eq!(
            logical_key_for(KeyCode::KeyA, None),
            Key::Character("a".into())
        );
        assert_eq!(
            logical_key_for(KeyCode::KeyA, Some("A")),
            Key::Character("A".into())
        );
    }

    #[test]
    fn digits_are_characters() {
        assert_eq!(
            logical_key_for(KeyCode::Digit7, None),
            Key::Character("7".into())
        );
    }

    #[test]
    fn named_keys_map_to_named_variants() {
        assert_eq!(logical_key_for(KeyCode::Enter, Some("\n")), Key::Enter);
        assert_eq!(logical_key_for(KeyCode::Space, None), Key::Space);
        assert_eq!(logical_key_for(KeyCode::ShiftRight, None), Key::Shift);
        assert_eq!(logical_key_for(KeyCode::ArrowLeft, None), Key::ArrowLeft);
        assert_eq!(logical_key_for(KeyCode::F12, None), Key::F12);
    }

    #[test]
    fn other_keys_are_unidentified_unless_text_is_given() {
        assert_eq!(
            logical_key_for(KeyCode::Comma, None),
            Key::Unidentified(NativeKey::Unidentified)
        );
        assert_eq!(
            logical_key_for(KeyCode::Comma, Some(",")),
            Key::Character(",".into())
        );
    }
}
