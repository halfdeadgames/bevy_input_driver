//! Command types and parsing.

use std::fmt;

use bevy::input::{keyboard::KeyCode, mouse::MouseButton};
use serde::Deserialize;
use serde_json::Value;

use crate::render_readiness::SETTLED_RENDER_FRAMES;

/// A position in logical pixels in the primary window, origin top left.
pub type LogicalPoint = [f32; 2];

const DEFAULT_TAP_HOLD_FRAMES: u32 = 1;
const DEFAULT_DRAG_FRAMES: u32 = 10;
/// Largest frame count one command may use.
pub const MAXIMUM_COMMAND_FRAMES: u32 = 100_000;
/// Seconds `wait_until_drawn` waits when the command gives no `timeout_seconds`.
pub const DEFAULT_DRAWN_TIMEOUT_SECONDS: f32 = 60.0;
/// Largest `timeout_seconds` a `wait_until_drawn` command may use.
pub const MAXIMUM_DRAWN_TIMEOUT_SECONDS: f32 = 3600.0;
/// Largest `settle_frames` a `wait_until_drawn` command may use.
pub const MAXIMUM_DRAWN_SETTLE_FRAMES: u32 = 600;

fn default_hold_frames() -> u32 {
    DEFAULT_TAP_HOLD_FRAMES
}

fn default_drag_frames() -> u32 {
    DEFAULT_DRAG_FRAMES
}

fn default_drawn_timeout_seconds() -> f32 {
    DEFAULT_DRAWN_TIMEOUT_SECONDS
}

fn default_drawn_settle_frames() -> u32 {
    SETTLED_RENDER_FRAMES
}

fn default_mouse_button() -> MouseButton {
    MouseButton::Left
}

/// Unit of a `scroll` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollUnit {
    /// Wheel notches.
    #[default]
    Line,
    /// Pixels, as a trackpad reports.
    Pixel,
}

/// Phase of a `touch` command.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TouchCommandPhase {
    /// A finger touched the screen.
    Started,
    /// A finger moved.
    Moved,
    /// A finger lifted.
    Ended,
    /// The touch was interrupted.
    Canceled,
}

