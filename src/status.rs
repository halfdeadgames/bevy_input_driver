//! Observable driver state, served by `drive/status`.

use bevy::{
    input::{keyboard::KeyCode, mouse::MouseButton, touch::TouchPhase},
    log::error,
    math::Vec2,
    prelude::{Resource, World},
};
use serde::Serialize;

use crate::{render_readiness::RenderReadiness, steps::DriverQueue};

const MAX_RECORDED_ERRORS: usize = 50;
const MAX_RECORDED_STATE_OPERATIONS: usize = 50;

/// A finger the driver put down and has not lifted.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct HeldTouch {
    /// Finger identifier.
    pub id: u64,
    /// Last logical position, used when the touch is canceled.
    pub position: [f32; 2],
}

/// What the driver has done and what it is still doing.
#[derive(Resource, Debug, Default, Serialize)]
pub struct DriverStatus {
    /// Frames the executor has run.
    pub frame_count: u64,
    /// Steps still waiting in the queue.
    pub queue_length: usize,
    /// True when the queue is empty, no wait is active, and no screenshot is pending.
    pub idle: bool,
    /// Driver cursor in logical pixels, once it has moved.
    pub cursor_position: Option<[f32; 2]>,
    /// Keys the driver pressed and has not released.
    pub held_keys: Vec<KeyCode>,
    /// Mouse buttons the driver pressed and has not released.
    pub held_buttons: Vec<MouseButton>,
    /// Fingers the driver started and has not ended or canceled.
    pub held_touches: Vec<HeldTouch>,
    /// Screenshot paths requested but not yet written.
    pub pending_screenshots: Vec<String>,
    /// Screenshot paths written successfully.
    pub screenshots_saved: Vec<String>,
    /// The most recent 50 errors.
    pub errors: Vec<String>,
    /// Path of the launch script that was loaded, if any.
    pub script_loaded: Option<String>,
    /// The seed in `DriverSeed`, if one is set.
    pub seed: Option<u64>,
    /// Whether `Time<Virtual>` is paused. Refreshed once per frame.
    pub paused: bool,
    /// Relative speed of `Time<Virtual>`. Refreshed once per frame, so it reads 0 until the
    /// first frame has finished.
    pub speed: f32,
    /// Milliseconds every frame advances by under `time_fixed_step`, or none under real time.
    pub fixed_step_ms: Option<f64>,
    /// Seconds elapsed on `Time<Virtual>`. Refreshed once per frame.
    pub virtual_elapsed_seconds: f64,
    /// The most recent 50 finished `save_state` and `load_state` operations.
    pub state_operations: Vec<String>,
    /// True once the render pipelines in use have compiled and stayed steady for 3 frames,
    /// the condition `wait_until_drawn` waits for unless it gives another `settle_frames`. Refreshed once per frame; always false
    /// without a renderer.
    pub render_ready: bool,
    /// Render pipelines still queued or compiling. Refreshed once per frame.
    pub pipelines_pending: usize,
}

/// Logs an error and records it in [`DriverStatus`] from code that only has the world.
pub fn record_error_in_world(world: &mut World, message: String) {
    match world.get_resource_mut::<DriverStatus>() {
        Some(mut status) => status.record_error(message),
        None => error!("input driver: {message}"),
    }
}

impl DriverStatus {
    /// Records a finished `save_state` or `load_state`, dropping the oldest past 50.
    pub fn note_state_operation(&mut self, description: String) {
        self.state_operations.push(description);
        let excess = self
            .state_operations
            .len()
            .saturating_sub(MAX_RECORDED_STATE_OPERATIONS);
        self.state_operations.drain(..excess);
    }
}

impl DriverStatus {
    /// Copies the latest pipeline compilation progress, or clears it without a renderer.
    pub fn refresh_render_readiness(&mut self, readiness: Option<&RenderReadiness>) {
        self.render_ready = readiness.is_some_and(RenderReadiness::is_drawn);
        self.pipelines_pending = readiness.map_or(0, RenderReadiness::pending_pipelines);
    }

    /// Copies queue-derived fields so a reader sees the queue as it is right now.
    pub fn synchronize_with_queue(&mut self, queue: &DriverQueue) {
        self.queue_length = queue.step_count();
        self.idle = queue.is_idle() && self.pending_screenshots.is_empty();
    }

