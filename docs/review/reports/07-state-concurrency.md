## ClippiBoy review: app shell, shared state and concurrency

**Scope:** I read state.rs, game.rs, tray.rs, overlay.rs, console.rs, sounds.rs, tint.rs, model.rs, main.rs and lib.rs in full. I skimmed commands.rs, control.rs, config.rs, logging.rs and parts of pipeline.rs, mft.rs, recorder.rs, muxer.rs, audio/devices.rs and audio/engine.rs.

**Method:** I did not modify the repo and did not build it. To check runtime behaviour I downloaded the exact crate sources from crates.io: global-hotkey 0.8.0, tauri-plugin-global-shortcut 2.3.2, tao 0.35.3 and tauri-runtime-wry 2.11.4. I compiled two small standalone Rust test programs in the scratchpad to check the UTF-8 edge cases.

**Facts the findings rely on (all checked in the downloaded sources):**
- **Hotkeys run on the main thread.** `GlobalHotKeyManager::new()` runs in the plugin's `setup`, which is on the main thread. That creates a hidden window, so every Pressed handler runs on the main (UI) thread inside the window procedure. The plugin also holds its own `shortcuts` mutex while the handler runs.
- **Sync commands run on the main thread** (standard Tauri 2 behaviour).
- **Window queries from other threads block.** `is_visible`, `hwnd`, `monitor_from_point` and similar calls go through tauri-runtime-wry's `getter!`, which sends a message to the main thread and waits for the reply (`rx.recv()`). Called on the main thread they run inline.

### Map of threads and locks
- **Main thread (tao event loop):** window and tray events, hotkey Pressed handlers, all sync commands (`set_config`, `engine_status`, `list_encoders`, `file_clip`, …), `run_on_main_thread` tasks, and every window/tray call made from other threads.
- **Status thread** (`spawn_ui_updates`, lib.rs:928):
  - Runs a 50 ms loop forever, with no stop flag and no join.
  - Every 50 ms: audio levels. Every 1 s: status emit and tray refresh. Every 2 s: `track_game`, the audio refresh, device enumeration, the buffer automation, the monitor watcher and the screen watcher. Every 10 s: free disk space and the recording-room guard.