/// One input or control command, as written in JSON.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(tag = "command", rename_all = "snake_case", deny_unknown_fields)]
pub enum DriverCommand {
    /// Press a key and keep it held.
    KeyDown {
        /// Physical key, by `KeyCode` name such as `KeyA` or `Space`.
        key: KeyCode,
        /// Text the key produces, if any.
        #[serde(default)]
        text: Option<String>,
    },
    /// Release a key.
    KeyUp {
        /// Physical key.
        key: KeyCode,
    },
    /// Press a key, hold it for some frames, then release it.
    KeyTap {
        /// Physical key.
        key: KeyCode,
        /// Text the key produces, if any.
        #[serde(default)]
        text: Option<String>,
        /// Frames the key stays down. Must be at least 1.
        #[serde(default = "default_hold_frames")]
        hold_frames: u32,
    },
    /// Type each character as a key press, one key down and one key up per frame.
    TypeText {
        /// Characters to type.
        text: String,
    },
    /// Move the cursor.
    MouseMove {
        /// Logical x.
        x: f32,
        /// Logical y.
        y: f32,
    },
    /// Press a mouse button.
    MouseDown {
        /// Button, `Left` by default.
        #[serde(default = "default_mouse_button")]
        button: MouseButton,
    },
    /// Release a mouse button.
    MouseUp {
        /// Button, `Left` by default.
        #[serde(default = "default_mouse_button")]
        button: MouseButton,
    },
    /// Press and release a mouse button, moving first when `x` and `y` are given.
    Click {
        /// Button, `Left` by default.
        #[serde(default = "default_mouse_button")]
        button: MouseButton,
        /// Logical x to move to first. Requires `y`.
        #[serde(default)]
        x: Option<f32>,
        /// Logical y to move to first. Requires `x`.
        #[serde(default)]
        y: Option<f32>,
    },
    /// Press at `from`, move in evenly spaced steps to `to`, and release.
    Drag {
        /// Start position.
        from: LogicalPoint,
        /// End position.
        to: LogicalPoint,
        /// Button, `Left` by default.
        #[serde(default = "default_mouse_button")]
        button: MouseButton,
        /// Number of moves from `from` to `to`. Must be at least 1.
        #[serde(default = "default_drag_frames")]
        frames: u32,
    },
    /// Scroll the wheel.
    Scroll {
        /// Horizontal amount.
        #[serde(default)]
        x: f32,
        /// Vertical amount.
        #[serde(default)]
        y: f32,
        /// Unit of the amounts.
        #[serde(default)]
        unit: ScrollUnit,
    },
    /// Send one touch event.
    Touch {
        /// Touch phase.
        phase: TouchCommandPhase,
        /// Logical x.
        x: f32,
        /// Logical y.
        y: f32,
        /// Finger identifier.
        #[serde(default)]
        id: u64,
    },
    /// Touch down at `from`, move in evenly spaced steps to `to`, and lift.
    TouchDrag {
        /// Start position.
        from: LogicalPoint,
        /// End position.
        to: LogicalPoint,
        /// Finger identifier.
        #[serde(default)]
        id: u64,
        /// Number of moves from `from` to `to`. Must be at least 1.
        #[serde(default = "default_drag_frames")]
        frames: u32,
    },
    /// Two-finger pinch gesture.
    Pinch {
        /// Pinch amount.
        delta: f32,
    },
    /// Two-finger rotation gesture.
    Rotate {
        /// Rotation amount.
        delta: f32,
    },
    /// Pan gesture.
    Pan {
        /// Horizontal amount.
        x: f32,
        /// Vertical amount.
        y: f32,
    },
    /// Double tap gesture.
    DoubleTap {},
    /// Wait for a number of frames.
    WaitFrames {
        /// Frames to wait.
        frames: u32,
    },
    /// Wait for wall-clock seconds, measured with `std::time::Instant` from the moment the wait
    /// starts. Pausing virtual time or `time_fixed_step` does not change its length.
    WaitSeconds {
        /// Seconds to wait.
        seconds: f32,
    },
    /// Wait until the render pipelines in use have compiled and a frame using them was drawn,
    /// or until `timeout_seconds` of wall-clock time have passed.
    WaitUntilDrawn {
        /// Wall-clock seconds before giving up and recording an error. Above 0 and at most 3600.
        #[serde(default = "default_drawn_timeout_seconds")]
        timeout_seconds: f32,
        /// Render frames in a row with nothing compiling that count as drawn. From 1 to 600;
        /// default 3. A game that keeps making new pipelines may never have 3 such frames in a
        /// row, so a script for it can ask for fewer.
        #[serde(default = "default_drawn_settle_frames")]
        settle_frames: u32,
    },
    /// Capture the primary window to a PNG file, relative to the working directory.
    Screenshot {
        /// Output path.
        path: String,
    },
    /// Ask Bevy to minimize the primary window. Only requests the change; use `wait_frames` and
    /// read `window_minimized` in `drive/status` to see it happen.
    WindowMinimize {},
    /// Ask Bevy to restore the primary window from minimized. Only requests the change; use
    /// `wait_frames` and read `window_minimized` in `drive/status` to see it happen.
    WindowRestore {},
    /// Block or allow real keyboard and pointer input while the driver is in control.
    RealInput {
        /// True to block real input, false to let it through.
        blocked: bool,
    },
    /// Release every key and button the driver holds.
    ReleaseAll {},
    /// Exit the app: `AppExit::Success`, or `AppExit::error()` (code 1) when any `expect` failed.
    Quit {},
    /// Set `DriverSeed` to this value.
    SetSeed {
        /// The new seed.
        seed: u64,
    },
    /// Pause `Time<Virtual>`.
    TimePause {},
    /// Unpause `Time<Virtual>`.
    TimeResume {},
    /// Set the relative speed of `Time<Virtual>`.
    TimeSpeed {
        /// Speed multiplier. Must be finite and above 0.
        factor: f32,
    },
    /// Make every frame advance time by exactly this many milliseconds.
    TimeFixedStep {
        /// Milliseconds per frame. Must be finite and above 0.
        milliseconds: f64,
    },
    /// Return to automatic real-time updates and speed 1.0, without unpausing.
    TimeReal {},
    /// From a paused state, unpause, run exactly this many frames, and pause again.
    StepFrames {
        /// Frames to run. Must be at least 1.
        frames: u32,
    },
    /// Run the registered save hook with this path.
    SaveState {
        /// File path handed to the hook.
        path: String,
    },
    /// Run the registered load hook with this path.
    LoadState {
        /// File path handed to the hook.
        path: String,
    },
    /// Run a check the game registered with `add_driver_check` in the frame after everything
    /// before it, and record whether it passed in `drive/status`.
    Expect {
        /// Name the check was registered under.
        check: String,
        /// Input handed to the check; `null` when absent.
        #[serde(default)]
        arguments: Option<Value>,
        /// Value the check must return. Numbers compare by numeric value, everything else
        /// exactly. An explicit `null` expects null. Without it, any returned value passes.
        #[serde(default, deserialize_with = "deserialize_present_value")]
        equals: Option<Value>,
    },
}

