# bevy_input_driver

Lets an automated tester drive a running Bevy 0.20 game. It injects keyboard, mouse (including drags), scroll, touch, and gesture input, and takes screenshots. Commands arrive live over the Bevy Remote Protocol (BRP, HTTP JSON-RPC on 127.0.0.1) or from a JSON Lines file named at launch. The game cannot tell injected input from real input: each input is written as both its `bevy_input` message and the matching `WindowEvent`, the same way `bevy_winit` does, so `bevy_picking` and `bevy_ui` react normally. By default the real OS cursor never moves.

## Setup

In the game's `Cargo.toml`, make the driver an optional git dependency behind a feature:

```toml
[dependencies]
bevy_input_driver = { git = "https://github.com/halfdeadgames/bevy_input_driver", optional = true }

[features]
input_driver = ["dep:bevy_input_driver"]
```

In `main.rs`, add the plugin after `DefaultPlugins`:

```rust
#[cfg(feature = "input_driver")]
app.add_plugins(bevy_input_driver::InputDriverPlugin::default());
```

`InputDriverPlugin` has `port` (default 15702), `script_path` (default none), `enable_remote` (default true), `seed` (default none), and `update_window_cursor_position` (default false). By default the driver keeps its cursor only in `drive/status` and does not write `Window::cursor_position`, so `window.cursor_position()` stays empty; `bevy_picking` and `bevy_ui` are unaffected. Set the field to true if your game reads `window.cursor_position()`. Warning: `bevy_winit` compares each changed window with a private cache, so with the field on, any later change to the `Window` (a title or cursor icon, for example) warps your real OS cursor to the injected position. Turn it on only for unattended runs. Do not add `RemotePlugin` yourself while `enable_remote` is true: the driver adds it, with the built-in `world.*` methods still available.

Try it with the demo scene: `cargo run --example driver_demo`.

## Commands

One JSON object per command, tagged by `command`. Positions are logical pixels in the primary window, origin top left. Keys use Bevy `KeyCode` names, buttons use `MouseButton` names (`Left` by default).

