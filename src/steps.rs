//! Expanding commands into per-frame steps, and the queue that holds them.

use std::collections::VecDeque;
use std::iter::repeat_n;
use std::time::{Duration, Instant};

use bevy::{
    input::{
        keyboard::{KeyCode, NativeKeyCode},
        mouse::{MouseButton, MouseScrollUnit},
        touch::TouchPhase,
    },
    math::Vec2,
    prelude::Resource,
};

use crate::{
    commands::{DriverCommand, ScrollUnit, TouchCommandPhase},
    state_handlers::StateOperation,
    time_control::TimeControlAction,
};

const CHARACTER_KEYS: [(char, KeyCode); 36] = [
    ('a', KeyCode::KeyA),
    ('b', KeyCode::KeyB),
    ('c', KeyCode::KeyC),
    ('d', KeyCode::KeyD),
    ('e', KeyCode::KeyE),
    ('f', KeyCode::KeyF),
    ('g', KeyCode::KeyG),
    ('h', KeyCode::KeyH),
    ('i', KeyCode::KeyI),
    ('j', KeyCode::KeyJ),
    ('k', KeyCode::KeyK),
    ('l', KeyCode::KeyL),
    ('m', KeyCode::KeyM),
    ('n', KeyCode::KeyN),
    ('o', KeyCode::KeyO),
    ('p', KeyCode::KeyP),
    ('q', KeyCode::KeyQ),
    ('r', KeyCode::KeyR),
    ('s', KeyCode::KeyS),
    ('t', KeyCode::KeyT),
    ('u', KeyCode::KeyU),
    ('v', KeyCode::KeyV),
    ('w', KeyCode::KeyW),
    ('x', KeyCode::KeyX),
    ('y', KeyCode::KeyY),
    ('z', KeyCode::KeyZ),
    ('0', KeyCode::Digit0),
    ('1', KeyCode::Digit1),
    ('2', KeyCode::Digit2),
    ('3', KeyCode::Digit3),
    ('4', KeyCode::Digit4),
    ('5', KeyCode::Digit5),
    ('6', KeyCode::Digit6),
    ('7', KeyCode::Digit7),
    ('8', KeyCode::Digit8),
    ('9', KeyCode::Digit9),
];

/// Maps a typed character to the physical key that produces it, or an unidentified key.
#[must_use]
pub fn key_code_for_character(character: char) -> KeyCode {
    match character {
        ' ' => KeyCode::Space,
        '\n' => KeyCode::Enter,
        other => {
            let lowercase = other.to_ascii_lowercase();
            CHARACTER_KEYS
                .iter()
                .find(|(candidate, _)| *candidate == lowercase)
                .map_or(
                    KeyCode::Unidentified(NativeKeyCode::Unidentified),
                    |(_, key)| *key,
                )
        }
    }
}

/// The unshifted character a letter or digit key produces.
#[must_use]
pub fn character_for_key_code(key_code: KeyCode) -> Option<char> {
    CHARACTER_KEYS
        .iter()
        .find(|(_, candidate)| *candidate == key_code)
        .map(|(character, _)| *character)
}