// Plain `Option<Value>` turns a present `null` into `None`; a present field must stay `Some`.
fn deserialize_present_value<'de, DeserializerType>(
    deserializer: DeserializerType,
) -> Result<Option<Value>, DeserializerType::Error>
where
    DeserializerType: serde::Deserializer<'de>,
{
    Value::deserialize(deserializer).map(Some)
}

/// Why a command could not be parsed.
#[derive(Debug)]
pub enum CommandParseError {
    /// The JSON was malformed or did not match any command.
    Json(serde_json::Error),
    /// The JSON matched a command but its values are not usable.
    Invalid(String),
}

impl fmt::Display for CommandParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(source) => write!(formatter, "{source}"),
            Self::Invalid(problem) => write!(formatter, "{problem}"),
        }
    }
}

impl std::error::Error for CommandParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(source) => Some(source),
            Self::Invalid(_) => None,
        }
    }
}

impl DriverCommand {
    fn validate(self) -> Result<Self, CommandParseError> {
        if let Some(problem) = self.frame_cap_problem() {
            return Err(CommandParseError::Invalid(problem));
        }
        let problem = match &self {
            Self::KeyTap { hold_frames: 0, .. } => Some("key_tap hold_frames must be at least 1"),
            Self::Drag { frames: 0, .. } | Self::TouchDrag { frames: 0, .. } => {
                Some("drag frames must be at least 1")
            }
            Self::Click { x, y, .. } if x.is_some() != y.is_some() => {
                Some("click needs both x and y, or neither")
            }
            Self::WaitSeconds { seconds } if !seconds.is_finite() || *seconds < 0.0 => {
                Some("wait_seconds seconds must be a finite number of at least 0")
            }
            Self::WaitUntilDrawn {
                timeout_seconds, ..
            } if !(*timeout_seconds > 0.0 && *timeout_seconds <= MAXIMUM_DRAWN_TIMEOUT_SECONDS) => {
                Some("wait_until_drawn timeout_seconds must be above 0 and at most 3600")
            }
            Self::WaitUntilDrawn { settle_frames, .. }
                if !(1..=MAXIMUM_DRAWN_SETTLE_FRAMES).contains(settle_frames) =>
            {
                Some("wait_until_drawn settle_frames must be from 1 to 600")
            }
            Self::TimeSpeed { factor } if !factor.is_finite() || *factor <= 0.0 => {
                Some("time_speed factor must be a finite number above 0")
            }
            Self::TimeFixedStep { milliseconds }
                if !milliseconds.is_finite() || *milliseconds <= 0.0 =>
            {
                Some("time_fixed_step milliseconds must be a finite number above 0")
            }
            Self::StepFrames { frames: 0 } => Some("step_frames frames must be at least 1"),
            _ => None,
        };
        match problem {
            Some(message) => Err(CommandParseError::Invalid(message.to_owned())),
            None => Ok(self),
        }
    }

