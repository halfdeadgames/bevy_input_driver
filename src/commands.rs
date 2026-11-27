//! Command types and parsing.

use std::fmt;

use bevy::{
    input::{
        gamepad::{GamepadAxis, GamepadButton},
        keyboard::KeyCode,
        mouse::MouseButton,
    },
    math::Vec2,
};
use serde::{Deserialize, de::Error as _};
use serde_json::{Map, Value};

use crate::{
    gamepad_injection::{GamepadSlot, deserialize_gamepad_axis, deserialize_gamepad_button},
    render_readiness::SETTLED_RENDER_FRAMES,
};

/// A position in logical pixels in the primary window, origin top left.
pub type LogicalPoint = [f32; 2];

/// A position the game names instead of the script spelling it in pixels, such as
/// `{"target":"cell","column":3,"row":5}`.
///
/// The game turns it into a pixel position with a system registered through
/// `add_driver_target`, right before the command runs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct DriverTarget {
    /// Name the game registered its target system under.
    pub target: String,
    /// Every other key of the object, handed to the target system as a JSON object. The key
    /// `target` is reserved for the name, so a target system never receives an argument by that
    /// name.
    #[serde(flatten)]
    pub arguments: Map<String, Value>,
}

/// One end of a `drag` or `touch_drag`: a pixel position or a game-defined target.
#[derive(Debug, Clone, PartialEq)]
pub enum PointerPosition {
    /// Logical pixels, written as `[x, y]`.
    Pixels(LogicalPoint),
    /// A position the game's target system computes when the command starts.
    Target(DriverTarget),
}

impl PointerPosition {
    /// The pixel position, or none while it is still a target.
    #[must_use]
    pub fn pixels(&self) -> Option<Vec2> {
        match self {
            Self::Pixels(point) => Some(Vec2::from(*point)),
            Self::Target(_) => None,
        }
    }
}