/// One primitive action the executor performs.
#[derive(Debug, Clone, PartialEq)]
pub enum DriverStep {
    /// Press a key.
    KeyDown {
        /// Physical key.
        key: KeyCode,
        /// Text the key produces.
        text: Option<String>,
    },
    /// Release a key.
    KeyUp {
        /// Physical key.
        key: KeyCode,
    },
    /// Move the cursor.
    MouseMove {
        /// Logical position.
        position: Vec2,
    },
    /// Press a mouse button.
    MouseDown {
        /// Button.
        button: MouseButton,
    },
    /// Release a mouse button.
    MouseUp {
        /// Button.
        button: MouseButton,
    },
    /// Scroll the wheel.
    Scroll {
        /// Unit of the amounts.
        unit: MouseScrollUnit,
        /// Horizontal amount.
        x: f32,
        /// Vertical amount.
        y: f32,
    },
    /// Send a touch event.
    Touch {
        /// Phase.
        phase: TouchPhase,
        /// Logical position.
        position: Vec2,
        /// Finger identifier.
        id: u64,
    },
    /// Pinch gesture.
    Pinch {
        /// Amount.
        delta: f32,
    },
    /// Rotation gesture.
    Rotate {
        /// Amount.
        delta: f32,
    },
    /// Pan gesture.
    Pan {
        /// Amount.
        delta: Vec2,
    },
    /// Double tap gesture.
    DoubleTap,
    /// Capture the primary window.
    Screenshot {
        /// Output path.
        path: String,
    },
    /// Release everything the driver holds.
    ReleaseAll,
    /// Exit the app.
    Quit,
    /// Make this the current `DriverSeed`.
    SetSeed(u64),
    /// Change the game clock.
    Time(TimeControlAction),
    /// Run the save or load hook.
    State {
        /// Which hook to run.
        operation: StateOperation,
        /// File path handed to the hook.
        path: String,
    },
    /// Stop executing until the next frame so game systems see the steps so far.
    EndFrame,
    /// Skip this many frames.
    WaitFrames(u32),
    /// Skip frames until this many wall-clock seconds have passed.
    WaitSeconds(f32),
    /// Skip frames until the render pipelines in use are compiled, for at most this many
    /// wall-clock seconds.
    WaitUntilDrawn {
        /// Wall-clock seconds before giving up.
        timeout_seconds: f32,
        /// Render frames in a row with nothing compiling that count as drawn.
        settle_frames: u32,
    },
    /// Unpause game time for a `step_frames` command and remember that it is in flight.
    BeginSteppedFrames,
    /// Pause game time again at the end of a `step_frames` command.
    EndSteppedFrames,
}

fn then_end_frame(step: DriverStep) -> Vec<DriverStep> {
    vec![step, DriverStep::EndFrame]
}

// `bevy_picking` reads `WindowEvent`s in `First` of the frame after they are written, so its
// result appears one frame later than the input. One more barrier lets the next command see it.
fn with_picking_barrier(mut steps: Vec<DriverStep>) -> Vec<DriverStep> {
    steps.push(DriverStep::EndFrame);
    steps
}

fn bevy_touch_phase(phase: TouchCommandPhase) -> TouchPhase {
    match phase {
        TouchCommandPhase::Started => TouchPhase::Started,
        TouchCommandPhase::Moved => TouchPhase::Moved,
        TouchCommandPhase::Ended => TouchPhase::Ended,
        TouchCommandPhase::Canceled => TouchPhase::Canceled,
    }
}

/// Positions of `frames` evenly spaced moves from `from`, the last exactly at `to`.
fn drag_path(from: Vec2, to: Vec2, frames: u32) -> Vec<Vec2> {
    let frames = frames.max(1);
    (1..=frames)
        .map(|index| {
            if index == frames {
                to
            } else {
                from.lerp(to, index as f32 / frames as f32)
            }
        })
        .collect()
}

fn mouse_drag_steps(from: Vec2, to: Vec2, button: MouseButton, frames: u32) -> Vec<DriverStep> {
    let mut steps = Vec::new();
    steps.extend(then_end_frame(DriverStep::MouseMove { position: from }));
    steps.extend(then_end_frame(DriverStep::MouseDown { button }));
    for position in drag_path(from, to, frames) {
        steps.extend(then_end_frame(DriverStep::MouseMove { position }));
    }
    steps.extend(then_end_frame(DriverStep::MouseUp { button }));
    with_picking_barrier(steps)
}

