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

One JSON object per command, tagged by `cmd`. Positions are logical pixels in the primary window, origin top left. Keys use Bevy `KeyCode` names, buttons use `MouseButton` names (`Left` by default).

| Command | Example |
| --- | --- |
| `key_down` | `{"cmd":"key_down","key":"KeyA","text":"a"}` |
| `key_up` | `{"cmd":"key_up","key":"KeyA"}` |
| `key_tap` | `{"cmd":"key_tap","key":"Space","hold_frames":2}` (default 1) |
| `type_text` | `{"cmd":"type_text","text":"hello"}` |
| `mouse_move` | `{"cmd":"mouse_move","x":100,"y":80}` |
| `mouse_down` / `mouse_up` | `{"cmd":"mouse_down","button":"Right"}` |
| `click` | `{"cmd":"click","x":140,"y":80}` (moves first when `x` and `y` are given) |
| `drag` | `{"cmd":"drag","from":[450,300],"to":[200,300],"frames":10}` |
| `scroll` | `{"cmd":"scroll","y":-3,"unit":"line"}` (`line` or `pixel`) |
| `touch` | `{"cmd":"touch","phase":"started","x":50,"y":60,"id":0}` (`started`, `moved`, `ended`, `canceled`) |
| `touch_drag` | `{"cmd":"touch_drag","from":[0,0],"to":[100,100],"id":0,"frames":10}` |
| `pinch` / `rotate` | `{"cmd":"pinch","delta":0.5}` |
| `pan` | `{"cmd":"pan","x":10,"y":0}` |
| `double_tap` | `{"cmd":"double_tap"}` |
| `wait_frames` | `{"cmd":"wait_frames","frames":30}` |
| `wait_seconds` | `{"cmd":"wait_seconds","seconds":0.5}` (wall-clock time from `std::time::Instant`, so pausing or `time_fixed_step` does not change it) |
| `wait_until_drawn` | `{"cmd":"wait_until_drawn","timeout_seconds":60,"settle_frames":3}` (waits until the shaders in use have compiled; see below; timeout default 60, above 0 and at most 3600; `settle_frames` default 3, from 1 to 600) |
| `screenshot` | `{"cmd":"screenshot","path":"shots/start.png"}` |
| `window_minimize` / `window_restore` | `{"cmd":"window_minimize"}` (see [Minimized windows](#minimized-windows)) |
| `release_all` | `{"cmd":"release_all"}` |
| `quit` | `{"cmd":"quit"}` |
| `set_seed` | `{"cmd":"set_seed","seed":42}` (sets `DriverSeed`) |
| `time_pause` / `time_resume` | `{"cmd":"time_pause"}` (pauses or resumes `Time<Virtual>`) |
| `time_speed` | `{"cmd":"time_speed","factor":0.5}` (factor must be finite and above 0) |
| `time_fixed_step` | `{"cmd":"time_fixed_step","ms":16.667}` (every frame advances exactly `ms` milliseconds, above 0) |
| `time_real` | `{"cmd":"time_real"}` (automatic real-time updates and speed 1.0; does not unpause) |
| `step_frames` | `{"cmd":"step_frames","frames":5}` (resume, run exactly 5 frames, pause; at least 1) |
| `save_state` / `load_state` | `{"cmd":"save_state","path":"saves/slot1.txt"}` (runs your registered hook) |

Frame counts (`hold_frames`, `frames`, and `wait_frames`) are capped at 100000 per command; larger values are rejected. `click`, `drag`, `touch`, `touch_drag`, and `scroll` end with an extra frame so `bevy_picking`, which reads window events a frame late, has finished before the next command (such as a screenshot) runs. `clear` also cancels touches the driver started and pauses time again if a `step_frames` was running.

Every command is visible to game systems for at least one frame of its own. `key_tap` is down, wait `hold_frames`, up, wait one frame, because a press and release in one frame are invisible to `pressed()`. `drag` moves to `from`, presses, makes `frames` evenly spaced moves ending exactly at `to`, and releases.

## Live use over BRP

BRP methods: `drive/send` (one command object or an array; all are parsed before any is queued), `drive/status`, and `drive/clear` (empties the queue and releases everything the driver holds). `drive/status` reports `frame_count`, `queue_length`, `idle`, `cursor_position`, `held_keys`, `held_buttons`, `held_touches`, `pending_screenshots`, `screenshots_saved`, `errors` (last 50), and `script_loaded`, `seed`, `paused`, `speed`, `fixed_step_ms` (null under real time), `virtual_elapsed_seconds`, `state_operations` (last 50 entries such as `saved saves/slot1.txt`), `render_ready`, `pipelines_pending`, `render_frame_count` (runs of Bevy's `Render` schedule, or null without a renderer), `window_size` (the primary window's `[width, height]` in physical pixels, or null without a primary window), and `window_minimized`. The time, render, and window fields are refreshed once per frame, and `speed` reads 0 until the first frame finishes. `render_frame_count` usually runs 1 or 2 behind `frame_count`, because a frame is drawn while the next one runs. It counts render runs, not frames shown on screen, so it keeps rising in a minimized window.

The helper `tools/drive.py` needs only Python 3:

```text
python tools/drive.py send '{"cmd":"click","x":140,"y":80}' --wait
python tools/drive.py send '[{"cmd":"key_tap","key":"KeyA"},{"cmd":"screenshot","path":"shots/a.png"}]' --wait
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

## Waiting for the first drawn frame

Bevy compiles shaders on background threads and skips drawing anything whose render pipeline is not ready, so the first frames of a run can be blank. Start a script with `wait_until_drawn` instead of a fixed `wait_frames`:

```text
{"cmd":"wait_until_drawn"}
{"cmd":"screenshot","path":"shots/start.png"}
```

The driver watches Bevy's `PipelineCache` in the render world. The game counts as drawn once 3 render frames in a row (or `settle_frames`, if the command gives it) had at least one pipeline, none queued or compiling, and no new ones. Only render frames after the command starts count (at least one more than that number, because the render world runs a frame behind), so a scene loaded or spawned just before the command is waited for too. The command then waits one more frame so that frame has been presented. `drive/status` shows the same check, always with 3 frames, as `render_ready`, with `pipelines_pending` counting the pipelines still compiling.

- On a timeout, an error such as `wait_until_drawn: timed out: 2 of 14 pipelines still compiling` is recorded and the script continues. Other reasons are that no pipelines exist (nothing is being drawn) or that the pipeline count kept changing.
- A game that keeps making new pipelines (materials or variants that change over time) may never have 3 quiet frames in a row, so the wait runs until the timeout. Pass a lower `settle_frames`, such as `{"cmd":"wait_until_drawn","settle_frames":1}`, to end on the first quiet frame. Fewer frames give slow assets less time to queue their pipelines, so keep the default when the game allows it.
- A minimized window does not stop the wait: on Windows it succeeds at once, though nothing is on screen. See [Minimized windows](#minimized-windows).
- A shader that fails to compile counts as finished, so the wait does not hang; an error names how many failed, and Bevy's log has the details.
- With no renderer (no `RenderPlugin`), the command records an error and the script continues.
- It checks the pipelines queued so far. Pipelines only queue once their assets have loaded, so a slow asset that arrives after the 3 steady frames is not waited for. Something spawned later in the game (a new material) is not covered either: run `wait_until_drawn` again after spawning it.

## Minimized windows

On Windows, Bevy reports a minimized window's size as 0 by 0, and `window_minimized` is true exactly when `window_size` is `[0, 0]`. Other platforms may keep the size while minimized, so there the field may stay false. While minimized, `frame_count` and `render_frame_count` keep rising as usual, and `wait_until_drawn` succeeds even though nothing is on screen (tested on Windows 11 with Bevy 0.20.1). Do not minimize a game under test:

- In the demo scene, minimizing alone left the window as a 16 by 16 frame at the top-left of the screen with nothing drawn, and restoring did not bring it back. `window_size` stays `[0, 0]` in that state, so `window_minimized` stays true and a script can spot it. In another game, minimizing and restoring worked, but a `screenshot` taken while minimized broke the window the same way, and every later screenshot was a 1 by 1 image with no error (the driver now refuses that screenshot, as the next point says). The cause of the demo scene's breakage is not known yet.
- While `window_minimized` is true, the driver refuses a `screenshot`: it captures nothing and records the error `screenshot <path>: window is minimized (0x0), skipped`, and the script goes on. Where the field stays false while minimized (on some platforms other than Windows), the screenshot is still taken.
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

## Gotchas

- `wait_seconds` uses wall-clock time, so it keeps counting while virtual time is paused. `wait_frames` counts rendered frames and also keeps running.
- `Time<Virtual>` clamps each frame's advance to its max delta (250 ms by default). A `time_fixed_step` above that raises the max delta to the step, and `time_real` restores 250 ms.

- Keep your real mouse and keyboard off the game window during tests. Real input mixes with injected input.
- Only one `RemotePlugin` may exist. The driver adds it when `enable_remote` is true.
- Screenshot paths are relative to the process working directory. Parent folders are created. Use a `.png` extension, since the format follows the extension.
- The `drive/*` methods run in `Update`, so a paused or unfocused game that stops updating also stops the driver.
- Commands with no primary window are skipped and an error is recorded.

## License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT license](LICENSE-MIT), at your option, the same as Bevy.

Unless you explicitly state otherwise, any contribution you intentionally submit for inclusion in this work, as defined in the Apache-2.0 license, shall be dual licensed as above, without any additional terms or conditions.