    /// Logs an error and keeps it in [`DriverStatus::errors`], dropping the oldest past 50.
    pub fn record_error(&mut self, message: impl Into<String>) {
        let message = message.into();
        error!("input driver: {message}");
        self.errors.push(message);
        let excess = self.errors.len().saturating_sub(MAX_RECORDED_ERRORS);
        self.errors.drain(..excess);
    }

    /// The driver cursor as a vector.
    #[must_use]
    pub fn cursor(&self) -> Option<Vec2> {
        self.cursor_position.map(Vec2::from)
    }

    /// Remembers the driver cursor.
    pub fn set_cursor(&mut self, position: Vec2) {
        self.cursor_position = Some(position.to_array());
    }

    /// Records that the driver pressed `key`.
    pub fn note_key_down(&mut self, key: KeyCode) {
        if !self.held_keys.contains(&key) {
            self.held_keys.push(key);
        }
    }

    /// Records that the driver released `key`.
    pub fn note_key_up(&mut self, key: KeyCode) {
        self.held_keys.retain(|held| *held != key);
    }

    /// Records that the driver pressed `button`.
    pub fn note_button_down(&mut self, button: MouseButton) {
        if !self.held_buttons.contains(&button) {
            self.held_buttons.push(button);
        }
    }

    /// Records that the driver released `button`.
    pub fn note_button_up(&mut self, button: MouseButton) {
        self.held_buttons.retain(|held| *held != button);
    }

    /// Records that the driver touched, moved, or lifted the finger `id` at `position`.
    pub fn note_touch(&mut self, phase: TouchPhase, id: u64, position: Vec2) {
        let existing = self.held_touches.iter().position(|touch| touch.id == id);
        match (phase, existing) {
            (TouchPhase::Started | TouchPhase::Moved, Some(index)) => {
                self.held_touches[index].position = position.to_array();
            }
            (TouchPhase::Started, None) => self.held_touches.push(HeldTouch {
                id,
                position: position.to_array(),
            }),
            (TouchPhase::Ended | TouchPhase::Canceled, Some(index)) => {
                self.held_touches.remove(index);
            }
            (TouchPhase::Moved | TouchPhase::Ended | TouchPhase::Canceled, None) => {}
        }
    }

    /// Records a finished screenshot and removes one matching pending entry.
    pub fn note_screenshot_saved(&mut self, path: &str) {
        self.remove_one_pending_screenshot(path);
        self.screenshots_saved.push(path.to_owned());
    }

    /// Removes one matching screenshot from the pending list after it failed.
    pub fn note_screenshot_failed(&mut self, path: &str, problem: &str) {
        self.remove_one_pending_screenshot(path);
        self.record_error(format!("screenshot {path}: {problem}"));
    }

    // Two requests for one path are two captures, so each result clears only one entry.
    fn remove_one_pending_screenshot(&mut self, path: &str) {
        if let Some(index) = self
            .pending_screenshots
            .iter()
            .position(|pending| pending == path)
        {
            self.pending_screenshots.remove(index);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_screenshot_result_clears_only_one_pending_entry() {
        let mut status = DriverStatus {
            pending_screenshots: vec!["a.png".to_owned(), "a.png".to_owned()],
            ..DriverStatus::default()
        };
        status.note_screenshot_saved("a.png");
        assert_eq!(status.pending_screenshots, vec!["a.png".to_owned()]);
        status.note_screenshot_failed("a.png", "disk full");
        assert!(status.pending_screenshots.is_empty());
        assert_eq!(status.screenshots_saved, vec!["a.png".to_owned()]);
    }

    #[test]
    fn touches_are_held_from_start_until_end_or_cancel() {
        let mut status = DriverStatus::default();
        status.note_touch(TouchPhase::Started, 1, Vec2::new(1.0, 2.0));
        status.note_touch(TouchPhase::Started, 2, Vec2::ZERO);
        status.note_touch(TouchPhase::Moved, 1, Vec2::new(5.0, 6.0));
        assert_eq!(status.held_touches.len(), 2);
        assert_eq!(status.held_touches[0].position, [5.0, 6.0]);
        status.note_touch(TouchPhase::Ended, 1, Vec2::ZERO);
        status.note_touch(TouchPhase::Canceled, 2, Vec2::ZERO);
        assert!(status.held_touches.is_empty());
    }

    #[test]
    fn keeps_only_the_last_fifty_errors() {
        let mut status = DriverStatus::default();
        for index in 0..60 {
            status.record_error(format!("error {index}"));
        }
        assert_eq!(status.errors.len(), 50);
        assert_eq!(status.errors.first().map(String::as_str), Some("error 10"));
    }
}
