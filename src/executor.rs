//! The system that runs queued steps, one frame at a time.

use std::time::Instant;

use bevy::{
    ecs::system::{NonSendMarker, SystemParam},
    input::{ButtonState, InputSystems, touch::TouchPhase},
    prelude::*,
    window::PrimaryWindow,
};
use bevy_winit::WINIT_WINDOWS;

use crate::{
    checks::{queue_check, queue_quit},
    clipboard_injection::queue_clipboard_action,
    gamepad_injection::GamepadInjector,
    injection::{InjectionError, InputInjector},
    plugin::DriverSettings,
    real_input::RealInputBlocking,
    render_readiness::RenderReadiness,
    screenshot::request_screenshot,
    seed::queue_set_seed,
    state_handlers::queue_state_operation,
    status::DriverStatus,
    steps::{DrawnWaitProgress, DriverQueue, DriverStep},
    targets::queue_target_resolution,
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

/// Copies the primary window's physical size and winit's minimized and focus answers into
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
    let primary_entity = primary_window.map(|(entity, _)| entity);
    let os_minimized = primary_entity.and_then(winit_reports_minimized);
    let os_focused = primary_entity.and_then(winit_reports_focus);
    status.refresh_window_state(physical_size, os_minimized, os_focused);
}

fn winit_reports_minimized(window_entity: Entity) -> Option<bool> {
    WINIT_WINDOWS.with_borrow(|winit_windows| {
        winit_windows
            .get_window(window_entity)
            .and_then(|winit_window| winit_window.is_minimized())
    })
}

/// Whether the operating system has given the window keyboard focus, or none when it has no
/// winit window. Call it only from a system holding `NonSendMarker`.
pub(crate) fn winit_reports_focus(window_entity: Entity) -> Option<bool> {
    WINIT_WINDOWS.with_borrow(|winit_windows| {
        winit_windows
            .get_window(window_entity)
            .map(|winit_window| winit_window.has_focus())
    })
}

/// Every injector the executor writes through, bundled to keep the system's parameter count low.
#[derive(SystemParam)]
pub struct DriverInjectors<'w, 's> {
    input: InputInjector<'w, 's>,
    gamepad: GamepadInjector<'w, 's>,
}