- **Threads started per action:**
  - save, screenshot and record (from hotkey, tray, and the control port's `reply_later`);
  - automation start/stop (lib.rs:660), screen restart (581), recording stops (901, 1046), recovery of leftover recordings (277);
  - one sleeper thread per banner (overlay.rs:425), the console close fallback (console.rs:144);
  - an audio refresh thread every 2 s (engine.rs:195), and global-hotkey's key-release poller.
- **Long-lived threads:** control server, ffmpeg download (tools), updater, telemetry, audio capture, pipeline pacer/mixer/MFT encoder/recorder writer, WGC thread-pool callbacks, and the tokio runtime (friends/share).
- **AppState locks:** `lifecycle` (always outermost), config, library, status, pipeline, shared, recorder, active_recording, seven game-tracking mutexes, source_notes. Inside `Shared`: packets, tracks, sources, source, encoder, recorder, and others. Elsewhere: the plugin's `shortcuts` (std mutex), tray `last_tooltip`/icons, `sounds::PLAYING`, `friends.view`, control `RUNNING`.
- **Lock orders I observed:**
  - `lifecycle` → {pipeline, shared→packets, status, recorder, current_game, buffering_game, active_recording, config}
  - status → shared → shared.encoder (state.rs:518-526)
  - status → recorder (state.rs:681, via a temporary in an `if` condition)
  - recorder → shared (state.rs:711-713)
  - shared → packets or source
  - pipeline → packets → shared.recorder (pipeline.rs:955, 973)
  - `pipeline` is held across `Pipeline::stop`'s joins (state.rs:559)
  - game_pid → game_since; config and library held across file I/O.
- **No cycle among AppState locks.** The one cycle that exists runs through the main thread (finding 1).

### Findings, most severe first

**1. [Critical: rare but a permanent hang] [bug] A lock is held while waiting on the main thread, so the app can deadlock. Confirmed pattern; the interleaving is plausible.**
- lib.rs:216-221: `overlay::show(app, BannerKind::Recording, "Recording", state.current_game.lock().clone());`
- lib.rs:356: `Some(match state.current_game.lock().as_ref() { … })`
- The MutexGuard is a temporary in a call argument (or a `match` scrutinee), so it lives until the end of the statement, which includes the whole of `overlay::show`.
- Inside `overlay::show`, `window.is_visible()` (overlay.rs:404) and `window.hwnd()` (overlay.rs:479) are blocking main-thread round trips when called from a worker thread.
- These main-thread paths lock `current_game`:
  - `engine_status` (sync, commands.rs:316) → `status_snapshot` (state.rs:736);
  - the toggle-buffer hotkey (runs on main, see finding 2) → `start_pipeline` (state.rs:472/515);
  - `set_config` → `start_buffer_and_notify`.
- **Interleaving:**
  1. At boot, `start_buffer_if_configured`'s thread takes `current_game` at lib.rs:356 and queues IsVisible.
  2. At the same moment the console page mounts and calls `api.status()` (Console.tsx:337). The main thread dispatches that IPC first and blocks on `current_game`.
  3. Neither side can continue. The event loop is dead, so all windows, the tray and the hotkeys (which arrive through the same message loop) hang for good.
- A second trigger: the record hotkey, then the toggle-buffer hotkey within a few milliseconds.
- **Fix:** read the value into a local first (`let game = state.current_game.lock().clone(); overlay::show(…, game)`) at both sites. Make it a rule that no AppState guard is alive while calling window APIs. Better still, send banners through a channel to one overlay worker thread.

**2. [High] [perf/robustness] The toggle-buffer hotkey starts and stops the capture stack on the UI thread. Confirmed.**
- lib.rs:813-816: `if event.state() == ShortcutState::Pressed { toggle_buffer_and_notify(app); }` runs directly; every other hotkey spawns a thread.
- The work done there:
  - `launch_pipeline` spawns `ffmpeg -version` every time (state.rs:486, `!muxer::available() && tools::is_ready()` evaluates the expensive check first);
  - `Pipeline::start` waits up to 10 s for encoder readiness (mft.rs:1135, `recv_timeout(10s)`);
  - `lifecycle` may be held elsewhere, e.g. by `attach_recorder`'s 5 s keyframe loop (state.rs:1014-1020) or a `close_recording` whose writer is flushing to a slow disk;
  - stop drains the encoder with unbounded joins.
- **Effect:** every webview freezes (main window, console, banner). Every worker thread that needs a window query stalls too, including the status thread's `window_awake` and banner display. This is also what makes finding 1 possible.
- The comment at lib.rs:786 ("the hotkey arrives on one of its own") is wrong.
- **Latent:** the plugin's `unregister_all` holds its `shortcuts` lock while it waits on the main thread, and the hotkey handler takes the same lock on the main thread. That deadlocks if `register_hotkeys` is ever called off the main thread. Today only sync commands and `setup` call it, so it is not reachable yet; keep it that way.
- **Fix:** `std::thread::spawn(move || toggle_buffer_and_notify(&app))`, and swap the operand order at state.rs:486.

**3. [High] [bug/UX] A persistent start failure makes the automation retry every 2 s with an error sound and banner. Confirmed by reading the logic.**
- state.rs:188-190: `if !buffer_active && !self.suppressed.load(SeqCst) { self.started.store(true, SeqCst); return AutoAction::Start; }`. `poll_always` (state.rs:246-252) has the same shape. Nothing records the failure.
- Each failure runs `notify(error)`, `sounds::play(Error)` and `overlay::show(Error, "Buffer will not start")` (lib.rs:363-366). Error banners stay at least 6 s, so the banner never goes away.
- Each attempt also builds and tears down D3D, WGC, MFT and the ffmpeg probe during gameplay.
- **Triggers:** ffmpeg quarantined by antivirus after `tools` reported ready, encoder session limits, a lost GPU device.
- **Fix:** keep a failure count with exponential backoff. After N failures, set `suppressed` and announce it once.

**4. [Medium-High] [bug] Starting a second instance rotates and destroys the running instance's log. Confirmed.**
- lib.rs:1153: `logging::init()` runs before `.plugin(tauri_plugin_single_instance…)`. Plugin setup only runs at `.build()`, and `.manage(AppState::new())` (lib.rs:1185) is also evaluated eagerly before that.
- `rotate()` (logging.rs:59-63) renames the live `clippiboy.log` over `clippiboy.log.1`, destroying the previous session's log, then creates a new file. Rust opens files with share-delete, so the rename succeeds while the first instance has the log open.
- **Effect:** every relaunch from the Start menu while the app sits in the tray, and every `clippiboy://` deep link (each Discord sign-in), leaves a 5-line stub as `clippiboy.log`. The live log moves to `.log.1` and the previous session's log is gone.
- The second instance also loads config, enumerates MFT encoders (`encode::resolve`), runs SQLite migrations with no busy_timeout against the live database, and starts audio streams before it exits.
- **Fix:** do the single-instance check first (a named mutex in `main()` before `logging::init`, or a lazy `AppState`), and rotate only once this is known to be the primary instance.

**5. [Medium] [bug] Stream Deck start/stop bypass the automation's "user decided" flags. Confirmed.**
- control.rs:225-226 call `"start" => crate::start_buffer_and_notify(app)` and `"stop" => crate::stop_buffer_and_notify(app)` directly, with no `auto.manual_start()` / `manual_stop()`.
- **Effect:** with auto-start on, a Stream Deck "stop" is undone 2 s later by `poll_always` or `poll(playing)`.
- **Fix:** route through one `set_buffer(app, on, Origin::User)`.

**6. [Medium] [bug] A failed recording close leaves `status.recording = true` and an orphaned capture pipeline running. Confirmed.**
- state.rs:1072-1078: `Err(err) => { log::error!(…); return None; }` returns before `if !self.buffer_wanted() { self.halt_pipeline(); }` and before `self.status.lock().recording = false`.
- The recorder has already been removed from both AppState and the pipeline.
- `finish` fails when `write_meta` fails, which is exactly the disk-full case the room guard is there for.
- **Effect:** the UI and tray show "recording" forever, and a recording-only pipeline (capture plus encoder) keeps running with the buffer off.
- **Fix:** a scope guard that always halts the pipeline (when the buffer is not wanted) and clears the flag.

**7. [Medium] [robustness] Shutdown is not orderly. Confirmed.**
- **Quit does not wait for a save in progress.** tray.rs:139-148 runs `abandon_recording(); stop_pipeline(); app.exit(0)`, and so does the updater's `on_before_exit`. None of them check `is_saving()`. The save thread dies while ffmpeg is running (there is no Job object). ffmpeg finishes the MP4, but it is never inserted into the library and gets no thumbnail, so the clip is missing from the gallery. A quick restart's `discard_leftovers()` can delete ffmpeg's temp inputs mid-read.
- **Quit can hang.** The Quit handler runs on the main thread and joins encoder threads with no timeout; a driver hang makes Quit hang.
- **`halt_pipeline` holds the pipeline lock during the whole stop.** state.rs:559: `if let Some(mut pipeline) = self.pipeline.lock().take() { pipeline.stop(); }`. In edition 2021 the guard lives through the whole `if let`, so `is_buffering()` (status thread, every 2 s), `save_clip` and `set_config` block for the entire encoder drain.
- **Fix:**
  - Do Quit on a worker thread: show a "finishing clip…" banner, wait for saving to clear with a timeout, then exit.
  - `let p = self.pipeline.lock().take(); if let Some(mut p) = p { p.stop(); }`
  - Optionally handle WM_QUERYENDSESSION so a Windows logoff closes recordings cleanly.

**8. [Medium] [bug] "Only buffer in game": an alt-tab of 30 s or more cancels the user's manual stop, contradicting the README. Confirmed.**
- `poll` is fed `GameTick::playing`, which depends on what is in the foreground. state.rs:195-200: after `GRACE_POLLS` checks without the game in front, `self.suppressed.store(false, SeqCst)`.
- The README says the automation "holds back until the game has ended".
- **Effect:** a user stops the buffer by hand, checks Discord for 30 s, returns, and the buffer starts again. The same 30 s alt-tab also discards an automation-started buffer and its history.
- **Fix:** clear `suppressed` only when the bound game process exits (`game_pid` becomes None); consider pausing rather than stopping while the game process is still alive.

**9. [Medium] [robustness] Duplicate start/stop actions while one is still in progress. Likely.**
- `buffer_active` only becomes true at the end of `launch_pipeline`. Any 2 s tick during a slow start (cold boot, or `Pipeline::start` taking up to 10 s) spawns another Start (lib.rs:660).
- That thread waits on `lifecycle`, sees the pipeline running and returns Ok, producing a second "Replay buffer running" toast, sound and banner, plus another device enumeration.
- At boot, `start_buffer_if_configured` and the first `poll_always` hit this.
- `toggle_buffer_and_notify` (lib.rs:335) and the `start_buffer`/`stop_buffer` commands also decide outside the lock. Both commands are implemented as toggles (commands.rs:326-348), so a race can act in the wrong direction.
- `AutoBuffer` is five separate atomics, so `manual_stop` can tear against `poll` between its load of `suppressed` and its store to `started`.
- **Fix:** an "in-flight" flag or `lifecycle.try_lock()` in `apply_auto_buffer`; idempotent `set_buffer(on)` checked under `lifecycle`; `AutoBuffer` as one `Mutex<AutoState>`.

**10. [Medium] [bug] Game detection treats maximized windows as fullscreen games. Likely.**
- game.rs:453-456: `rect.left <= screen.left && rect.top <= screen.top && rect.right >= screen.right && rect.bottom >= screen.bottom` against `rcMonitor`.
- A maximized framed window's `GetWindowRect` hangs about 8 px past the work area on every side. On a monitor with no taskbar (common on secondary screens) or with an auto-hide taskbar, the work area is the whole monitor, so any maximized unknown app counts (blender.exe, photoshop.exe, unity.exe, mstsc.exe, …).
- **Effect:** in only-in-game mode the buffer starts, game audio binds to that app, and clips are filed under its title.
- **Fix:** require `!IsZoomed(hwnd)` and no `WS_CAPTION`/`WS_THICKFRAME`, or use `SHQueryUserNotificationState` (QUNS_BUSY or D3D fullscreen).

**11. [Medium] [perf/robustness] Heavy sync commands block the UI thread. Confirmed.**
- Sync, so on the main thread: `set_config` (stops and restarts the pipeline on a source change, commands.rs:117-121), `list_encoders` (MFT enumeration), `list_capture_targets` (EnumWindows + QueryDisplayConfig), `list_audio_devices` and `list_audio_processes` (COM).
- `file_clip` holds the library lock across `filing::place`, whose rename retries sleep up to 600 ms in total.
- `delete_clip` does file I/O; `set_clip_dir` writes a probe file, which can stall on a network path.
- **Fix:** `#[tauri::command(async)]` for all of these.

**12. [Medium] [perf] Status loop design and polling cost. Confirmed.**
- One thread mixes 20 Hz UI work with slow polling. Any stall freezes the meters, game detection and the automation together, e.g. `refresh_free_bytes` on a dead network drive, or COM `list_devices` on a bad audio driver.
- Per 2 s tick:
  - `QueryDisplayConfig` twice (watch_monitors, screen_drift); frequent display-config polling is a reported cause of periodic stutter on some drivers (speculative);
  - `OpenInputDesktop` three times;
  - endpoint enumeration plus property stores;
  - a freshly spawned thread doing a Toolhelp snapshot with one String per process, plus audio session enumeration;
  - foreground detection, with a second `OpenProcess` in `foreground_is_ours`.
- Per 50 ms: `config_snapshot().sources` clones the whole `AppConfig` (lib.rs:946), and `std::env::var("CLIPPIBOY_LOG_LEVELS")` is looked up (lib.rs:963).
- **Fix:** separate threads for meters and monitoring; an `Arc<AppConfig>` swapped on change; event-driven sources (see the improvements list).

**13. [Low-Medium] [bug] Two stop requests for one recording produce a spurious "Recording failed" banner. Likely, in the disk-full case.**
- lib.rs:1043-1048 spawns `stop_recording_and_notify` every second while `recording_failure()` is Some, with no guard. Only `guard_recording_room` uses `STOPPING`.
- Both conditions fire together when the disk fills. The losing thread returns "No recording is running." and plays an error sound and banner on top of "Saving recording…".
- **Fix:** a shared stopping flag, or make the failure report one-shot.

**14. [Low-Medium] [bug] Alt+F4 on the console destroys it for the rest of the session. Confirmed by Tauri semantics.**
- lib.rs:1276-1280 only prevents close for `"main"`. The console takes focus, so Alt+F4 destroys it.
- After that, `console::open` logs "console window missing" until the app restarts.
- **Fix:** prevent close for the console and call `console::close`, or recreate it lazily in `open()`.

**15. [Low-Medium] [panic, release build aborts] tint.rs:36 crashes on a non-ASCII accent colour. Confirmed by test.**
- `u8::from_str_radix(&hex[at..at + 2], 16)` runs after a byte-length check only. `"#€ab1"` passes the check and panics.
- `config::load` does not call `clean_accent` (only `set_config` does), so a hand-edited config.json crashes the app at `tray::build` on every start.
- **Fix:** `if !hex.is_ascii() { return None }`, and sanitize in `load`.
- Other panic sites: game.rs:321 `text.truncate(text.len() - suffix.len())` after `to_lowercase()` is safe. I enumerated every codepoint: only U+212A (Kelvin) lowercases to ASCII, and the cut still lands on a char boundary. Everything else in my files is guarded or test-only; `lib.rs:1367 expect` is startup-only.

**16. [Low] [bug] The banner can get the wrong size or land partly off-screen on monitors with different scaling. Likely.**
- overlay.rs:532-533 calls `set_size(PhysicalSize)` and then `set_position`. Crossing to a monitor with another scale triggers WM_DPICHANGED, and tao rescales the window (allow_resize is true for non-maximized windows; checked in the source).
- The console works around this by placing twice; the overlay does not.
- **Fix:** position, then size, then position again.

**17. [Low] [bug] The overlay appears in Alt+Tab and Task View while it is visible, which is the whole recording when the REC badge is on. Likely.**
- tao implements `skip_taskbar` with `ITaskbarList::DeleteTab` only. The overlay is not `focusable(false)` and has no WS_EX_TOOLWINDOW or WS_EX_NOACTIVATE.
- It does not steal focus on show: `MARKER_DONT_FOCUS` persists, so tao uses SW_SHOWNOACTIVATE every time (checked in the source).
- **Fix:** `.focusable(false)`, or add WS_EX_TOOLWINDOW|WS_EX_NOACTIVATE.

**18. [Low] [bug] Free space of 0 means both "unknown" and "disk completely full". Confirmed.**
- lib.rs:860: `disk::free_bytes(...).unwrap_or(0)`, and `guard_recording_room` returns early when it reads 0 (lib.rs:890-893).
- A recording can therefore start on a full disk, and the room guard is off exactly at 0.
- **Fix:** a separate "known" flag, or `u64::MAX` as the sentinel.

**19. [Low] [bug] A user override in games.json may or may not win, at random. Confirmed by reading.**
- game.rs:198-206: `map.extend(extra)` runs before keys are lowercased. `"CS2.exe"` and the built-in `"cs2.exe"` then collide in `.collect()`, and HashMap iteration order picks the winner.
- **Fix:** lowercase the user's keys before `extend`.

**20. [Low] [quality] Detection details.**
- A title-derived name is overwritten on every tick (state.rs:671 `(Some(_), _) => *held = name.clone()`). Titles that change ("Loading", map names) fragment folders and spam presence updates; pin the name per PID.
- `exe_name` uses a MAX_PATH buffer (game.rs:408), so longer paths are never detected.
- UWP games behind `applicationframehost.exe` (ignored) are never detected.
- `from_exe` does not trim ".EXE" in upper case.

**21. [Low] [robustness] Config saving.**
- `config::save` uses one fixed `config.json.tmp`. `replace_config` (main thread, saves outside the lock) can race `repair_screens` (status thread). Interleaved writes could corrupt the file, and `load()` then falls back to defaults, losing all settings. Speculative and rare.
- `set_config` replaces the whole object from the page's copy. A stale settings page reverts `repair_screens`' monitor corrections, because `keep_once_flags` only covers two flags.
- **Fix:** serialize saves, use unique tmp names, keep a `.bak`, struct-level `#[serde(default)]`, and send patch-style updates from the UI.

**22. [Low] [bug] Tray and notice desyncs.**
- The tray's CheckMenuItem is toggled automatically by the menu library when clicked. After a failed start it stays checked, because `refresh` only calls `set_checked` when `buffer_active` changes (tray.rs:345-357). Likely.
- A Stream Deck "stop" while the buffer is already off still plays "Replay buffer stopped" with sound and banner.

**23. [Low] [anti-cheat]**
- game.rs is fine: `PROCESS_QUERY_LIMITED_INFORMATION` only, no module enumeration, and it works for elevated games.
- The audio process picker (audio/devices.rs:91-98) opens every audio-session process, including the game, with `PROCESS_VM_READ`. Use `QueryFullProcessImageNameW` like game.rs does.
- tao's `set_focus` falls back to injecting a synthetic Alt press via `SendInput` when `SetForegroundWindow` fails, which some anti-cheats flag (speculative).

**24. [Quality]**
- **AppState is a god object.** It mixes config persistence, capture lifecycle, game tracking, automation, file naming, and building `Clip` (written out three times: state.rs:859, 941, 1123). `now_ms` is duplicated.
  - Seven game-tracking mutexes are always updated together. Make them one `Mutex<GameTracker>`: that removes the "one lock at a time" dance, and `game_for` would no longer read three locks separately and get an inconsistent picture.
  - Move pipeline, recorder, buffer_wanted, active_recording and `lifecycle` into a `CaptureController` with a single state enum. `status.buffer_active` and `is_buffering()` are currently two sources of truth.
  - Make `track_game` a pure, testable function.
- **lib.rs (1385 lines):** split into notify/actions, hotkeys, status loop and setup.
- **Errors:** everything is `Result<_, String>`, while thiserror is already a dependency.
- **Stale comments:** the `shared` field doc says `save_clip` holds `pipeline` across ffmpeg; "stopping waits for a save in progress"; the console hotkey thread comment; tray.rs "several times a second"; "Reassign the two global hotkeys" (there are five).
- **German mixed into an English codebase,** including user-facing log lines: "Overlay bleibt klickbar", "Unbekannter Tray-Eintrag", "Pegel:", "blieb liegen", "Taktgeber", "Mischer".
- **Tests:** game.rs's naming tests are meaningful (they use real regression data). Missing: the fullscreen heuristic, the games.json merge, and `AutoBuffer::poll`, `poll_always` and the manual-action interplay, which is exactly where findings 3, 5 and 8 sit. Only `poll_lock` and `assigned_game` are tested.

**model.rs vs src/lib/types.ts:** I compared every struct and enum field by field (AppConfig 33/33, EngineStatus 14/14, Clip 18/18, Friends, Sounds, Overlay, Accent `color_2`→`color2`, SourceKind tagging, BannerKind, console `Layout`, overlay `Placement`, ShotEdit/Rect, `tools::Status`). There are no casing or optionality mismatches. Minor notes:
- `SourceKind::Leftovers` has no TS counterpart; it is only read and is migrated at load.
- `Banner.avatar` and `corner` are always sent but optional in TS.
- `UpdateProgress.finished` exists only in TS, and is handled with `Omit<…>`.

### Improvements and missing features
- **Event-driven instead of polling:**
  - `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT)` for game detection: instant, with no injection into the game.
  - A process `SYNCHRONIZE` handle plus `RegisterWaitForSingleObject` for game exit, replacing `still_running` and its PID-reuse checks.
  - `WTSRegisterSessionNotification`, or polling `WTSQuerySessionInformation(WTSSessionInfoEx)` lock flags. This tells the lock screen from a UAC prompt, so the 10 s LOCK_POLLS grace becomes unnecessary.
  - `WM_DISPLAYCHANGE` or `RegisterDeviceNotification(GUID_DEVINTERFACE_MONITOR)` for monitors.
  - `IMMNotificationClient` and `IAudioSessionNotification` for audio devices and sessions.
- **Better game detection:**
  - Use the full path `QueryFullProcessImageNameW` already returns (game.rs throws it away): `steamapps\common\<Folder>`, Epic, XboxGames, GOG and Riot paths give windowed games and stable names.
  - Read the Xbox Game Bar's own game list at `HKCU\System\GameConfigStore\Children` (`MatchedExeFullPath`).
- **Capture state machine** with `Origin::{User, Automation, StreamDeck, Lock}`, idempotent commands, and backoff.
- **Graceful quit:** wait for in-flight saves, finalize, then exit. Handle Windows logoff.
- **One overlay/UI-effects worker thread** fed by a channel. This removes blocking main-thread round trips from worker threads and with them the class of bug in finding 1.
- **Overlay:**
  - Skip banners when `exclusive_fullscreen()` is true.
  - A REC badge window over the game for the whole recording can force composition and lose independent flip, adding latency (speculative); offer a tray-only REC indicator.
  - Pause CSS animations while hidden: occlusion detection and backgrounding are disabled, and rendering is CPU-only (`--disable-gpu`).
- **Memory:** destroy the main webview while hidden in the tray, and create the console lazily.
- **Hardening:** a Tauri app-command ACL (`app_manifest` in build.rs). Today the overlay and console windows can invoke every app command.