    // A frame count expands into that many queued steps, so an unbounded one could exhaust memory.
    fn frame_cap_problem(&self) -> Option<String> {
        let (name, frames) = match self {
            Self::KeyTap { hold_frames, .. } => ("key_tap hold_frames", *hold_frames),
            Self::Drag { frames, .. } => ("drag frames", *frames),
            Self::TouchDrag { frames, .. } => ("touch_drag frames", *frames),
            Self::StepFrames { frames } => ("step_frames frames", *frames),
            Self::WaitFrames { frames } => ("wait_frames frames", *frames),
            _ => return None,
        };
        (frames > MAXIMUM_COMMAND_FRAMES)
            .then(|| format!("{name} must be at most {MAXIMUM_COMMAND_FRAMES}, got {frames}"))
    }
}

/// Parses one command from JSON text.
///
/// # Errors
///
/// Returns [`CommandParseError`] for malformed JSON, an unknown `command`, an unknown field, a
/// missing field, or an unusable value.
pub fn parse_command_text(text: &str) -> Result<DriverCommand, CommandParseError> {
    serde_json::from_str::<DriverCommand>(text)
        .map_err(CommandParseError::Json)?
        .validate()
}

/// Parses one command from an already decoded JSON value.
///
/// # Errors
///
/// Returns [`CommandParseError`] under the same conditions as [`parse_command_text`].
pub fn parse_command_value(value: Value) -> Result<DriverCommand, CommandParseError> {
    serde_json::from_value::<DriverCommand>(value)
        .map_err(CommandParseError::Json)?
        .validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(text: &str) -> DriverCommand {
        parse_command_text(text).expect("command should parse")
    }

    #[test]
    fn parses_keyboard_commands_with_defaults() {
        assert_eq!(
            parse(r#"{"command":"key_down","key":"KeyA"}"#),
            DriverCommand::KeyDown {
                key: KeyCode::KeyA,
                text: None
            }
        );
        assert_eq!(
            parse(r#"{"command":"key_up","key":"Space"}"#),
            DriverCommand::KeyUp {
                key: KeyCode::Space
            }
        );
        assert_eq!(
            parse(r#"{"command":"key_tap","key":"Enter"}"#),
            DriverCommand::KeyTap {
                key: KeyCode::Enter,
                text: None,
                hold_frames: 1
            }
        );
        assert_eq!(
            parse(r#"{"command":"key_tap","key":"KeyB","text":"b","hold_frames":3}"#),
            DriverCommand::KeyTap {
                key: KeyCode::KeyB,
                text: Some("b".to_owned()),
                hold_frames: 3
            }
        );
        assert_eq!(
            parse(r#"{"command":"type_text","text":"hi"}"#),
            DriverCommand::TypeText {
                text: "hi".to_owned()
            }
        );
    }

    #[test]
    fn parses_mouse_commands_with_defaults() {
        assert_eq!(
            parse(r#"{"command":"mouse_move","x":1.5,"y":2}"#),
            DriverCommand::MouseMove { x: 1.5, y: 2.0 }
        );
        assert_eq!(
            parse(r#"{"command":"mouse_down"}"#),
            DriverCommand::MouseDown {
                button: MouseButton::Left
            }
        );
        assert_eq!(
            parse(r#"{"command":"mouse_up","button":"Right"}"#),
            DriverCommand::MouseUp {
                button: MouseButton::Right
            }
        );
        assert_eq!(
            parse(r#"{"command":"click","x":10,"y":20,"button":"Middle"}"#),
            DriverCommand::Click {
                button: MouseButton::Middle,
                x: Some(10.0),
                y: Some(20.0)
            }
        );
        assert_eq!(
            parse(r#"{"command":"drag","from":[1,2],"to":[3,4]}"#),
            DriverCommand::Drag {
                from: [1.0, 2.0],
                to: [3.0, 4.0],
                button: MouseButton::Left,
                frames: 10
            }
        );
    }

    #[test]
    fn parses_scroll_touch_and_gesture_commands() {
        assert_eq!(
            parse(r#"{"command":"scroll"}"#),
            DriverCommand::Scroll {
                x: 0.0,
                y: 0.0,
                unit: ScrollUnit::Line
            }
        );
        assert_eq!(
            parse(r#"{"command":"scroll","y":-3,"unit":"pixel"}"#),
            DriverCommand::Scroll {
                x: 0.0,
                y: -3.0,
                unit: ScrollUnit::Pixel
            }
        );
        assert_eq!(
            parse(r#"{"command":"touch","phase":"started","x":5,"y":6}"#),
            DriverCommand::Touch {
                phase: TouchCommandPhase::Started,
                x: 5.0,
                y: 6.0,
                id: 0
            }
        );
        assert_eq!(
            parse(r#"{"command":"touch_drag","from":[0,0],"to":[9,9],"id":2}"#),
            DriverCommand::TouchDrag {
                from: [0.0, 0.0],
                to: [9.0, 9.0],
                id: 2,
                frames: 10
            }
        );
        assert_eq!(
            parse(r#"{"command":"pinch","delta":0.5}"#),
            DriverCommand::Pinch { delta: 0.5 }
        );
        assert_eq!(
            parse(r#"{"command":"rotate","delta":-1}"#),
            DriverCommand::Rotate { delta: -1.0 }
        );
        assert_eq!(
            parse(r#"{"command":"pan","x":1,"y":2}"#),
            DriverCommand::Pan { x: 1.0, y: 2.0 }
        );
        assert_eq!(
            parse(r#"{"command":"double_tap"}"#),
            DriverCommand::DoubleTap {}
        );
    }

    #[test]
    fn parses_control_commands() {
        assert_eq!(
            parse(r#"{"command":"wait_frames","frames":4}"#),
            DriverCommand::WaitFrames { frames: 4 }
        );
        assert_eq!(
            parse(r#"{"command":"wait_seconds","seconds":0.25}"#),
            DriverCommand::WaitSeconds { seconds: 0.25 }
        );
        assert_eq!(
            parse(r#"{"command":"wait_until_drawn"}"#),
            DriverCommand::WaitUntilDrawn {
                timeout_seconds: DEFAULT_DRAWN_TIMEOUT_SECONDS,
                settle_frames: SETTLED_RENDER_FRAMES,
            }
        );
        assert_eq!(
            parse(r#"{"command":"wait_until_drawn","timeout_seconds":5,"settle_frames":1}"#),
            DriverCommand::WaitUntilDrawn {
                timeout_seconds: 5.0,
                settle_frames: 1,
            }
        );
        assert_eq!(
            parse(r#"{"command":"screenshot","path":"shots/a.png"}"#),
            DriverCommand::Screenshot {
                path: "shots/a.png".to_owned()
            }
        );
        assert_eq!(
            parse(r#"{"command":"release_all"}"#),
            DriverCommand::ReleaseAll {}
        );
        assert_eq!(parse(r#"{"command":"quit"}"#), DriverCommand::Quit {});
        assert_eq!(
            parse(r#"{"command":"window_minimize"}"#),
            DriverCommand::WindowMinimize {}
        );
        assert_eq!(
            parse(r#"{"command":"window_restore"}"#),
            DriverCommand::WindowRestore {}
        );
        assert!(parse_command_text(r#"{"command":"window_minimize","extra":1}"#).is_err());
        assert!(parse_command_text(r#"{"command":"window_restore","extra":1}"#).is_err());
    }

    #[test]
    fn parses_real_input_with_a_required_flag() {
        assert_eq!(
            parse(r#"{"command":"real_input","blocked":false}"#),
            DriverCommand::RealInput { blocked: false }
        );
        assert_eq!(
            parse(r#"{"command":"real_input","blocked":true}"#),
            DriverCommand::RealInput { blocked: true }
        );
        assert!(parse_command_text(r#"{"command":"real_input"}"#).is_err());
        assert!(parse_command_text(r#"{"command":"real_input","blocked":"yes"}"#).is_err());
        assert!(
            parse_command_text(r#"{"command":"real_input","blocked":true,"extra":1}"#).is_err()
        );
    }

    #[test]
    fn parses_seed_time_and_state_commands() {
        assert_eq!(
            parse(r#"{"command":"set_seed","seed":18446744073709551615}"#),
            DriverCommand::SetSeed { seed: u64::MAX }
        );
        assert_eq!(
            parse(r#"{"command":"time_pause"}"#),
            DriverCommand::TimePause {}
        );
        assert_eq!(
            parse(r#"{"command":"time_resume"}"#),
            DriverCommand::TimeResume {}
        );
        assert_eq!(
            parse(r#"{"command":"time_speed","factor":0.5}"#),
            DriverCommand::TimeSpeed { factor: 0.5 }
        );
        assert_eq!(
            parse(r#"{"command":"time_fixed_step","milliseconds":16.5}"#),
            DriverCommand::TimeFixedStep { milliseconds: 16.5 }
        );
        assert_eq!(
            parse(r#"{"command":"time_real"}"#),
            DriverCommand::TimeReal {}
        );
        assert_eq!(
            parse(r#"{"command":"step_frames","frames":3}"#),
            DriverCommand::StepFrames { frames: 3 }
        );
        assert_eq!(
            parse(r#"{"command":"save_state","path":"slot.txt"}"#),
            DriverCommand::SaveState {
                path: "slot.txt".to_owned()
            }
        );
        assert_eq!(
            parse(r#"{"command":"load_state","path":"slot.txt"}"#),
            DriverCommand::LoadState {
                path: "slot.txt".to_owned()
            }
        );
    }

    #[test]
    fn parses_expect_with_and_without_arguments_and_equals() {
        assert_eq!(
            parse(r#"{"command":"expect","check":"alive"}"#),
            DriverCommand::Expect {
                check: "alive".to_owned(),
                arguments: None,
                equals: None
            }
        );
        assert_eq!(
            parse(r#"{"command":"expect","check":"hp","arguments":{"who":1},"equals":[5,6.5]}"#),
            DriverCommand::Expect {
                check: "hp".to_owned(),
                arguments: Some(serde_json::json!({"who": 1})),
                equals: Some(serde_json::json!([5, 6.5]))
            }
        );
        assert_eq!(
            parse(r#"{"command":"expect","check":"flag","equals":false}"#),
            DriverCommand::Expect {
                check: "flag".to_owned(),
                arguments: None,
                equals: Some(Value::Bool(false))
            }
        );
        assert_eq!(
            parse(r#"{"command":"expect","check":"flag","equals":null}"#),
            DriverCommand::Expect {
                check: "flag".to_owned(),
                arguments: None,
                equals: Some(Value::Null)
            }
        );
        assert_eq!(
            parse(r#"{"command":"expect","check":"flag","equals":5}"#),
            DriverCommand::Expect {
                check: "flag".to_owned(),
                arguments: None,
                equals: Some(serde_json::json!(5))
            }
        );
        assert!(parse_command_text(r#"{"command":"expect"}"#).is_err());
        assert!(parse_command_text(r#"{"command":"expect","check":"a","extra":1}"#).is_err());
    }

    #[test]
    fn rejects_unusable_seed_time_and_state_values() {
        assert!(parse_command_text(r#"{"command":"set_seed"}"#).is_err());
        assert!(parse_command_text(r#"{"command":"set_seed","seed":-1}"#).is_err());
        assert!(parse_command_text(r#"{"command":"time_speed","factor":0}"#).is_err());
        assert!(parse_command_text(r#"{"command":"time_speed","factor":-2}"#).is_err());
        assert!(parse_command_text(r#"{"command":"time_speed"}"#).is_err());
        assert!(parse_command_text(r#"{"command":"time_fixed_step","milliseconds":0}"#).is_err());
        assert!(parse_command_text(r#"{"command":"time_fixed_step","milliseconds":-16}"#).is_err());
        assert!(parse_command_text(r#"{"command":"step_frames","frames":0}"#).is_err());
        assert!(parse_command_text(r#"{"command":"save_state"}"#).is_err());
        assert!(parse_command_text(r#"{"command":"load_state","path":"a","extra":1}"#).is_err());
    }

    #[test]
    fn rejects_unknown_command_and_unknown_field() {
        let unknown_command = parse_command_text(r#"{"command":"teleport"}"#)
            .expect_err("unknown command should fail")
            .to_string();
        assert!(unknown_command.contains("teleport"), "{unknown_command}");

        let unknown_field = parse_command_text(r#"{"command":"key_up","key":"KeyA","extra":1}"#)
            .expect_err("unknown field should fail")
            .to_string();
        assert!(unknown_field.contains("extra"), "{unknown_field}");
    }

    #[test]
    fn rejects_unusable_values() {
        assert!(
            parse_command_text(r#"{"command":"key_tap","key":"KeyA","hold_frames":0}"#).is_err()
        );
        assert!(
            parse_command_text(r#"{"command":"drag","from":[0,0],"to":[1,1],"frames":0}"#).is_err()
        );
        assert!(parse_command_text(r#"{"command":"click","x":1}"#).is_err());
        assert!(parse_command_text(r#"{"command":"wait_seconds","seconds":-1}"#).is_err());
        for timeout in ["0", "-1", "3601"] {
            let text = format!(r#"{{"command":"wait_until_drawn","timeout_seconds":{timeout}}}"#);
            assert!(parse_command_text(&text).is_err(), "{text}");
        }
        for settle_frames in ["0", "601"] {
            let text =
                format!(r#"{{"command":"wait_until_drawn","settle_frames":{settle_frames}}}"#);
            assert!(parse_command_text(&text).is_err(), "{text}");
        }
    }

    #[test]
    fn rejects_frame_counts_above_the_cap() {
        for text in [
            r#"{"command":"key_tap","key":"KeyA","hold_frames":100001}"#,
            r#"{"command":"drag","from":[0,0],"to":[1,1],"frames":4000000000}"#,
            r#"{"command":"touch_drag","from":[0,0],"to":[1,1],"frames":100001}"#,
            r#"{"command":"step_frames","frames":100001}"#,
            r#"{"command":"wait_frames","frames":100001}"#,
        ] {
            let message = parse_command_text(text)
                .expect_err("count above the cap should fail")
                .to_string();
            assert!(message.contains("at most 100000"), "{message}");
        }
        assert!(parse_command_text(r#"{"command":"wait_frames","frames":100000}"#).is_ok());
    }

    #[test]
    fn rejects_the_old_command_and_millisecond_keys() {
        assert!(parse_command_text(r#"{"cmd":"quit"}"#).is_err());
        assert!(parse_command_text(r#"{"command":"time_fixed_step","ms":16}"#).is_err());
    }
}