impl<'de> Deserialize<'de> for PointerPosition {
    fn deserialize<DeserializerType>(
        deserializer: DeserializerType,
    ) -> Result<Self, DeserializerType::Error>
    where
        DeserializerType: serde::Deserializer<'de>,
    {
        let value = Value::deserialize(deserializer)?;
        match value {
            Value::Array(_) => serde_json::from_value(value)
                .map(Self::Pixels)
                .map_err(DeserializerType::Error::custom),
            Value::Object(_) => serde_json::from_value(value)
                .map(Self::Target)
                .map_err(DeserializerType::Error::custom),
            _ => Err(DeserializerType::Error::custom(
                "a position must be [x, y] or an object with a \"target\" key",
            )),
        }
    }
}

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
/// Largest `width` or `height`, in logical pixels, a `resize` command may use.
pub const MAXIMUM_WINDOW_SIZE: f32 = 8192.0;

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
    /// Move the cursor, to `at` or to `x` and `y`. Exactly one of the two forms is required.
    MouseMove {
        /// Logical x. Requires `y`.
        #[serde(default)]
        x: Option<f32>,
        /// Logical y. Requires `x`.
        #[serde(default)]
        y: Option<f32>,
        /// Game-defined target to move to, instead of `x` and `y`.
        #[serde(default)]
        at: Option<DriverTarget>,
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
    /// Press and release a mouse button, moving first when `at`, or `x` and `y`, are given.
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
        /// Game-defined target to move to first, instead of `x` and `y`.
        #[serde(default)]
        at: Option<DriverTarget>,
    },
    /// Press at `from`, move in evenly spaced steps to `to`, and release.
    Drag {
        /// Start position, `[x, y]` or a game-defined target.
        from: PointerPosition,
        /// End position, `[x, y]` or a game-defined target.
        to: PointerPosition,
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
        /// Logical x. Requires `y`.
        #[serde(default)]
        x: Option<f32>,
        /// Logical y. Requires `x`.
        #[serde(default)]
        y: Option<f32>,
        /// Game-defined target to touch, instead of `x` and `y`.
        #[serde(default)]
        at: Option<DriverTarget>,
        /// Finger identifier.
        #[serde(default)]
        id: u64,
    },
    /// Touch down at `from`, move in evenly spaced steps to `to`, and lift.
    TouchDrag {
        /// Start position, `[x, y]` or a game-defined target.
        from: PointerPosition,
        /// End position, `[x, y]` or a game-defined target.
        to: PointerPosition,
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
    /// Connect a fake gamepad in a slot.
    GamepadConnect {
        /// Slot number the other gamepad commands use.
        gamepad: GamepadSlot,
    },
    /// Disconnect the fake gamepad in a slot.
    GamepadDisconnect {
        /// Slot number.
        gamepad: GamepadSlot,
    },
    /// Set a fake gamepad button's value.
    #[serde(rename = "gamepad_button")]
    GamepadSetButton {
        /// Slot number.
        gamepad: GamepadSlot,
        /// Button by snake_case name, such as `south` or `d_pad_up`.
        #[serde(deserialize_with = "deserialize_gamepad_button")]
        button: GamepadButton,
        /// From 0.0 (released) to 1.0 (pressed).
        value: f32,
    },
    /// Set a fake gamepad axis position.
    #[serde(rename = "gamepad_axis")]
    GamepadSetAxis {
        /// Slot number.
        gamepad: GamepadSlot,
        /// Axis by snake_case name, such as `left_stick_x`.
        #[serde(deserialize_with = "deserialize_gamepad_axis")]
        axis: GamepadAxis,
        /// From -1.0 to 1.0.
        value: f32,
    },
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
    /// Tell the game the primary window gained or lost focus, by writing `WindowFocused`.
    ///
    /// `Window::focused` is deliberately left unchanged: `bevy_winit` compares a changed
    /// `Window` with its cache and calls the real operating system `focus_window()` when
    /// `focused` becomes true, which would let the user's real keyboard into the game.
    Focus {
        /// True when the window gained focus, false when it lost it.
        focused: bool,
    },
    /// Really resize the primary window, so winit resizes the operating system window and the
    /// game gets genuine `WindowResized` events.
    Resize {
        /// Logical width. Finite, above 0, and at most 8192.
        width: f32,
        /// Logical height. Finite, above 0, and at most 8192.
        height: f32,
    },
    /// Report a file hovering over the primary window.
    FileHover {
        /// Path handed to the game unchanged. It need not exist. Must not be empty.
        path: String,
    },
    /// Report a file dropped on the primary window.
    FileDrop {
        /// Path handed to the game unchanged. It need not exist. Must not be empty.
        path: String,
    },
    /// Report that a hovering file left the primary window without being dropped.
    FileHoverCancel {},
    /// Report that the input method editor was enabled.
    ImeEnable {},
    /// Report composing text from the input method editor.
    ImePreedit {
        /// The text being composed.
        text: String,
        /// Cursor start and end as byte offsets into `text`, on character boundaries, with start
        /// at most end. Absent or `null` hides the cursor.
        #[serde(default)]
        cursor: Option<[usize; 2]>,
    },
    /// Report text the input method editor wants inserted.
    ImeCommit {
        /// The text to insert.
        text: String,
    },
    /// Report that the input method editor was disabled.
    ImeDisable {},
    /// Put text on the real operating system clipboard. The first one of a run first saves the
    /// user's text so `clipboard_restore` or exiting can put it back.
    ClipboardSet {
        /// The text to place.
        text: String,
    },
    /// Put the user's saved clipboard text back and forget it.
    ClipboardRestore {},
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
    /// A command gave both an `at` target and `x` or `y`.
    ConflictingPosition {
        /// Name of the command.
        command: &'static str,
    },
}

impl fmt::Display for CommandParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(source) => write!(formatter, "{source}"),
            Self::Invalid(problem) => write!(formatter, "{problem}"),
            Self::ConflictingPosition { command } => write!(
                formatter,
                "{command} takes either `at` or x and y, not both"
            ),
        }
    }
}

impl std::error::Error for CommandParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Json(source) => Some(source),
            Self::Invalid(_) | Self::ConflictingPosition { .. } => None,
        }
    }
}