fn touch_drag_steps(from: Vec2, to: Vec2, id: u64, frames: u32) -> Vec<DriverStep> {
    let touch = |phase, position| DriverStep::Touch {
        phase,
        position,
        id,
    };
    let mut steps = Vec::new();
    steps.extend(then_end_frame(touch(TouchPhase::Started, from)));
    for position in drag_path(from, to, frames) {
        steps.extend(then_end_frame(touch(TouchPhase::Moved, position)));
    }
    steps.extend(then_end_frame(touch(TouchPhase::Ended, to)));
    with_picking_barrier(steps)
}

fn key_tap_steps(key: KeyCode, text: Option<&str>, hold_frames: u32) -> Vec<DriverStep> {
    let mut steps = vec![DriverStep::KeyDown {
        key,
        text: text.map(str::to_owned),
    }];
    steps.extend(repeat_n(DriverStep::EndFrame, hold_frames.max(1) as usize));
    steps.extend(then_end_frame(DriverStep::KeyUp { key }));
    steps
}

fn type_text_steps(text: &str) -> Vec<DriverStep> {
    text.chars()
        .flat_map(|character| {
            let key = key_code_for_character(character);
            key_tap_steps(key, Some(&character.to_string()), 1)
        })
        .collect()
}

fn click_steps(button: MouseButton, position: Option<Vec2>) -> Vec<DriverStep> {
    let mut steps = Vec::new();
    if let Some(position) = position {
        steps.extend(then_end_frame(DriverStep::MouseMove { position }));
    }
    steps.extend(then_end_frame(DriverStep::MouseDown { button }));
    steps.extend(then_end_frame(DriverStep::MouseUp { button }));
    with_picking_barrier(steps)
}

