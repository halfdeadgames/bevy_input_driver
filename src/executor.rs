//! The system that runs queued steps, one frame at a time.

use std::time::Instant;

use bevy::{
    app::AppExit,
    input::{ButtonState, InputSystems, touch::TouchPhase},
    prelude::*,
};

use crate::{
    injection::{InjectionError, InputInjector},
    plugin::DriverSettings,
    screenshot::request_screenshot,
    seed::queue_set_seed,
    state_handlers::queue_state_operation,
    status::DriverStatus,
    steps::{DriverQueue, DriverStep},
    time_control::{TimeControlAction, queue_time_action},
};

/// Registers the queue, the status, and the executor system.
pub struct DriverExecutorPlugin;

impl Plugin for DriverExecutorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DriverQueue>()
            .init_resource::<DriverStatus>()
            .add_systems(PreUpdate, execute_driver_steps.before(InputSystems));
    }
}

/// Runs steps until a barrier or wait, so injected input is read by `InputSystems` this frame.
pub fn execute_driver_steps(
    mut queue: ResMut<DriverQueue>,
    mut status: ResMut<DriverStatus>,
    settings: Res<DriverSettings>,
    mut injector: InputInjector,
    mut commands: Commands,
    mut app_exit: MessageWriter<AppExit>,
) {
    status.frame_count += 1;
    // Wall-clock time, because `time_fixed_step` makes `Time<Real>` advance by a fixed amount.
    let now = Instant::now();
    if !queue.is_still_waiting(now) {
        while let Some(step) = queue.pop_next_step() {
            match step {
                DriverStep::EndFrame => break,
                DriverStep::BeginSteppedFrames => {
                    queue_time_action(&mut commands, TimeControlAction::Resume);
                    queue.set_stepping_frames(true);
                }
                DriverStep::EndSteppedFrames => {
                    queue_time_action(&mut commands, TimeControlAction::Pause);
                    queue.set_stepping_frames(false);
                }
                DriverStep::WaitFrames(frames) => {
                    if queue.begin_frame_wait(frames) {
                        break;
                    }
                }
                DriverStep::WaitSeconds(seconds) => {
                    if queue.begin_timed_wait(now, seconds) {
                        break;
                    }
                }
                input_step => {
                    if let Err(problem) = apply_step(
                        &input_step,
                        &mut injector,
                        &mut status,
                        &mut commands,
                        &mut app_exit,
                        settings.update_window_cursor_position,
                    ) {
                        status.record_error(problem.to_string());
                    }
                }
            }
        }
    }
    status.synchronize_with_queue(&queue);
}

fn apply_step(
    step: &DriverStep,
    injector: &mut InputInjector,
    status: &mut DriverStatus,
    commands: &mut Commands,
    app_exit: &mut MessageWriter<AppExit>,
    update_window_cursor_position: bool,
) -> Result<(), InjectionError> {
    match step {
        DriverStep::KeyDown { key, text } => {
            injector.key(*key, text.as_deref(), ButtonState::Pressed)?;
            status.note_key_down(*key);
        }
        DriverStep::KeyUp { key } => {
            injector.key(*key, None, ButtonState::Released)?;
            status.note_key_up(*key);
        }
        DriverStep::MouseMove { position } => {
            injector.move_cursor(*position, status.cursor(), update_window_cursor_position)?;
            status.set_cursor(*position);
        }
        DriverStep::MouseDown { button } => {
            injector.mouse_button(*button, ButtonState::Pressed)?;
            status.note_button_down(*button);
        }
        DriverStep::MouseUp { button } => {
            injector.mouse_button(*button, ButtonState::Released)?;
            status.note_button_up(*button);
        }
        DriverStep::Scroll { unit, x, y } => injector.scroll(*unit, *x, *y)?,
        DriverStep::Touch {
            phase,
            position,
            id,
        } => {
            injector.touch(*phase, *position, *id)?;
            status.note_touch(*phase, *id, *position);
        }
        DriverStep::Pinch { delta } => injector.pinch(*delta),
        DriverStep::Rotate { delta } => injector.rotate(*delta),
        DriverStep::Pan { delta } => injector.pan(*delta),
        DriverStep::DoubleTap => injector.double_tap(),
        DriverStep::Screenshot { path } => request_screenshot(commands, status, path.clone()),
        DriverStep::ReleaseAll => release_everything_held(injector, status)?,
        DriverStep::Quit => {
            app_exit.write(AppExit::Success);
        }
        DriverStep::SetSeed(seed) => queue_set_seed(commands, *seed),
        DriverStep::Time(action) => queue_time_action(commands, *action),
        DriverStep::State { operation, path } => {
            queue_state_operation(commands, *operation, path.clone());
        }
        DriverStep::EndFrame
        | DriverStep::WaitFrames(_)
        | DriverStep::WaitSeconds(_)
        | DriverStep::BeginSteppedFrames
        | DriverStep::EndSteppedFrames => {}
    }
    Ok(())
}

// Each item leaves its list only after its release was written, so a failure keeps the rest.
fn release_everything_held(
    injector: &mut InputInjector,
    status: &mut DriverStatus,
) -> Result<(), InjectionError> {
    while let Some(key) = status.held_keys.first().copied() {
        injector.key(key, None, ButtonState::Released)?;
        status.note_key_up(key);
    }
    while let Some(button) = status.held_buttons.first().copied() {
        injector.mouse_button(button, ButtonState::Released)?;
        status.note_button_up(button);
    }
    while let Some(touch) = status.held_touches.first().copied() {
        let position = Vec2::from(touch.position);
        injector.touch(TouchPhase::Canceled, position, touch.id)?;
        status.note_touch(TouchPhase::Canceled, touch.id, position);
    }
    Ok(())
}