| Command | Example |
| --- | --- |
| `key_down` | `{"command":"key_down","key":"KeyA","text":"a"}` |
| `key_up` | `{"command":"key_up","key":"KeyA"}` |
| `key_tap` | `{"command":"key_tap","key":"Space","hold_frames":2}` (default 1) |
| `type_text` | `{"command":"type_text","text":"hello"}` |
| `mouse_move` | `{"command":"mouse_move","x":100,"y":80}` |
| `mouse_down` / `mouse_up` | `{"command":"mouse_down","button":"Right"}` |
| `click` | `{"command":"click","x":140,"y":80}` (moves first when `x` and `y` are given) |
| `drag` | `{"command":"drag","from":[450,300],"to":[200,300],"frames":10}` |
| `scroll` | `{"command":"scroll","y":-3,"unit":"line"}` (`line` or `pixel`) |
| `touch` | `{"command":"touch","phase":"started","x":50,"y":60,"id":0}` (`started`, `moved`, `ended`, `canceled`) |
| `touch_drag` | `{"command":"touch_drag","from":[0,0],"to":[100,100],"id":0,"frames":10}` |
| `pinch` / `rotate` | `{"command":"pinch","delta":0.5}` |
| `pan` | `{"command":"pan","x":10,"y":0}` |
| `double_tap` | `{"command":"double_tap"}` |
| `wait_frames` | `{"command":"wait_frames","frames":30}` |
| `wait_seconds` | `{"command":"wait_seconds","seconds":0.5}` (wall-clock time from `std::time::Instant`, so pausing or `time_fixed_step` does not change it) |
| `wait_until_drawn` | `{"command":"wait_until_drawn","timeout_seconds":60,"settle_frames":3}` (waits until the shaders in use have compiled; see below; timeout default 60, above 0 and at most 3600; `settle_frames` default 3, from 1 to 600) |
| `screenshot` | `{"command":"screenshot","path":"shots/start.png"}` |
| `window_minimize` / `window_restore` | `{"command":"window_minimize"}` (see [Minimized windows](#minimized-windows)) |
| `real_input` | `{"command":"real_input","blocked":false}` (`blocked` is required; see [Blocking real input](#blocking-real-input)) |
| `release_all` | `{"command":"release_all"}` |
| `quit` | `{"command":"quit"}` (exit code 1 if any `expect` check failed, otherwise 0) |
| `expect` | `{"command":"expect","check":"score","arguments":{"player":1},"equals":5}` (runs your registered check; `arguments` and `equals` are optional; see [Checks](#checks)) |
| `set_seed` | `{"command":"set_seed","seed":42}` (sets `DriverSeed`) |
| `time_pause` / `time_resume` | `{"command":"time_pause"}` (pauses or resumes `Time<Virtual>`) |
| `time_speed` | `{"command":"time_speed","factor":0.5}` (factor must be finite and above 0) |
| `time_fixed_step` | `{"command":"time_fixed_step","milliseconds":16.667}` (every frame advances exactly that many milliseconds; the value must be above 0) |
| `time_real` | `{"command":"time_real"}` (automatic real-time updates and speed 1.0; does not unpause) |
| `step_frames` | `{"command":"step_frames","frames":5}` (resume, run exactly 5 frames, pause; at least 1) |
| `save_state` / `load_state` | `{"command":"save_state","path":"saves/slot1.txt"}` (runs your registered hook) |

Frame counts (`hold_frames`, `frames`, and `wait_frames`) are capped at 100000 per command; larger values are rejected. `click`, `drag`, `touch`, `touch_drag`, and `scroll` end with an extra frame so `bevy_picking`, which reads window events a frame late, has finished before the next command (such as a screenshot) runs. `drive/clear` also cancels touches the driver started, and pauses time again if a `step_frames` was running.

Every command is visible to game systems for at least one frame of its own. `key_tap` is down, wait `hold_frames`, up, wait one frame, because a press and release in one frame are invisible to `pressed()`. `drag` moves to `from`, presses, makes `frames` evenly spaced moves ending exactly at `to`, and releases.

## Live use over BRP

BRP methods: `drive/send` (one command object or an array; all are parsed before any is queued), `drive/status`, and `drive/clear` (empties the queue and releases everything the driver holds). `drive/status` reports `frame_count`, `queue_length`, `idle`, `cursor_position`, `held_keys`, `held_buttons`, `held_touches`, `pending_screenshots`, `screenshots_saved`, `errors` (last 50), and `script_loaded`, `seed`, `paused`, `speed`, `fixed_step_milliseconds` (null under real time), `virtual_elapsed_seconds`, `state_operations` (last 50 entries such as `saved saves/slot1.txt`), `checks` (last 50 check results), `checks_failed` (every failed check since launch), `render_ready`, `pipelines_pending`, `failed_pipelines` (each entry has `label`, `shader_paths`, and `error`), `render_frame_count` (runs of Bevy's `Render` schedule, or null without a renderer), `window_size` (the primary window's `[width, height]` in physical pixels, or null without a primary window), `window_minimized`, `window_os_minimized` (see [Minimized windows](#minimized-windows)), and `real_input_blocked` (see [Blocking real input](#blocking-real-input)). The time, render, and window fields are refreshed once per frame, and `speed` reads 0 until the first frame finishes. `render_frame_count` usually runs 1 or 2 behind `frame_count`, because a frame is drawn while the next one runs. It counts render runs, not frames shown on screen, so it keeps rising in a minimized window.

The helper `tools/drive.py` needs only Python 3:

```text
python tools/drive.py send '{"command":"click","x":140,"y":80}' --wait
python tools/drive.py send '[{"command":"key_tap","key":"KeyA"},{"command":"screenshot","path":"shots/a.png"}]' --wait
python tools/drive.py send-file tests/smoke.jsonl --wait --timeout 60
python tools/drive.py status
python tools/drive.py wait --timeout 30
python tools/drive.py clear
```

Use `--port N` before the action for a non-default port. Exit codes: 0 on success, 1 on a timeout or bad input, 2 with a plain message when the game is not reachable at the start, and 3 when the game answers with a BRP error. If the game stops answering while `--wait` or `wait` is polling after it had answered before (for example after a `quit` command), the helper prints `{"game_exited": true}` and exits 0.

## Launch file

A JSON Lines file with one command per line. Blank lines and lines starting with `//` or `#` are skipped. The whole file is parsed first: on any error nothing is queued, and the file name, line number, and problem are logged and recorded in `drive/status` under `errors`.

```text
cargo run -- --input-script tests/smoke.jsonl
```

`--input-script` overrides `InputDriverPlugin::script_path`.

A script with `expect` lines and a final `quit` is a self-checking test: the process exits with code 1 if any check failed, so a test runner only needs the exit code. Try `cargo run --example driver_demo -- --input-script scripts/demo_checks.jsonl` (exits 0) and `scripts/demo_checks_fail.jsonl` (exits 1).

## Blocking real input

A real mouse resting over a newly shown window sends cursor positions that mix with injected ones, so the driver blocks real input while it is in control.

- Blocking starts when a launch script loads, or when the first live command arrives through `drive/send`. It lasts until the game exits or `{"command":"real_input","blocked":false}` turns it off. `{"command":"real_input","blocked":true}` turns it back on. After you send `real_input`, later live commands do not change the setting. `drive/status` reports the state as `real_input_blocked`, and one info line is logged each time it changes.
- Mouse: while blocked, the primary window's `CursorOptions::hit_test` is false, so on Windows the real pointer passes through the window to whatever is below. It is written only when the state changes. The driver never writes `Window::cursor_position`.
- Keyboard and pointer messages: while blocked, the driver empties the typed `KeyboardInput`, `MouseButtonInput`, `MouseWheel`, `CursorMoved`, `MouseMotion`, `TouchInput`, and gesture message queues at the start of every frame (in `First`, after Bevy swaps the message buffers). Real input is written before that point and injected input in `PreUpdate`, so only real input is dropped. This holds even when the window has focus. Real `KeyboardFocusLost` is kept.
- Known limits: `Messages<WindowEvent>` is not cleared, because `bevy_picking` reads the injected pointer window events a frame late. A real key press can therefore still show up inside `WindowEvent::KeyboardInput` for a system that reads raw `WindowEvent`s. A game system that reads these typed messages in `FixedUpdate` can miss an injected message in a frame where no fixed step ran, because the next frame's clearing removes it before the step. Read state from `ButtonInput` or read in `Update` instead.

## Off-screen start

When a script path is given (`--input-script` or `InputDriverPlugin::script_path`) and the process arguments do not contain `--on-screen`, the driver places the primary window at (-30000, -30000), far off every monitor, and sets `Window::focused` to false so it does not steal focus. This happens before the operating system window exists, so it never flashes on screen. The window is not minimized, because a minimized window has no surface and screenshots are skipped. Set `InputDriverPlugin::off_screen_for_scripts` to false (default true) to turn this off in code. The setting is also stored in `DriverSettings`.

An unfocused window runs under Bevy's default `WinitSettings::game()` unfocused mode: reactive low power, at most 60 frames per second. For performance tests, run on-screen and focused: pass `--on-screen` (the window then takes focus as usual), or run without a script.

## Waiting for the first drawn frame

Bevy compiles shaders on background threads and skips drawing anything whose render pipeline is not ready, so the first frames of a run can be blank. Start a script with `wait_until_drawn` instead of a fixed `wait_frames`:

```text
{"command":"wait_until_drawn"}
{"command":"screenshot","path":"shots/start.png"}
```

The driver watches Bevy's `PipelineCache` in the render world. The game counts as drawn once 3 render frames in a row (or `settle_frames`, if the command gives it) had at least one pipeline, none queued or compiling, and no new ones. Only render frames after the command starts count (at least one more than that number, because the render world runs a frame behind), so a scene loaded or spawned just before the command is waited for too. The command then waits one more frame so that frame has been presented. `drive/status` shows the same check, always with 3 frames, as `render_ready`, with `pipelines_pending` counting the pipelines still compiling.

- On a timeout, an error such as `wait_until_drawn: timed out: 2 of 14 pipelines still compiling` is recorded and the script continues. Other reasons are that no pipelines exist (nothing is being drawn) or that the pipeline count kept changing.
- A game that keeps making new pipelines (materials or variants that change over time) may never have 3 quiet frames in a row, so the wait runs until the timeout. Pass a lower `settle_frames`, such as `{"command":"wait_until_drawn","settle_frames":1}`, to end on the first quiet frame. Fewer frames give slow assets less time to queue their pipelines, so keep the default when the game allows it.
- A minimized window does not stop the wait: on Windows it succeeds at once, though nothing is on screen. See [Minimized windows](#minimized-windows).
- A shader that fails to compile counts as finished, so the wait does not hang. The error says how many pipelines failed, and `failed_pipelines` in `drive/status` lists each failed pipeline's label, shader paths, and error text.
- With no renderer (no `RenderPlugin`), the command records an error and the script continues.
- It checks the pipelines queued so far. Pipelines only queue once their assets have loaded, so a slow asset that arrives after the 3 steady frames is not waited for. Something spawned later in the game (a new material) is not covered either: run `wait_until_drawn` again after spawning it.

## Minimized windows

To check a game window from outside the game with Windows calls such as `GetWindowRect` or `ShowWindow`, find it by class and title (`FindWindow("Window Class", "<title>")`), not by a process's main window handle. winit also creates a hidden helper window, 16 by 16 at the top-left corner of the screen with a 0 by 0 drawing area, and a main-window lookup can return it instead of the game window.

`window_os_minimized` is what winit, the library Bevy uses for windows, says about the primary window: `true`, `false`, or `null` when it cannot tell. It is always `null` on Wayland, iOS, Android, and the web, and in a game without winit. `window_minimized` is true when `window_os_minimized` is `true` or `window_size` is `[0, 0]`, which is how Windows reports a minimized window. The size check backs up winit's answer when winit cannot tell. Where winit answers `null` and the size is kept while minimized, `window_minimized` stays false. While minimized, `frame_count` and `render_frame_count` keep rising as usual, and `wait_until_drawn` succeeds even though nothing is on screen (tested on Windows 11 with Bevy 0.20.1). Do not minimize a game under test:

- In one game, a `screenshot` taken while minimized made every later screenshot a 1 by 1 image, with no error. The driver now refuses that screenshot, as the next point says.
- While `window_minimized` is true, the driver refuses a `screenshot`: it captures nothing and records the error `screenshot <path>: window is minimized, skipped`, and the script goes on. Where the field stays false while minimized (where winit answers `null` and the size is kept), the screenshot is still taken.
- Check `window_minimized` before a `wait_until_drawn` in a script that might run while the window is minimized.

To test this on purpose, `window_minimize` and `window_restore` ask Bevy to minimize or restore the primary window (`Window::set_minimized`), then wait one frame. They only send the request: the operating system applies it a few frames later, or not at all. Follow each one with a `wait_frames` and read `window_minimized` in `drive/status` to see what happened. Put a `wait_frames` between a `window_minimize` and a `screenshot` too: in 6 tries on Windows the one frame was enough for the screenshot guard to see the minimized window, but a slower machine may need longer. Without a primary window, or with more than one, each records an error such as `window_minimize: no primary window` and the script goes on.

## Seeds and repeatable tests

For a test to repeat exactly, the game must take all of its randomness from one random generator, built from `DriverSeed` at startup. The driver does not depend on `rand`: it only stores the number.

```rust
fn build_rng(seed: Res<bevy_input_driver::DriverSeed>, mut commands: Commands) {
    let value = seed.0.unwrap_or_else(pick_my_own_seed); // None means the game chooses
    commands.insert_resource(GameRng(StdRng::seed_from_u64(value)));
}
```

Set the seed with `--seed 42` (or `--seed=42`) on the command line, with `InputDriverPlugin { seed: Some(42), ..default() }`, or with the `set_seed` command. The argument wins over the plugin field. The resource exists before your `Startup` systems run. A game may also react to `Changed<DriverSeed>` to reseed after `set_seed`. Pair the seed with `time_fixed_step` so frame timing repeats too.

## Time control

`time_pause`, `time_resume`, and `time_speed` change `Time<Virtual>`, so only code that reads virtual time (including `FixedUpdate`) is affected. `time_fixed_step` switches `TimeUpdateStrategy` to a manual duration per frame, and `time_real` switches it back. `step_frames` expands to resume, wait the given number of frames, and pause, so call it while paused.

## Save and load hooks

Register two one-shot systems that take the command's path as `In<PathBuf>` and return `Result<(), String>`:

```rust
use bevy_input_driver::InputDriverAppExt;

app.set_driver_state_handlers(save_game_state, load_game_state);

fn save_game_state(In(path): In<PathBuf>, player: Single<&Transform, With<Player>>) -> Result<(), String> {
    std::fs::write(&path, format!("{}", player.translation.x)).map_err(|problem| problem.to_string())
}
```

`save_state` and `load_state` run the matching hook, then end the frame so the game sees the loaded state on the next frame. Success adds an entry such as `loaded saves/slot1.txt` to `state_operations`. A failure, or calling either command with no hooks registered ("no save system registered: call set_driver_state_handlers"), is recorded in `errors` with the command and path in front, and the queue continues.

## Checks

A check reads a value from the game. Register it by name as a one-shot system that takes the command's `arguments` as `In<serde_json::Value>` (null when `arguments` is left out) and returns the value it read, or an error message:

```rust
use bevy_input_driver::InputDriverAppExt;
use serde_json::{Value, json};

app.add_driver_check("score", read_score);

fn read_score(In(_args): In<Value>, score: Res<Score>) -> Result<Value, String> {
    Ok(json!(score.0))
}
```

The game needs its own `serde_json = "1"` dependency for `Value` and `json!`. `BrpResult` (below) is `bevy::remote::BrpResult`.

`expect` ends the current frame, then runs the check at the start of the next one, so the check sees how the game reacted to every command before it. The check fails when:

- it returns an error,
- `equals` is given and does not match the returned value (numbers match by value, so `5` equals `5.0`, at any depth inside arrays and objects; everything else must match exactly; `"equals": null` expects null), or
- no check has that name (the message lists the registered names).

Each result is added to `checks` in `drive/status` with `check`, `passed`, `value` (the value the check returned, if any), and `message` (why it failed, at most 1000 characters, such as `expected 5, got 3`). A failure is also recorded in `errors` as `expect <name>: <message>`, adds 1 to `checks_failed`, and the queue continues, so one run reports every failure. `drive/clear` does not reset `checks_failed`.

Return an error from a check instead of panicking. A panic, such as an `unwrap()` on a missing resource, crashes the game with a non-zero exit code, and nothing is recorded in `checks`.

`quit` exits with code 1 if `checks_failed` is above 0. The exit code reaches the operating system only if `main` returns the `AppExit` from `App::run()`: write `fn main() -> AppExit { App::new() ... .run() }`.

## Game BRP methods

Because the driver owns `RemotePlugin`, add your own BRP methods through the driver:

```rust
app.add_driver_remote_method("game/score", read_score_method);

fn read_score_method(In(_params): In<Option<Value>>, score: Res<Score>) -> BrpResult {
    Ok(json!(score.0))
}
```

You can call `add_driver_remote_method` before or after adding `InputDriverPlugin`. With `enable_remote` set to false the methods are served only if the game adds its own `RemotePlugin`, before or after the call. If none exists at startup, a warning names the methods that are not served. Names starting with `drive/` are reserved for the driver: they are refused and an error is logged. Any other name that is already taken, such as `world.query`, is replaced, and a warning is logged.

## Gotchas

- `wait_seconds` uses wall-clock time, so it keeps counting while virtual time is paused. `wait_frames` counts rendered frames and also keeps running.
- `Time<Virtual>` clamps each frame's advance to its max delta (250 ms by default). A `time_fixed_step` above that raises the max delta to the step, and `time_real` restores 250 ms.
- Real input is blocked once a script loads or a live command arrives (see [Blocking real input](#blocking-real-input)), so it does not mix with injected input. Send `real_input` with `blocked` set to false to play by hand.
- Only one `RemotePlugin` may exist. The driver adds it when `enable_remote` is true. Add your own methods with `add_driver_remote_method`.
- Screenshot paths are relative to the process working directory. Parent folders are created. Use a `.png` extension, since the format follows the extension.
- The `drive/*` methods run in `Update`, so a paused or unfocused game that stops updating also stops the driver.
- Commands with no primary window are skipped and an error is recorded.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option, the same as Bevy.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