/// Expands one command into primitive steps. Every input step is followed by an
/// [`DriverStep::EndFrame`] so game systems see it for at least one frame.
#[must_use]
pub fn expand_command(command: &DriverCommand) -> Vec<DriverStep> {
    match command {
        DriverCommand::KeyDown { key, text } => then_end_frame(DriverStep::KeyDown {
            key: *key,
            text: text.clone(),
        }),
        DriverCommand::KeyUp { key } => then_end_frame(DriverStep::KeyUp { key: *key }),
        DriverCommand::KeyTap {
            key,
            text,
            hold_frames,
        } => key_tap_steps(*key, text.as_deref(), *hold_frames),
        DriverCommand::TypeText { text } => type_text_steps(text),
        DriverCommand::MouseMove { x, y } => then_end_frame(DriverStep::MouseMove {
            position: Vec2::new(*x, *y),
        }),
        DriverCommand::MouseDown { button } => {
            then_end_frame(DriverStep::MouseDown { button: *button })
        }
        DriverCommand::MouseUp { button } => {
            then_end_frame(DriverStep::MouseUp { button: *button })
        }
        DriverCommand::Click { button, x, y } => {
            let position = x.zip(*y).map(|(x, y)| Vec2::new(x, y));
            click_steps(*button, position)
        }
        DriverCommand::Drag {
            from,
            to,
            button,
            frames,
        } => mouse_drag_steps(Vec2::from(*from), Vec2::from(*to), *button, *frames),
        DriverCommand::Scroll { x, y, unit } => {
            with_picking_barrier(then_end_frame(DriverStep::Scroll {
                unit: match unit {
                    ScrollUnit::Line => MouseScrollUnit::Line,
                    ScrollUnit::Pixel => MouseScrollUnit::Pixel,
                },
                x: *x,
                y: *y,
            }))
        }
        DriverCommand::Touch { phase, x, y, id } => {
            with_picking_barrier(then_end_frame(DriverStep::Touch {
                phase: bevy_touch_phase(*phase),
                position: Vec2::new(*x, *y),
                id: *id,
            }))
        }
        DriverCommand::TouchDrag {
            from,
            to,
            id,
            frames,
        } => touch_drag_steps(Vec2::from(*from), Vec2::from(*to), *id, *frames),
        DriverCommand::Pinch { delta } => then_end_frame(DriverStep::Pinch { delta: *delta }),
        DriverCommand::Rotate { delta } => then_end_frame(DriverStep::Rotate { delta: *delta }),
        DriverCommand::Pan { x, y } => then_end_frame(DriverStep::Pan {
            delta: Vec2::new(*x, *y),
        }),
        DriverCommand::DoubleTap {} => then_end_frame(DriverStep::DoubleTap),
        DriverCommand::WaitFrames { frames } => vec![DriverStep::WaitFrames(*frames)],
        DriverCommand::WaitSeconds { seconds } => vec![DriverStep::WaitSeconds(*seconds)],
        // The barrier gives the game one more frame, so the first frame drawn with every
        // pipeline has been presented before the next command runs.
        DriverCommand::WaitUntilDrawn {
            timeout_seconds,
            settle_frames,
        } => then_end_frame(DriverStep::WaitUntilDrawn {
            timeout_seconds: *timeout_seconds,
            settle_frames: *settle_frames,
        }),
        DriverCommand::Screenshot { path } => {
            then_end_frame(DriverStep::Screenshot { path: path.clone() })
        }
        DriverCommand::ReleaseAll {} => then_end_frame(DriverStep::ReleaseAll),
        DriverCommand::Quit {} => then_end_frame(DriverStep::Quit),
        DriverCommand::SetSeed { seed } => then_end_frame(DriverStep::SetSeed(*seed)),
        DriverCommand::TimePause {} => time_steps(TimeControlAction::Pause),
        DriverCommand::TimeResume {} => time_steps(TimeControlAction::Resume),
        DriverCommand::TimeSpeed { factor } => time_steps(TimeControlAction::SetSpeed(*factor)),
        DriverCommand::TimeFixedStep { ms } => {
            time_steps(TimeControlAction::FixedStep { milliseconds: *ms })
        }
        DriverCommand::TimeReal {} => time_steps(TimeControlAction::RealTime),
        // Measured live (2026-11-25, 100 ms fixed step): waiting `frames - 1` advanced only
        // `frames - 1` steps, and stepping 1 frame advanced none, so the wait is the full count.
        DriverCommand::StepFrames { frames } => vec![
            DriverStep::BeginSteppedFrames,
            DriverStep::WaitFrames(*frames),
            DriverStep::EndSteppedFrames,
            DriverStep::EndFrame,
        ],
        DriverCommand::SaveState { path } => state_steps(StateOperation::Save, path),
        DriverCommand::LoadState { path } => state_steps(StateOperation::Load, path),
    }
}

fn time_steps(action: TimeControlAction) -> Vec<DriverStep> {
    then_end_frame(DriverStep::Time(action))
}

fn state_steps(operation: StateOperation, path: &str) -> Vec<DriverStep> {
    then_end_frame(DriverStep::State {
        operation,
        path: path.to_owned(),
    })
}

#[derive(Debug, Default)]
struct PendingWait {
    frames_remaining: u32,
    until: Option<Instant>,
    drawn_deadline: Option<Instant>,
    drawn_start_render_frame: u32,
    drawn_settle_frames: u32,
}

/// How a `wait_until_drawn` wait stands after one more frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrawnWaitProgress {
    /// No `wait_until_drawn` wait is active.
    NotWaiting,
    /// Pipelines are still compiling and the deadline has not passed.
    StillWaiting,
    /// The game is drawn, so the wait ended.
    Drawn,
    /// The deadline passed before the game was drawn, so the wait ended.
    TimedOut,
}

/// Steps waiting to run, plus the wait the executor is currently serving.
#[derive(Resource, Debug, Default)]
pub struct DriverQueue {
    steps: VecDeque<DriverStep>,
    wait: PendingWait,
    stepping_frames: bool,
}