/// Runs steps until a barrier or wait, so injected input is read by `InputSystems` this frame.
pub fn execute_driver_steps(
    mut queue: ResMut<DriverQueue>,
    mut status: ResMut<DriverStatus>,
    settings: Res<DriverSettings>,
    readiness: Option<Res<RenderReadiness>>,
    mut blocking: ResMut<RealInputBlocking>,
    mut injectors: DriverInjectors,
    mut commands: Commands,
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
                DriverStep::ResolveTargets(command) => {
                    queue.begin_target_resolution();
                    queue_target_resolution(&mut commands, command, queue.generation());
                    break;
                }
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
                DriverStep::Gamepad { slot, action } => {
                    if let Err(problem) = injectors.gamepad.apply(slot, action, &mut commands) {
                        status.record_error(format!("{}: {problem}", action.command_name()));
                    }
                }
                DriverStep::ReleaseAll => {
                    if let Err(problem) = release_everything_held(&mut injectors, &mut status) {
                        status.record_error(problem.to_string());
                    }
                }
                input_step => {
                    if let Err(problem) = apply_step(
                        &input_step,
                        &mut injectors.input,
                        &mut status,
                        &mut commands,
                        &mut blocking,
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
    blocking: &mut RealInputBlocking,
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
        DriverStep::Focus { focused } => injector.focus(*focused)?,
        DriverStep::FileDrag(action) => injector.file_drag_and_drop(action.clone())?,
        DriverStep::Ime(action) => injector.ime(action.clone())?,
        DriverStep::Clipboard(action) => queue_clipboard_action(commands, action.clone()),
        DriverStep::RealInput { blocked } => blocking.set_blocked(*blocked),
        DriverStep::Quit => queue_quit(commands),
        DriverStep::Check {
            check,
            arguments,
            equals,
        } => queue_check(commands, check.clone(), arguments.clone(), equals.clone()),
        DriverStep::SetSeed(seed) => queue_set_seed(commands, *seed),
        DriverStep::Time(action) => queue_time_action(commands, *action),
        DriverStep::State { operation, path } => {
            queue_state_operation(commands, *operation, path.clone());
        }
        DriverStep::Gamepad { .. }
        | DriverStep::ReleaseAll
        | DriverStep::EndFrame
        | DriverStep::ResolveTargets(_)
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
    injectors: &mut DriverInjectors,
    status: &mut DriverStatus,
) -> Result<(), InjectionError> {
    // First, because it cannot fail and so is never skipped by an error below.
    injectors.gamepad.release_all();
    let injector = &mut injectors.input;
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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use bevy::{
        ecs::message::Messages,
        input::InputPlugin,
        window::{FileDragAndDrop, Ime, WindowEvent, WindowFocused},
    };

    use super::*;
    use crate::{
        commands::parse_command_text, gamepad_injection::DriverGamepadPlugin,
        injection::DriverWindowEventPlugin, real_input::DriverRealInputPlugin,
    };

    fn window_app(window: Window) -> (App, Entity) {
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
        let window_entity = app.world_mut().spawn((window, PrimaryWindow)).id();
        (app, window_entity)
    }

    // Every command ends its frame, so one update runs exactly one command.
    fn run_command(app: &mut App, text: &str) {
        let command = parse_command_text(text).expect("command should parse");
        app.world_mut()
            .resource_mut::<DriverQueue>()
            .enqueue_commands(&[command]);
        app.update();
    }

    fn written<MessageType: Message + Clone>(app: &App) -> Vec<MessageType> {
        let messages = app.world().resource::<Messages<MessageType>>();
        messages.get_cursor().read(messages).cloned().collect()
    }

    #[test]
    fn focus_writes_the_message_but_leaves_the_window_component_alone() {
        let (mut app, window) = window_app(Window {
            focused: false,
            ..default()
        });
        run_command(&mut app, r#"{"command":"focus","focused":true}"#);

        let expected = WindowFocused {
            window,
            focused: true,
        };
        assert_eq!(written::<WindowFocused>(&app), vec![expected.clone()]);
        assert_eq!(
            written::<WindowEvent>(&app),
            vec![WindowEvent::WindowFocused(expected)]
        );
        let component = app.world().get::<Window>(window).expect("window exists");
        assert!(!component.focused);
    }

    #[test]
    fn resize_changes_the_logical_size() {
        let (mut app, window) = window_app(Window::default());
        run_command(&mut app, r#"{"command":"resize","width":800,"height":600}"#);

        let resolution = &app
            .world()
            .get::<Window>(window)
            .expect("window exists")
            .resolution;
        assert_eq!((resolution.width(), resolution.height()), (800.0, 600.0));
    }

    #[test]
    fn file_drop_writes_the_path_for_the_primary_window() {
        let (mut app, window) = window_app(Window::default());
        run_command(
            &mut app,
            r#"{"command":"file_drop","path":"assets/level.txt"}"#,
        );

        let expected = FileDragAndDrop::DroppedFile {
            window,
            path_buf: PathBuf::from("assets/level.txt"),
        };
        assert_eq!(written::<FileDragAndDrop>(&app), vec![expected.clone()]);
        assert_eq!(
            written::<WindowEvent>(&app),
            vec![WindowEvent::FileDragAndDrop(expected)]
        );
    }

    #[test]
    fn ime_commit_and_preedit_write_ime_messages() {
        let (mut app, window) = window_app(Window::default());
        run_command(
            &mut app,
            r#"{"command":"ime_preedit","text":"に","cursor":[0,3]}"#,
        );
        run_command(&mut app, r#"{"command":"ime_commit","text":"日本"}"#);

        assert_eq!(
            written::<Ime>(&app),
            vec![
                Ime::Preedit {
                    window,
                    value: "に".to_owned(),
                    cursor: Some((0, 3))
                },
                Ime::Commit {
                    window,
                    value: "日本".to_owned()
                }
            ]
        );
    }

    #[test]
    fn clipboard_set_without_a_clipboard_records_the_error() {
        let (mut app, _window) = window_app(Window::default());
        run_command(&mut app, r#"{"command":"clipboard_set","text":"hello"}"#);

        assert_eq!(
            app.world().resource::<DriverStatus>().errors,
            vec![
                "clipboard_set: the game has no ClipboardPlugin, so the clipboard cannot be set"
                    .to_owned()
            ]
        );
    }

    #[test]
    fn window_events_without_a_primary_window_record_an_error() {
        let (mut app, window) = window_app(Window::default());
        app.world_mut().despawn(window);
        run_command(&mut app, r#"{"command":"focus","focused":true}"#);

        assert_eq!(app.world().resource::<DriverStatus>().errors.len(), 1);
    }
}