fn window_size_problem(width: f32, height: f32) -> Option<String> {
    let usable = |size: f32| size > 0.0 && size <= MAXIMUM_WINDOW_SIZE;
    (!usable(width) || !usable(height)).then(|| {
        format!(
            "resize width and height must be finite numbers above 0 and at most \
             {MAXIMUM_WINDOW_SIZE}, got {width} by {height}"
        )
    })
}

fn preedit_cursor_problem(text: &str, [start, end]: [usize; 2]) -> Option<String> {
    if start > end || end > text.len() {
        return Some(format!(
            "ime_preedit cursor must satisfy start <= end <= {} (the text's length in bytes), \
             got [{start}, {end}]",
            text.len()
        ));
    }
    (!text.is_char_boundary(start) || !text.is_char_boundary(end))
        .then(|| "ime_preedit cursor offsets must fall on character boundaries".to_owned())
}

// What `command` needs to say where it points: `at`, or both `x` and `y`. `optional` commands
// may also give neither.
fn pointer_position_problem(
    command: &'static str,
    x: Option<f32>,
    y: Option<f32>,
    has_target: bool,
    optional: bool,
) -> Option<CommandParseError> {
    match (x, y, has_target) {
        (Some(_), Some(_), false) | (None, None, true) => None,
        (None, None, false) if optional => None,
        (_, _, true) => Some(CommandParseError::ConflictingPosition { command }),
        _ if optional => Some(CommandParseError::Invalid(format!(
            "{command} needs `at`, both x and y, or neither"
        ))),
        _ => Some(CommandParseError::Invalid(format!(
            "{command} needs `at`, or both x and y"
        ))),
    }
}

fn resolve_pointer<ErrorType>(
    at: Option<DriverTarget>,
    x: Option<f32>,
    y: Option<f32>,
    resolve: &mut impl FnMut(&DriverTarget) -> Result<Vec2, ErrorType>,
) -> Result<(Option<f32>, Option<f32>), ErrorType> {
    match at {
        Some(target) => {
            let position = resolve(&target)?;
            Ok((Some(position.x), Some(position.y)))
        }
        None => Ok((x, y)),
    }
}

fn resolve_end<ErrorType>(
    end: PointerPosition,
    resolve: &mut impl FnMut(&DriverTarget) -> Result<Vec2, ErrorType>,
) -> Result<PointerPosition, ErrorType> {
    match end {
        PointerPosition::Target(target) => {
            resolve(&target).map(|position| PointerPosition::Pixels(position.to_array()))
        }
        pixels @ PointerPosition::Pixels(_) => Ok(pixels),
    }
}

impl DriverCommand {
    /// True when the command still names a game-defined target instead of pixels.
    #[must_use]
    pub fn has_targets(&self) -> bool {
        match self {
            Self::MouseMove { at, .. } | Self::Click { at, .. } | Self::Touch { at, .. } => {
                at.is_some()
            }
            Self::Drag { from, to, .. } | Self::TouchDrag { from, to, .. } => {
                matches!(from, PointerPosition::Target(_))
                    || matches!(to, PointerPosition::Target(_))
            }
            _ => false,
        }
    }