impl DriverQueue {
    /// Appends the expansion of every command.
    pub fn enqueue_commands<'a>(&mut self, commands: impl IntoIterator<Item = &'a DriverCommand>) {
        for command in commands {
            self.steps.extend(expand_command(command));
        }
    }

    /// Drops every pending step and wait, then queues a release of everything held.
    ///
    /// When a `step_frames` command has unpaused time and not yet paused it again, the queue also
    /// pauses time again so the cleared command cannot leave the game running.
    pub fn clear_and_release_all(&mut self) {
        let was_stepping_frames = std::mem::take(&mut self.stepping_frames);
        self.steps.clear();
        self.wait = PendingWait::default();
        self.steps.push_back(DriverStep::ReleaseAll);
        if was_stepping_frames {
            self.steps
                .push_back(DriverStep::Time(TimeControlAction::Pause));
        }
        self.steps.push_back(DriverStep::EndFrame);
    }

    /// Records whether a `step_frames` command has unpaused time and not yet paused it again.
    pub fn set_stepping_frames(&mut self, stepping_frames: bool) {
        self.stepping_frames = stepping_frames;
    }

    /// Number of steps not yet run.
    #[must_use]
    pub fn step_count(&self) -> usize {
        self.steps.len()
    }

    /// True when no step is pending and no wait is active.
    #[must_use]
    pub fn is_idle(&self) -> bool {
        self.steps.is_empty()
            && self.wait.frames_remaining == 0
            && self.wait.until.is_none()
            && self.wait.drawn_deadline.is_none()
    }

    /// Removes and returns the next step.
    pub fn pop_next_step(&mut self) -> Option<DriverStep> {
        self.steps.pop_front()
    }

    /// Starts skipping frames. Returns false when `frames` is zero and no wait is needed.
    ///
    /// The frame that calls this counts as the first waited frame.
    pub fn begin_frame_wait(&mut self, frames: u32) -> bool {
        self.wait.frames_remaining = frames.saturating_sub(1);
        frames > 0
    }

    /// Starts waiting `seconds` of wall-clock time from `now`. Returns false when `seconds` is
    /// zero, negative, or too large to represent, and no wait is needed.
    pub fn begin_timed_wait(&mut self, now: Instant, seconds: f32) -> bool {
        if seconds <= 0.0 {
            return false;
        }
        let Some(until) = Duration::try_from_secs_f32(seconds)
            .ok()
            .and_then(|length| now.checked_add(length))
        else {
            return false;
        };
        self.wait.until = Some(until);
        true
    }

    /// Starts waiting until the game is drawn, giving up `timeout_seconds` of wall-clock time
    /// after `now`. Parsing caps the timeout at 3600 seconds, so the deadline always fits.
    ///
    /// The wait ends once `settle_frames` render frames in a row were quiet (see
    /// [`RenderReadiness::quiet_render_frames`](crate::RenderReadiness::quiet_render_frames)).
    /// `render_frame` is the render frame counter now; the wait needs more than `settle_frames`
    /// render frames after it, so a quiet run left over from before the command (such as before
    /// a `load_state`) cannot end it.
    pub fn begin_drawn_wait(
        &mut self,
        now: Instant,
        timeout_seconds: f32,
        settle_frames: u32,
        render_frame: u32,
    ) {
        self.wait.drawn_start_render_frame = render_frame;
        self.wait.drawn_settle_frames = settle_frames;
        let length = Duration::try_from_secs_f32(timeout_seconds).unwrap_or(Duration::ZERO);
        let deadline = now.checked_add(length).unwrap_or(now);
        self.wait.drawn_deadline = Some(deadline);
    }

    /// Checks an active `wait_until_drawn` wait against the latest render state. Read
    /// `render_frame` before `quiet_render_frames`.
    pub fn poll_drawn_wait(
        &mut self,
        now: Instant,
        render_frame: u32,
        quiet_render_frames: u32,
    ) -> DrawnWaitProgress {
        let Some(deadline) = self.wait.drawn_deadline else {
            return DrawnWaitProgress::NotWaiting;
        };
        let settle_frames = self.wait.drawn_settle_frames;
        let render_frames_since_start =
            render_frame.wrapping_sub(self.wait.drawn_start_render_frame);
        let progress =
            if quiet_render_frames >= settle_frames && render_frames_since_start > settle_frames {
                DrawnWaitProgress::Drawn
            } else if now >= deadline {
                DrawnWaitProgress::TimedOut
            } else {
                return DrawnWaitProgress::StillWaiting;
            };
        self.wait.drawn_deadline = None;
        progress
    }

    /// Advances an active wait by one frame and reports whether the executor must stay idle.
    pub fn is_still_waiting(&mut self, now: Instant) -> bool {
        if self.wait.frames_remaining > 0 {
            self.wait.frames_remaining -= 1;
            return true;
        }
        if let Some(until) = self.wait.until {
            if now < until {
                return true;
            }
            self.wait.until = None;
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{commands::parse_command_text, render_readiness::SETTLED_RENDER_FRAMES};

    fn expand(text: &str) -> Vec<DriverStep> {
        expand_command(&parse_command_text(text).expect("command should parse"))
    }

    #[test]
    fn key_tap_has_down_barriers_up_barrier() {
        let steps = expand(r#"{"cmd":"key_tap","key":"KeyA","hold_frames":3}"#);
        assert_eq!(
            steps,
            vec![
                DriverStep::KeyDown {
                    key: KeyCode::KeyA,
                    text: None
                },
                DriverStep::EndFrame,
                DriverStep::EndFrame,
                DriverStep::EndFrame,
                DriverStep::KeyUp { key: KeyCode::KeyA },
                DriverStep::EndFrame,
            ]
        );
    }

    #[test]
    fn drag_moves_evenly_and_ends_exactly_at_target() {
        let steps = expand(r#"{"cmd":"drag","from":[0,0],"to":[100,50],"frames":4}"#);
        let moves: Vec<Vec2> = steps
            .iter()
            .filter_map(|step| match step {
                DriverStep::MouseMove { position } => Some(*position),
                _ => None,
            })
            .collect();
        assert_eq!(moves.len(), 5);
        assert_eq!(moves[0], Vec2::ZERO);
        assert_eq!(moves[1], Vec2::new(25.0, 12.5));
        assert_eq!(moves[4], Vec2::new(100.0, 50.0));
        let down = steps
            .iter()
            .position(|step| matches!(step, DriverStep::MouseDown { .. }));
        let up = steps
            .iter()
            .position(|step| matches!(step, DriverStep::MouseUp { .. }));
        assert!(down < up);
        assert_eq!(steps.last(), Some(&DriverStep::EndFrame));
    }

    #[test]
    fn touch_drag_starts_moves_and_ends() {
        let steps = expand(r#"{"cmd":"touch_drag","from":[1,1],"to":[3,3],"frames":2,"id":7}"#);
        let phases: Vec<TouchPhase> = steps
            .iter()
            .filter_map(|step| match step {
                DriverStep::Touch { phase, .. } => Some(*phase),
                _ => None,
            })
            .collect();
        assert_eq!(
            phases,
            vec![
                TouchPhase::Started,
                TouchPhase::Moved,
                TouchPhase::Moved,
                TouchPhase::Ended
            ]
        );
    }

    #[test]
    fn click_with_position_moves_first() {
        let steps = expand(r#"{"cmd":"click","x":5,"y":6}"#);
        assert_eq!(
            steps.first(),
            Some(&DriverStep::MouseMove {
                position: Vec2::new(5.0, 6.0)
            })
        );
        assert_eq!(steps.len(), 7);
    }

    #[test]
    fn type_text_maps_characters_to_keys_one_frame_each() {
        let steps = expand(r#"{"cmd":"type_text","text":"Ab 1\n!"}"#);
        let downs: Vec<(KeyCode, Option<String>)> = steps
            .iter()
            .filter_map(|step| match step {
                DriverStep::KeyDown { key, text } => Some((*key, text.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(
            downs,
            vec![
                (KeyCode::KeyA, Some("A".to_owned())),
                (KeyCode::KeyB, Some("b".to_owned())),
                (KeyCode::Space, Some(" ".to_owned())),
                (KeyCode::Digit1, Some("1".to_owned())),
                (KeyCode::Enter, Some("\n".to_owned())),
                (
                    KeyCode::Unidentified(NativeKeyCode::Unidentified),
                    Some("!".to_owned())
                ),
            ]
        );
        assert_eq!(steps.len(), downs.len() * 4);
    }

    #[test]
    fn step_frames_resumes_waits_then_pauses() {
        assert_eq!(
            expand(r#"{"cmd":"step_frames","frames":3}"#),
            vec![
                DriverStep::BeginSteppedFrames,
                DriverStep::WaitFrames(3),
                DriverStep::EndSteppedFrames,
                DriverStep::EndFrame,
            ]
        );
    }

    #[test]
    fn seed_time_and_state_commands_end_with_a_barrier() {
        assert_eq!(
            expand(r#"{"cmd":"set_seed","seed":9}"#),
            vec![DriverStep::SetSeed(9), DriverStep::EndFrame]
        );
        assert_eq!(
            expand(r#"{"cmd":"time_fixed_step","ms":20}"#),
            vec![
                DriverStep::Time(TimeControlAction::FixedStep { milliseconds: 20.0 }),
                DriverStep::EndFrame
            ]
        );
        assert_eq!(
            expand(r#"{"cmd":"time_speed","factor":2}"#),
            vec![
                DriverStep::Time(TimeControlAction::SetSpeed(2.0)),
                DriverStep::EndFrame
            ]
        );
        assert_eq!(
            expand(r#"{"cmd":"load_state","path":"slot.txt"}"#),
            vec![
                DriverStep::State {
                    operation: StateOperation::Load,
                    path: "slot.txt".to_owned()
                },
                DriverStep::EndFrame
            ]
        );
    }

    #[test]
    fn character_tables_round_trip() {
        for (character, key) in CHARACTER_KEYS {
            assert_eq!(key_code_for_character(character), key);
            assert_eq!(character_for_key_code(key), Some(character));
        }
    }

    #[test]
    fn frame_wait_lasts_the_requested_number_of_frames() {
        let mut queue = DriverQueue::default();
        let now = Instant::now();
        assert!(queue.begin_frame_wait(3));
        assert!(queue.is_still_waiting(now));
        assert!(queue.is_still_waiting(now));
        assert!(!queue.is_still_waiting(now));
    }

    #[test]
    fn timed_wait_ends_once_the_wall_clock_passes_the_deadline() {
        let mut queue = DriverQueue::default();
        let start = Instant::now();
        assert!(queue.begin_timed_wait(start, 2.0));
        assert!(!queue.is_idle());
        assert!(queue.is_still_waiting(start + Duration::from_secs(1)));
        assert!(!queue.is_still_waiting(start + Duration::from_secs(3)));
        assert!(queue.is_idle());
        assert!(!queue.begin_timed_wait(start, 0.0));
    }

    #[test]
    fn wait_until_drawn_ends_with_a_barrier() {
        assert_eq!(
            expand(r#"{"cmd":"wait_until_drawn","timeout_seconds":5}"#),
            vec![
                DriverStep::WaitUntilDrawn {
                    timeout_seconds: 5.0,
                    settle_frames: SETTLED_RENDER_FRAMES,
                },
                DriverStep::EndFrame
            ]
        );
    }

    #[test]
    fn drawn_wait_ends_when_drawn_or_when_the_deadline_passes() {
        let mut queue = DriverQueue::default();
        let start = Instant::now();
        let later = start + Duration::from_secs(1);
        let fresh_frame = 10 + SETTLED_RENDER_FRAMES + 1;
        assert_eq!(
            queue.poll_drawn_wait(start, fresh_frame, SETTLED_RENDER_FRAMES),
            DrawnWaitProgress::NotWaiting
        );
        queue.begin_drawn_wait(start, 2.0, SETTLED_RENDER_FRAMES, 10);
        assert!(!queue.is_idle());
        assert_eq!(
            queue.poll_drawn_wait(later, fresh_frame, SETTLED_RENDER_FRAMES - 1),
            DrawnWaitProgress::StillWaiting
        );
        assert_eq!(
            queue.poll_drawn_wait(later, fresh_frame, SETTLED_RENDER_FRAMES),
            DrawnWaitProgress::Drawn
        );
        assert!(queue.is_idle());
        queue.begin_drawn_wait(start, 2.0, SETTLED_RENDER_FRAMES, 10);
        assert_eq!(
            queue.poll_drawn_wait(start + Duration::from_secs(3), fresh_frame, 0),
            DrawnWaitProgress::TimedOut
        );
        assert!(queue.is_idle());
    }

    #[test]
    fn drawn_state_from_before_the_wait_does_not_end_it() {
        let mut queue = DriverQueue::default();
        let start = Instant::now();
        queue.begin_drawn_wait(start, 2.0, SETTLED_RENDER_FRAMES, u32::MAX - 1);
        assert_eq!(
            queue.poll_drawn_wait(
                start,
                (u32::MAX - 1).wrapping_add(SETTLED_RENDER_FRAMES),
                SETTLED_RENDER_FRAMES
            ),
            DrawnWaitProgress::StillWaiting
        );
        assert_eq!(
            queue.poll_drawn_wait(
                start,
                (u32::MAX - 1).wrapping_add(SETTLED_RENDER_FRAMES + 1),
                SETTLED_RENDER_FRAMES
            ),
            DrawnWaitProgress::Drawn
        );
    }

    #[test]
    fn settle_frames_sets_how_many_quiet_frames_end_the_wait() {
        let mut queue = DriverQueue::default();
        let start = Instant::now();
        queue.begin_drawn_wait(start, 2.0, 1, 10);
        assert_eq!(
            queue.poll_drawn_wait(start, 11, 1),
            DrawnWaitProgress::StillWaiting
        );
        assert_eq!(
            queue.poll_drawn_wait(start, 12, 0),
            DrawnWaitProgress::StillWaiting
        );
        assert_eq!(
            queue.poll_drawn_wait(start, 12, 1),
            DrawnWaitProgress::Drawn
        );
        queue.begin_drawn_wait(start, 2.0, 5, 10);
        assert_eq!(
            queue.poll_drawn_wait(start, 20, 4),
            DrawnWaitProgress::StillWaiting
        );
        assert_eq!(
            queue.poll_drawn_wait(start, 20, 5),
            DrawnWaitProgress::Drawn
        );
    }

    #[test]
    fn clearing_during_step_frames_queues_a_pause() {
        let mut queue = DriverQueue::default();
        queue.set_stepping_frames(true);
        queue.clear_and_release_all();
        assert_eq!(
            Vec::from(queue.steps.clone()),
            vec![
                DriverStep::ReleaseAll,
                DriverStep::Time(TimeControlAction::Pause),
                DriverStep::EndFrame
            ]
        );
        queue.clear_and_release_all();
        assert_eq!(queue.step_count(), 2);
    }

    #[test]
    fn pointer_commands_end_with_two_barriers() {
        for text in [
            r#"{"cmd":"click"}"#,
            r#"{"cmd":"scroll","y":1}"#,
            r#"{"cmd":"touch","phase":"ended","x":1,"y":1}"#,
        ] {
            let steps = expand(text);
            let tail = &steps[steps.len() - 2..];
            assert_eq!(tail, [DriverStep::EndFrame, DriverStep::EndFrame], "{text}");
        }
    }
}
