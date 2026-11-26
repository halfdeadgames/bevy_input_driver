//! The system that runs queued steps, one frame at a time.

use std::time::Instant;

use bevy::{
    app::AppExit,
    ecs::system::NonSendMarker,
    input::{ButtonState, InputSystems, touch::TouchPhase},
    prelude::*,
    window::PrimaryWindow,
};
use bevy_winit::WINIT_WINDOWS;

use crate::{
    injection::{InjectionError, InputInjector},
    plugin::DriverSettings,
    render_readiness::RenderReadiness,
    screenshot::request_screenshot,
    seed::queue_set_seed,
    state_handlers::queue_state_operation,
    status::DriverStatus,
    steps::{DrawnWaitProgress, DriverQueue, DriverStep},
    time_control::{TimeControlAction, queue_time_action},
    window_control::queue_window_action,
};

/// Registers the queue, the status, and the executor system.
pub struct DriverExecutorPlugin;

impl Plugin for DriverExecutorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<DriverQueue>()
            .init_resource::<DriverStatus>()
            .add_systems(
                PreUpdate,
                (refresh_window_status, execute_driver_steps)
                    .chain()
                    .before(InputSystems),
            );
    }
}

/// Copies the primary window's physical size and winit's minimized answer into
/// [`DriverStatus`].
// `NonSendMarker` keeps this on the main thread, the only thread whose `WINIT_WINDOWS` is filled.
pub fn refresh_window_status(
    primary_window: Query<(Entity, &Window), With<PrimaryWindow>>,
    mut status: ResMut<DriverStatus>,
    _main_thread: NonSendMarker,
) {
    let primary_window = primary_window.single().ok();
    let physical_size =
        primary_window.map(|(_, window)| [window.physical_width(), window.physical_height()]);
    let os_minimized = primary_window.and_then(|(entity, _)| winit_reports_minimized(entity));
    status.refresh_window_state(physical_size, os_minimized);
}

fn winit_reports_minimized(window_entity: Entity) -> Option<bool> {
    WINIT_WINDOWS.with_borrow(|winit_windows| {
        winit_windows
            .get_window(window_entity)
            .and_then(|winit_window| winit_window.is_minimized())
    })
}

/// Runs steps until a barrier or wait, so injected input is read by `InputSystems` this frame.
pub fn execute_driver_steps(
    mut queue: ResMut<DriverQueue>,
    mut status: ResMut<DriverStatus>,
    settings: Res<DriverSettings>,
    readiness: Option<Res<RenderReadiness>>,
    mut injector: InputInjector,
    mut commands: Commands,
    mut app_exit: MessageWriter<AppExit>,
) {
    status.frame_count += 1;
    // Wall-clock time, because `time_fixed_step` makes `Time<Real>` advance by a fixed amount.
    let now = Instant::now();
    let readiness = readiness.as_deref();
    // Read before the other counters, which are then at least as new as this frame.
    let render_frame = readiness.map_or(0, RenderReadiness::render_frames_published);
    status.refresh_render_readiness(readiness);
    let quiet_render_frames = readiness.map_or(0, RenderReadiness::quiet_render_frames);
    let waiting_until_drawn = match queue.poll_drawn_wait(now, render_frame, quiet_render_frames) {
        DrawnWaitProgress::StillWaiting => true,
        DrawnWaitProgress::Drawn => {
            report_failed_pipelines(readiness, &mut status);
            false
        }
        DrawnWaitProgress::TimedOut => {
            let message = drawn_timeout_message(readiness);
            status.record_error(message);
            false
        }
        DrawnWaitProgress::NotWaiting => false,
    };
    if !waiting_until_drawn && !queue.is_still_waiting(now) {
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
                DriverStep::WaitUntilDrawn {
                    timeout_seconds,
                    settle_frames,
                } => match readiness {
                    None => status.record_error(
                        "wait_until_drawn: the app has no renderer, so nothing is drawn",
                    ),
                    Some(_) => {
                        queue.begin_drawn_wait(now, timeout_seconds, settle_frames, render_frame);
                        break;
                    }
                },
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

fn drawn_timeout_message(readiness: Option<&RenderReadiness>) -> String {
    let pending = readiness.map_or(0, RenderReadiness::pending_pipelines);
    let total = readiness.map_or(0, RenderReadiness::total_pipelines);
    let reason = if total == 0 {
        "no render pipelines exist yet (is a camera drawing anything?)".to_owned()
    } else if pending > 0 {
        format!("{pending} of {total} pipelines still compiling")
    } else {
        format!(
            "the pipeline count ({total}) kept changing; a game that keeps making new              pipelines can pass a lower settle_frames"
        )
    };
    format!("wait_until_drawn: timed out: {reason}")
}

fn report_failed_pipelines(readiness: Option<&RenderReadiness>, status: &mut DriverStatus) {
    let failed = readiness.map_or(0, RenderReadiness::failed_pipelines);
    if failed > 0 {
        let noun = if failed == 1 { "pipeline" } else { "pipelines" };
        status.record_error(format!(
            "wait_until_drawn: {failed} {noun} failed to compile; see failed_pipelines in drive/status"
        ));
    }
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
        DriverStep::Window(action) => queue_window_action(commands, *action),
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
        | DriverStep::WaitUntilDrawn { .. }
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