    /// The command's name as written in scripts, for messages.
    #[must_use]
    pub fn name(&self) -> &'static str {
        match self {
            Self::MouseMove { .. } => "mouse_move",
            Self::Click { .. } => "click",
            Self::Drag { .. } => "drag",
            Self::Touch { .. } => "touch",
            Self::TouchDrag { .. } => "touch_drag",
            _ => "command",
        }
    }

    /// Replaces every target with the pixel position `resolve` returns for it, in reading order
    /// (`from` before `to`), and stops at the first error.
    ///
    /// # Errors
    ///
    /// Returns the first error `resolve` returns.
    pub fn resolve_targets<ErrorType>(
        self,
        mut resolve: impl FnMut(&DriverTarget) -> Result<Vec2, ErrorType>,
    ) -> Result<Self, ErrorType> {
        Ok(match self {
            Self::MouseMove { x, y, at } => {
                let (x, y) = resolve_pointer(at, x, y, &mut resolve)?;
                Self::MouseMove { x, y, at: None }
            }
            Self::Click { button, x, y, at } => {
                let (x, y) = resolve_pointer(at, x, y, &mut resolve)?;
                Self::Click {
                    button,
                    x,
                    y,
                    at: None,
                }
            }
            Self::Touch {
                phase,
                x,
                y,
                at,
                id,
            } => {
                let (x, y) = resolve_pointer(at, x, y, &mut resolve)?;
                Self::Touch {
                    phase,
                    x,
                    y,
                    at: None,
                    id,
                }
            }
            Self::Drag {
                from,
                to,
                button,
                frames,
            } => Self::Drag {
                from: resolve_end(from, &mut resolve)?,
                to: resolve_end(to, &mut resolve)?,
                button,
                frames,
            },
            Self::TouchDrag {
                from,
                to,
                id,
                frames,
            } => Self::TouchDrag {
                from: resolve_end(from, &mut resolve)?,
                to: resolve_end(to, &mut resolve)?,
                id,
                frames,
            },
            other => other,
        })
    }

    fn position_problem(&self) -> Option<CommandParseError> {
        match self {
            Self::MouseMove { x, y, at } => {
                pointer_position_problem("mouse_move", *x, *y, at.is_some(), false)
            }
            Self::Touch { x, y, at, .. } => {
                pointer_position_problem("touch", *x, *y, at.is_some(), false)
            }
            Self::Click { x, y, at, .. } => {
                pointer_position_problem("click", *x, *y, at.is_some(), true)
            }
            _ => None,
        }
    }

    fn validate(self) -> Result<Self, CommandParseError> {
        if let Some(problem) = self.frame_cap_problem() {
            return Err(CommandParseError::Invalid(problem));
        }
        if let Some(problem) = self.position_problem() {
            return Err(problem);
        }
        if let Some(problem) = self.argument_problem() {
            return Err(CommandParseError::Invalid(problem));
        }
        let problem = match &self {
            Self::KeyTap { hold_frames: 0, .. } => Some("key_tap hold_frames must be at least 1"),
            Self::Drag { frames: 0, .. } | Self::TouchDrag { frames: 0, .. } => {
                Some("drag frames must be at least 1")
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
            Self::GamepadSetButton { value, .. } if !(0.0..=1.0).contains(value) => {
                Some("gamepad_button value must be a number from 0.0 to 1.0")
            }
            Self::GamepadSetAxis { value, .. } if !(-1.0..=1.0).contains(value) => {
                Some("gamepad_axis value must be a number from -1.0 to 1.0")
            }
            _ => None,
        };
        match problem {
            Some(message) => Err(CommandParseError::Invalid(message.to_owned())),
            None => Ok(self),
        }
    }

    fn argument_problem(&self) -> Option<String> {
        match self {
            Self::Resize { width, height } => window_size_problem(*width, *height),
            Self::FileHover { path } | Self::FileDrop { path } if path.is_empty() => {
                Some("file_hover and file_drop path must not be empty".to_owned())
            }
            Self::ImePreedit {
                text,
                cursor: Some(cursor),
            } => preedit_cursor_problem(text, *cursor),
            _ => None,
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
            DriverCommand::MouseMove {
                x: Some(1.5),
                y: Some(2.0),
                at: None
            }
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
                y: Some(20.0),
                at: None
            }
        );
        assert_eq!(
            parse(r#"{"command":"drag","from":[1,2],"to":[3,4]}"#),
            DriverCommand::Drag {
                from: PointerPosition::Pixels([1.0, 2.0]),
                to: PointerPosition::Pixels([3.0, 4.0]),
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
                x: Some(5.0),
                y: Some(6.0),
                at: None,
                id: 0
            }
        );
        assert_eq!(
            parse(r#"{"command":"touch_drag","from":[0,0],"to":[9,9],"id":2}"#),
            DriverCommand::TouchDrag {
                from: PointerPosition::Pixels([0.0, 0.0]),
                to: PointerPosition::Pixels([9.0, 9.0]),
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
    fn parses_gamepad_commands() {
        assert_eq!(
            parse(r#"{"command":"gamepad_connect","gamepad":0}"#),
            DriverCommand::GamepadConnect {
                gamepad: GamepadSlot(0)
            }
        );
        assert_eq!(
            parse(r#"{"command":"gamepad_disconnect","gamepad":3}"#),
            DriverCommand::GamepadDisconnect {
                gamepad: GamepadSlot(3)
            }
        );
        assert_eq!(
            parse(r#"{"command":"gamepad_button","gamepad":0,"button":"d_pad_up","value":1.0}"#),
            DriverCommand::GamepadSetButton {
                gamepad: GamepadSlot(0),
                button: GamepadButton::DPadUp,
                value: 1.0
            }
        );
        assert_eq!(
            parse(r#"{"command":"gamepad_axis","gamepad":1,"axis":"left_stick_x","value":-0.5}"#),
            DriverCommand::GamepadSetAxis {
                gamepad: GamepadSlot(1),
                axis: GamepadAxis::LeftStickX,
                value: -0.5
            }
        );
    }

    #[test]
    fn rejects_unusable_gamepad_values() {
        for text in [
            r#"{"command":"gamepad_button","gamepad":0,"button":"cross","value":1}"#,
            r#"{"command":"gamepad_button","gamepad":0,"button":"south","value":1.5}"#,
            r#"{"command":"gamepad_button","gamepad":0,"button":"south","value":-0.1}"#,
            r#"{"command":"gamepad_button","gamepad":0,"button":"south"}"#,
            r#"{"command":"gamepad_axis","gamepad":0,"axis":"tilt","value":0}"#,
            r#"{"command":"gamepad_axis","gamepad":0,"axis":"left_stick_x","value":-1.01}"#,
            r#"{"command":"gamepad_connect","gamepad":-1}"#,
            r#"{"command":"gamepad_connect","gamepad":256}"#,
            r#"{"command":"gamepad_connect"}"#,
            r#"{"command":"gamepad_disconnect","gamepad":0,"extra":1}"#,
        ] {
            assert!(parse_command_text(text).is_err(), "{text}");
        }
        let message = parse_command_text(
            r#"{"command":"gamepad_button","gamepad":0,"button":"south","value":2}"#,
        )
        .expect_err("a value above 1 should fail")
        .to_string();
        assert!(message.contains("0.0 to 1.0"), "{message}");
    }

    #[test]
    fn parses_window_event_commands() {
        assert_eq!(
            parse(r#"{"command":"focus","focused":false}"#),
            DriverCommand::Focus { focused: false }
        );
        assert_eq!(
            parse(r#"{"command":"resize","width":800,"height":600.5}"#),
            DriverCommand::Resize {
                width: 800.0,
                height: 600.5
            }
        );
        assert_eq!(
            parse(r#"{"command":"file_hover","path":"assets/level.txt"}"#),
            DriverCommand::FileHover {
                path: "assets/level.txt".to_owned()
            }
        );
        assert_eq!(
            parse(r#"{"command":"file_drop","path":"a.png"}"#),
            DriverCommand::FileDrop {
                path: "a.png".to_owned()
            }
        );
        assert_eq!(
            parse(r#"{"command":"file_hover_cancel"}"#),
            DriverCommand::FileHoverCancel {}
        );
    }

    #[test]
    fn parses_ime_and_clipboard_commands() {
        assert_eq!(
            parse(r#"{"command":"ime_enable"}"#),
            DriverCommand::ImeEnable {}
        );
        assert_eq!(
            parse(r#"{"command":"ime_disable"}"#),
            DriverCommand::ImeDisable {}
        );
        assert_eq!(
            parse(r#"{"command":"ime_commit","text":"日本"}"#),
            DriverCommand::ImeCommit {
                text: "日本".to_owned()
            }
        );
        assert_eq!(
            parse(r#"{"command":"ime_preedit","text":"に","cursor":[0,3]}"#),
            DriverCommand::ImePreedit {
                text: "に".to_owned(),
                cursor: Some([0, 3])
            }
        );
        for text in [
            r#"{"command":"ime_preedit","text":"に"}"#,
            r#"{"command":"ime_preedit","text":"に","cursor":null}"#,
        ] {
            assert_eq!(
                parse(text),
                DriverCommand::ImePreedit {
                    text: "に".to_owned(),
                    cursor: None
                }
            );
        }
        assert_eq!(
            parse(r#"{"command":"clipboard_set","text":"hello"}"#),
            DriverCommand::ClipboardSet {
                text: "hello".to_owned()
            }
        );
        assert_eq!(
            parse(r#"{"command":"clipboard_restore"}"#),
            DriverCommand::ClipboardRestore {}
        );
    }

    #[test]
    fn rejects_unusable_window_event_values() {
        for text in [
            r#"{"command":"focus"}"#,
            r#"{"command":"focus","focused":"yes"}"#,
            r#"{"command":"resize","width":800}"#,
            r#"{"command":"resize","width":0,"height":600}"#,
            r#"{"command":"resize","width":800,"height":-1}"#,
            r#"{"command":"resize","width":8193,"height":600}"#,
            r#"{"command":"resize","width":1e999,"height":600}"#,
            r#"{"command":"file_hover","path":""}"#,
            r#"{"command":"file_drop","path":""}"#,
            r#"{"command":"file_drop"}"#,
            r#"{"command":"file_hover_cancel","extra":1}"#,
            r#"{"command":"ime_enable","extra":1}"#,
            r#"{"command":"ime_commit"}"#,
            r#"{"command":"ime_preedit"}"#,
            r#"{"command":"clipboard_set"}"#,
            r#"{"command":"clipboard_restore","extra":1}"#,
        ] {
            assert!(parse_command_text(text).is_err(), "{text}");
        }
        assert!(parse_command_text(r#"{"command":"resize","width":8192,"height":1}"#).is_ok());
    }

    #[test]
    fn preedit_cursor_must_fit_the_text_on_character_boundaries() {
        for text in [
            r#"{"command":"ime_preedit","text":"に","cursor":[2,1]}"#,
            r#"{"command":"ime_preedit","text":"に","cursor":[0,4]}"#,
            r#"{"command":"ime_preedit","text":"に","cursor":[1,3]}"#,
            r#"{"command":"ime_preedit","text":"に","cursor":[0,2]}"#,
            r#"{"command":"ime_preedit","text":"a","cursor":[0,1,2]}"#,
            r#"{"command":"ime_preedit","text":"a","cursor":[-1,0]}"#,
        ] {
            assert!(parse_command_text(text).is_err(), "{text}");
        }
        let message = parse_command_text(r#"{"command":"ime_preedit","text":"に","cursor":[1,3]}"#)
            .expect_err("a cursor inside a character should fail")
            .to_string();
        assert!(message.contains("character boundaries"), "{message}");
        assert!(
            parse_command_text(r#"{"command":"ime_preedit","text":"","cursor":[0,0]}"#).is_ok()
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

    fn cell_target(column: u32, row: u32) -> DriverTarget {
        let arguments = serde_json::json!({"column": column, "row": row});
        DriverTarget {
            target: "cell".to_owned(),
            arguments: arguments.as_object().cloned().unwrap_or_default(),
        }
    }

    #[test]
    fn parses_at_on_click_mouse_move_and_touch() {
        let at = r#""at":{"target":"cell","column":3,"row":5}"#;
        assert_eq!(
            parse(&format!(r#"{{"command":"click",{at}}}"#)),
            DriverCommand::Click {
                button: MouseButton::Left,
                x: None,
                y: None,
                at: Some(cell_target(3, 5))
            }
        );
        assert_eq!(
            parse(&format!(r#"{{"command":"mouse_move",{at}}}"#)),
            DriverCommand::MouseMove {
                x: None,
                y: None,
                at: Some(cell_target(3, 5))
            }
        );
        assert_eq!(
            parse(&format!(r#"{{"command":"touch","phase":"started",{at}}}"#)),
            DriverCommand::Touch {
                phase: TouchCommandPhase::Started,
                x: None,
                y: None,
                at: Some(cell_target(3, 5)),
                id: 0
            }
        );
    }

    #[test]
    fn the_target_arguments_exclude_the_target_name() {
        let command = parse(r#"{"command":"click","at":{"target":"cell","column":3,"row":5}}"#);
        let DriverCommand::Click { at: Some(at), .. } = command else {
            panic!("click should carry a target");
        };
        assert_eq!(at.target, "cell");
        assert_eq!(
            Value::Object(at.arguments),
            serde_json::json!({"column": 3, "row": 5})
        );
    }

    #[test]
    fn drag_ends_accept_arrays_and_targets_mixed() {
        assert_eq!(
            parse(
                r#"{"command":"drag","from":[1,2],"to":{"target":"cell","column":3,"row":5},"frames":4}"#
            ),
            DriverCommand::Drag {
                from: PointerPosition::Pixels([1.0, 2.0]),
                to: PointerPosition::Target(cell_target(3, 5)),
                button: MouseButton::Left,
                frames: 4
            }
        );
        assert_eq!(
            parse(
                r#"{"command":"touch_drag","from":{"target":"cell","column":3,"row":5},"to":[9,9]}"#
            ),
            DriverCommand::TouchDrag {
                from: PointerPosition::Target(cell_target(3, 5)),
                to: PointerPosition::Pixels([9.0, 9.0]),
                id: 0,
                frames: 10
            }
        );
    }

    #[test]
    fn rejects_unusable_positions() {
        let conflict = parse_command_text(r#"{"command":"click","x":1,"at":{"target":"cell"}}"#)
            .expect_err("at with x should fail");
        assert!(matches!(
            conflict,
            CommandParseError::ConflictingPosition { command: "click" }
        ));
        assert!(conflict.to_string().contains("not both"), "{conflict}");
        for text in [
            r#"{"command":"mouse_move","y":1,"at":{"target":"cell"}}"#,
            r#"{"command":"touch","phase":"started","x":1,"y":2,"at":{"target":"cell"}}"#,
            r#"{"command":"mouse_move"}"#,
            r#"{"command":"mouse_move","x":1}"#,
            r#"{"command":"touch","phase":"started"}"#,
            r#"{"command":"click","y":2}"#,
            r#"{"command":"click","at":{"column":3}}"#,
            r#"{"command":"drag","from":{"column":3},"to":[1,1]}"#,
            r#"{"command":"drag","from":"here","to":[1,1]}"#,
            r#"{"command":"drag","from":[1],"to":[1,1]}"#,
        ] {
            assert!(parse_command_text(text).is_err(), "{text}");
        }
        let missing_target = parse_command_text(r#"{"command":"click","at":{"column":3}}"#)
            .expect_err("at without target should fail")
            .to_string();
        assert!(missing_target.contains("target"), "{missing_target}");
        let wrong_type = parse_command_text(r#"{"command":"drag","from":7,"to":[1,1]}"#)
            .expect_err("a number is not a position")
            .to_string();
        assert!(wrong_type.contains("[x, y]"), "{wrong_type}");
    }

    #[test]
    fn click_may_have_no_position_at_all() {
        assert_eq!(
            parse(r#"{"command":"click"}"#),
            DriverCommand::Click {
                button: MouseButton::Left,
                x: None,
                y: None,
                at: None
            }
        );
    }

    #[test]
    fn resolving_replaces_targets_with_pixels_in_reading_order() {
        let command = parse(
            r#"{"command":"drag","from":{"target":"cell","column":1,"row":1},"to":{"target":"cell","column":2,"row":2}}"#,
        );
        assert!(command.has_targets());
        let mut asked = Vec::new();
        let resolved = command
            .resolve_targets(|target| -> Result<Vec2, String> {
                asked.push(target.arguments["column"].as_u64().unwrap_or_default());
                Ok(Vec2::new(asked.len() as f32, 0.0))
            })
            .expect("resolution should succeed");
        assert_eq!(asked, vec![1, 2]);
        assert!(!resolved.has_targets());
        assert_eq!(
            resolved,
            DriverCommand::Drag {
                from: PointerPosition::Pixels([1.0, 0.0]),
                to: PointerPosition::Pixels([2.0, 0.0]),
                button: MouseButton::Left,
                frames: 10
            }
        );
    }

    #[test]
    fn rejects_the_old_command_and_millisecond_keys() {
        assert!(parse_command_text(r#"{"cmd":"quit"}"#).is_err());
        assert!(parse_command_text(r#"{"command":"time_fixed_step","ms":16}"#).is_err());
    }
}
