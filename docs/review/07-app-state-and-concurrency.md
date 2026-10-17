# App shell, shared state and concurrency

**Scope:** state.rs, game.rs, tray.rs, overlay.rs, console.rs, sounds.rs, tint.rs, model.rs vs types.ts, lib.rs setup and threads.

**Findings:** 35 — 21 verified (3 medium or higher), 12 reported but not independently re-verified, 2 unverifiable/disputed, 0 rejected by verification.

Severity shown is the verified severity; where verification changed it, the reported one is in parentheses. See [README](README.md#how-findings-were-verified) for how verification worked.

## Verified issues (medium and above)

### ST-1 · High (reported Critical) · current_game MutexGuard held across overlay::show, which blocks on the main thread: possible permanent deadlock

- **Where:** `lib.rs:216-221`, `lib.rs:356`, `overlay.rs:404`, `overlay.rs:479`, `commands.rs:316`, `state.rs:736` (+3 more)
- **Verification:** confirmed (impact: confirmed; reachability: partly confirmed; code: confirmed)

**What verification found:** The mechanism is real as described: a worker thread holds current_game through at least two blocking main-thread round trips (is_visible, hwnd), and three main-thread paths lock current_game, so a parking_lot lock on the main thread at that moment hangs the event loop for good. Two details of the claim are wrong. (1) The window is narrow and its timing differs from the scenario. W holds the guard only from the lock() in the show() argument until the hwnd() round trip returns: sub-millisecond when the main thread is idle, longer only when the main thread is busy. For the hotkey pair, that window comes after start_recording has returned, which can be hundreds of ms after the record press when the pipeline has to launch, not 'within a few milliseconds' of it. The toggle-buffer press must land inside that window while a recording runs and the buffer is off, which leads to the state.rs:472 lock. (2) The boot scenario needs settings that are off by default: buffer.auto_start=true AND only_buffer_in_game=false (config.rs:40/61). With the defaults, start_buffer_if_configured returns at once, and engine_status runs only once per console page load. So this is a rare, user-self-inflicted race (two of the user's own actions colliding within about a millisecond). The outcome is a permanent hang that needs the process killed. It is not a critical, routinely hit defect. Severity: medium.

**Original claim:** At lib.rs:216-221 `overlay::show(app, BannerKind::Recording, "Recording", state.current_game.lock().clone());` and at lib.rs:356 `Some(match state.current_game.lock().as_ref() { … })` the MutexGuard is a temporary in a call argument (or a `match` scrutinee), so it lives until the end of the statement, which includes the whole of `overlay::show`. Inside `overlay::show`, `window.is_visible()` (overlay.rs:404) and `window.hwnd()` (overlay.rs:479) are blocking main-thread round trips when called from a worker thread (tauri-runtime-wry's `getter!` sends a message to the main thread and waits on `rx.recv()`). These main-thread paths lock `current_game`: `engine_status` (sync command, commands.rs:316) -> `status_snapshot` (state.rs:736); the toggle-buffer hotkey (runs on the main thread) -> `start_pipeline` (state.rs:472/515); `set_config` -> `start_buffer_and_notify`. The worker holds `current_game` while waiting on the main thread, and the main thread waits on `current_game`, so the app deadlocks. Report: rare but a permanent hang; the pattern is confirmed, the interleaving is plausible.

**Scenario:** At boot, `start_buffer_if_configured`'s thread takes `current_game` at lib.rs:356 and queues IsVisible. At the same moment the console page mounts and calls `api.status()` (Console.tsx:337); the main thread dispatches that IPC first and blocks on `current_game`. Neither side can continue: the event loop is dead, so all windows, the tray and the hotkeys (which arrive through the same message loop) hang for good. A second trigger: the record hotkey, then the toggle-buffer hotkey within a few milliseconds.

**Fix:** Read the value into a local first (`let game = state.current_game.lock().clone(); overlay::show(…, game)`) at both sites. Make it a rule that no AppState guard is alive while calling window APIs. Better still, send banners through a channel to one overlay worker thread.

**Verifier on the fix:** The proposed fix is correct and enough for these two sites: bind the value in its own `let` statement (`let game = state.current_game.lock().clone();` and `let detail = match state.current_game.lock().as_ref() {...};`) so the guard drops before overlay::show. It has no side effects. To remove the main-thread side of this class of bug, also make engine_status `#[tauri::command(async)]` and spawn the toggle-buffer hotkey's work onto a thread like the other hotkeys (it currently runs start_pipeline, a few hundred ms of capture setup, on the UI thread). A single overlay worker thread is not neede…

### ST-2 · Medium (reported High) · Toggle-buffer hotkey starts and stops the capture stack on the UI (main) thread

- **Where:** `lib.rs:813-816`, `lib.rs:786`, `state.rs:486`, `mft.rs:1135`, `state.rs:1014-1020`, `src-tauri/src/lib.rs:811-816` (+3 more)
- **Verification:** confirmed (code: confirmed; reachability: confirmed; impact: confirmed)
- **Also reported as:** LS-10 (Toggle-buffer hotkey starts/stops the pipeline inline on the UI thread; mislead…)

**Problem:** Global-shortcut Pressed handlers run on the main thread (inside the hidden hotkey window's window procedure). lib.rs:813-816: `if event.state() == ShortcutState::Pressed { toggle_buffer_and_notify(app); }` runs the toggle directly, while every other hotkey spawns a thread. The work done there: `launch_pipeline` spawns `ffmpeg -version` every time, because at state.rs:486 `!muxer::available() && tools::is_ready()` evaluates the expensive check first; `Pipeline::start` waits up to 10 s for encoder readiness (mft.rs:1135, `recv_timeout(10s)`); `lifecycle` may be held elsewhere, e.g. by `attach_recorder`'s 5 s keyframe loop (state.rs:1014-1020) or a `close_recording` whose writer is flushing to a slow disk; stop drains the encoder with unbounded joins. The comment at lib.rs:786 ("the hotkey arrives on one of its own") is wrong. Latent: the plugin's `unregister_all` holds its `shortcuts` lock while it waits on the main thread, and the hotkey handler takes the same lock on the main thread, so calling `register_hotkeys` off the main thread would deadlock (today only sync commands and `setup` call it, so it is not reachable yet).

**Scenario:** Pressing the toggle-buffer hotkey freezes every webview (main window, console, banner) while the pipeline starts or stops; every worker thread that needs a window query stalls too, including the status thread's `window_awake` and banner display. This is also what makes the current_game deadlock (ST-1) possible.

**Fix:** `std::thread::spawn(move || toggle_buffer_and_notify(&app))`, and swap the operand order at state.rs:486. Keep `register_hotkeys` on the main thread.

**Verifier on the fix:** Spawning a thread, as tray.rs:113 does (`let app = app.clone(); std::thread::spawn(move || toggle_buffer_and_notify(&app));`), is correct, sufficient for this handler, and consistent with every other route. It also removes the main-thread side of the ST-1 hotkey-pair deadlock. Side effect: two quick presses no longer queue one after the other on the main thread and can both see buffer_active=false. That race already exists between tray and control port, and the result is harmless (start is idempotent, so you get a duplicate toast). The state.rs:486 swap is close to pointless in steady state.…

### ST-3 · Medium (reported High) · Persistent start failure makes the automation retry every 2 s with an error sound and banner

- **Where:** `state.rs:188-190`, `state.rs:246-252`, `lib.rs:363-366`
- **Verification:** confirmed (reachability: confirmed; code: confirmed)

**Problem:** state.rs:188-190: `if !buffer_active && !self.suppressed.load(SeqCst) { self.started.store(true, SeqCst); return AutoAction::Start; }`. `poll_always` (state.rs:246-252) has the same shape. Nothing records a start failure. Each failure runs `notify(error)`, `sounds::play(Error)` and `overlay::show(Error, "Buffer will not start")` (lib.rs:363-366). Error banners stay at least 6 s, so with a 2 s retry the banner never goes away. Each attempt also builds and tears down D3D, WGC, MFT and the ffmpeg probe during gameplay.

**Scenario:** ffmpeg is quarantined by antivirus after `tools` reported ready, encoder session limits are hit, or the GPU device is lost: the automation retries Start every 2 s forever, playing an error sound and keeping an error banner up permanently during gameplay.

**Fix:** Keep a failure count with exponential backoff. After N failures, set `suppressed` and announce it once.

**Verifier on the fix:** The proposed fix fits: count consecutive automation start failures in AutoBuffer and back off. After N failures set `suppressed` (which is already cleared when the game ends, state.rs:200) and announce it once. Reset the counter on success, in manual_start and when the game ends. Optionally only announce the first failure.

## Reported, not independently re-verified

The original reviewer checked these against the code, but the independent verification pass was stopped before reaching them. Treat as likely, confirm before fixing.

### ST-4 · High · Starting a second instance rotates and destroys the running instance's log

- **Where:** `lib.rs:1153`, `lib.rs:1185`, `logging.rs:59-63`
- **Verification:** not independently re-verified (not independently re-verified)

**Problem:** lib.rs:1153: `logging::init()` runs before `.plugin(tauri_plugin_single_instance…)`. Plugin setup only runs at `.build()`, and `.manage(AppState::new())` (lib.rs:1185) is also evaluated eagerly before that. `rotate()` (logging.rs:59-63) renames the live `clippiboy.log` over `clippiboy.log.1`, destroying the previous session's log, then creates a new file. Rust opens files with share-delete, so the rename succeeds while the first instance has the log open. The second instance also loads config, enumerates MFT encoders (`encode::resolve`), runs SQLite migrations with no busy_timeout against the live database, and starts audio streams before it exits. Report severity: Medium-High.

**Scenario:** Every relaunch from the Start menu while the app sits in the tray, and every `clippiboy://` deep link (each Discord sign-in), leaves a 5-line stub as `clippiboy.log`. The live log moves to `.log.1` and the previous session's log is gone.

**Fix:** Do the single-instance check first (a named mutex in `main()` before `logging::init`, or a lazy `AppState`), and rotate only once this is known to be the primary instance.

- **ST-10 · Medium · Duplicate start/stop actions while one is still in progress (decisions taken outside lifecycle)** — `lib.rs:660`, `lib.rs:335`, `commands.rs:326-348` (+1 more)  
  `buffer_active` only becomes true at the end of `launch_pipeline`, so any 2 s automation tick during a slow start (cold boot, or `Pipeline::start` taking up to 10 s) spawns another Start (lib.rs:660). That thread waits on `lifecycle`, sees the pipeline running and returns Ok, producing a second "Replay buffer running" toast, sound and banner, plus another device enumeration. `toggle_buffer_and_notify` (lib.rs:335) and the `start_buffer`/`stop_buffer` commands also decide outside the lock; both commands are implemented as toggles (commands.rs:326-348), so a race can act in the wrong direction.…  
  *Fix:* An "in-flight" flag or `lifecycle.try_lock()` in `apply_auto_buffer`; an idempotent `set_buffer(on)` checked under `lifecycle`; `AutoBuffer` as one `Mutex<AutoState>`.
- **ST-11 · Medium · Game detection treats maximized windows as fullscreen games** — `game.rs:453-456`  
  game.rs:453-456 checks `rect.left <= screen.left && rect.top <= screen.top && rect.right >= screen.right && rect.bottom >= screen.bottom` against `rcMonitor`. A maximized framed window's `GetWindowRect` hangs about 8 px past the work area on every side. On a monitor with no taskbar (common on secondary screens) or with an auto-hide taskbar, the work area is the whole monitor, so any maximized unknown app counts as a fullscreen game (blender.exe, photoshop.exe, unity.exe, mstsc.exe, …).  
  *Fix:* Require `!IsZoomed(hwnd)` and no `WS_CAPTION`/`WS_THICKFRAME`, or use `SHQueryUserNotificationState` (QUNS_BUSY or D3D fullscreen).
- **ST-12 · Medium · Heavy sync Tauri commands block the UI thread** — `commands.rs:117-121`, `commands.rs`, `src-tauri/src/clipboard.rs:42-47` (+1 more)  
  These commands are sync, so they run on the main thread: `set_config` (stops and restarts the pipeline on a source change, commands.rs:117-121), `list_encoders` (MFT enumeration), `list_capture_targets` (EnumWindows + QueryDisplayConfig), `list_audio_devices` and `list_audio_processes` (COM). `file_clip` holds the library lock across `filing::place`, whose rename retries sleep up to 600 ms in total. `delete_clip` does file I/O; `set_clip_dir` writes a probe file, which can stall on a network path.  
  *Fix:* `#[tauri::command(async)]` for all of these.
- **ST-13 · Medium · Status loop mixes 20 Hz UI work with slow polling, and polling is costly** — `lib.rs:928`, `lib.rs:946`, `lib.rs:963`  
  The status thread (`spawn_ui_updates`, lib.rs:928) runs one 50 ms loop forever (no stop flag, no join) that mixes 20 Hz UI work with slow polling, so any stall freezes the meters, game detection and the automation together (e.g. `refresh_free_bytes` on a dead network drive, or COM `list_devices` on a bad audio driver). Per 2 s tick: `QueryDisplayConfig` twice (watch_monitors, screen_drift) — frequent display-config polling is a reported cause of periodic stutter on some drivers (speculative); `OpenInputDesktop` three times; endpoint enumeration plus property stores; a freshly spawned thread d…  
  *Fix:* Separate threads for meters and monitoring; an `Arc<AppConfig>` swapped on change; event-driven sources (see proposals).
- **ST-14 · Medium · Two stop requests for one recording produce a spurious "Recording failed" banner** — `lib.rs:1043-1048`  
  lib.rs:1043-1048 spawns `stop_recording_and_notify` every second while `recording_failure()` is Some, with no guard; only `guard_recording_room` uses `STOPPING`. Both conditions fire together when the disk fills. The losing thread returns "No recording is running." and plays an error sound and banner on top of "Saving recording…". Report severity: Low-Medium; likely in the disk-full case.  
  *Fix:* A shared stopping flag, or make the failure report one-shot.
- **ST-15 · Medium · Alt+F4 on the console destroys it for the rest of the session** — `lib.rs:1276-1280`  
  lib.rs:1276-1280 only prevents close for the `"main"` window. The console takes focus, so Alt+F4 destroys it. After that, `console::open` logs "console window missing" until the app restarts. Report severity: Low-Medium; confirmed by Tauri semantics.  
  *Fix:* Prevent close for the console and call `console::close`, or recreate it lazily in `open()`.
- **ST-16 · Medium · tint.rs:36 panics on a non-ASCII accent colour (release build aborts on every start)** — `tint.rs:36`  
  tint.rs:36: `u8::from_str_radix(&hex[at..at + 2], 16)` runs after a byte-length check only, so the byte slice can split a multi-byte character and panic; `"#€ab1"` passes the check and panics (confirmed by test). `config::load` does not call `clean_accent` (only `set_config` does), so a bad value from config.json reaches `tray::build`. Release builds abort on panic. Report severity: Low-Medium.  
  *Fix:* `if !hex.is_ascii() { return None }`, and sanitize the accent in `load`.
- **ST-5 · Medium · Stream Deck start/stop bypass the automation's "user decided" flags** — `control.rs:225-226`, `src-tauri/src/control.rs:225-226`, `src-tauri/src/lib.rs:333-342` (+1 more)  
  control.rs:225-226 call `"start" => crate::start_buffer_and_notify(app)` and `"stop" => crate::stop_buffer_and_notify(app)` directly, with no `auto.manual_start()` / `manual_stop()` call, so the buffer automation does not know the user decided.  
  *Fix:* Route through one `set_buffer(app, on, Origin::User)`.
- **ST-6 · Medium · Failed recording close leaves status.recording = true and an orphaned capture pipeline running** — `state.rs:1072-1078`, `src-tauri/src/state.rs:1072-1078`  
  state.rs:1072-1078: `Err(err) => { log::error!(…); return None; }` returns before `if !self.buffer_wanted() { self.halt_pipeline(); }` and before `self.status.lock().recording = false`. The recorder has already been removed from both AppState and the pipeline. `finish` fails when `write_meta` fails, which is exactly the disk-full case the room guard is there for.  
  *Fix:* A scope guard that always halts the pipeline (when the buffer is not wanted) and clears the recording flag.
- **ST-7 · Medium · Quit does not wait for a save in progress and can hang on the main thread** — `tray.rs:139-148`  
  tray.rs:139-148 Quit runs `abandon_recording(); stop_pipeline(); app.exit(0)`, and so does the updater's `on_before_exit`. None of them check `is_saving()`. The save thread dies while ffmpeg is running (there is no Job object). In addition, the Quit handler runs on the main thread and joins encoder threads with no timeout.  
  *Fix:* Do Quit on a worker thread: show a "finishing clip…" banner, wait for saving to clear with a timeout, then exit. Optionally handle WM_QUERYENDSESSION so a Windows logoff closes recordings cleanly.
- **ST-9 · Medium · "Only buffer in game": an alt-tab of 30 s or more cancels the user's manual stop, contradicting the README** — `state.rs:195-200`  
  `poll` is fed `GameTick::playing`, which depends on what is in the foreground. state.rs:195-200: after `GRACE_POLLS` checks without the game in front, `self.suppressed.store(false, SeqCst)`. The README says the automation "holds back until the game has ended".  
  *Fix:* Clear `suppressed` only when the bound game process exits (`game_pid` becomes None); consider pausing rather than stopping while the game process is still alive.

## Verified lower-severity issues

| ID | Severity | Title | Where | Fix |
|---|---|---|---|---|
| ST-17 | Low | Banner can get the wrong size or land partly off-screen on monitors with different scaling | overlay.rs:532-533 | Position, then size, then position again. |
| ST-18 | Low | Overlay appears in Alt+Tab and Task View while visible (the whole recording with the REC badge on) | overlay.rs | `.focusable(false)`, or add WS_EX_TOOLWINDOW\|WS_EX_NOACTIVATE. |
| ST-19 | Low | Free space of 0 means both "unknown" and "disk completely full" | lib.rs:860, lib.rs:890-893 | A separate "known" flag, or `u64::MAX` as the sentinel. |
| ST-20 | Low | A user override in games.json may or may not win, at random | game.rs:198-206 | Lowercase the user's keys before `extend`. |
| ST-21 | Low | Title-derived game name is overwritten on every tick | state.rs:671 | Pin the name per PID. |
| ST-23 | Low | UWP games behind applicationframehost.exe are never detected | game.rs | No explicit fix in the report (see the Xbox Game Bar / full-path detection proposals). |
| ST-26 | Low | set_config replaces the whole config from the page's copy, reverting repair_screens corrections | commands.rs | Send patch-style updates from the UI. |
| ST-27 | Low | Tray buffer CheckMenuItem stays checked after a failed start | tray.rs:345-357 | No explicit fix in the report (implied: re-sync the checked state after each click regardless of whether `buffer_active` changed). |
| ST-31 | Low | AppState is a god object with fragmented game-tracking locks and two sources of truth for capture state | state.rs:859, state.rs:941 | Make the seven game-tracking mutexes one `Mutex<GameTracker>`; move pipeline, recorder, buffer_wanted, active_recording and `lifecycle` into a `CaptureController` with a single state enum; make `track_game` a pure, test… |
| ST-34 | Low | Stale comments | state.rs, tray.rs | No explicit fix in the report (implied: update or remove the stale comments). |
| ST-36 | Low | Missing tests for fullscreen heuristic, games.json merge and AutoBuffer automation | game.rs, state.rs | No explicit fix in the report (implied: add tests for these areas; make `track_game` pure to enable it). |
| ST-24 | Info (reported Low) | from_exe does not trim an upper-case ".EXE" suffix | game.rs | No explicit fix in the report (implied: trim the extension case-insensitively). |
| ST-28 | Info (reported Low) | Stream Deck "stop" while the buffer is already off still announces "Replay buffer stopped" | control.rs | No explicit fix in the report (implied: idempotent `set_buffer(on)` as in the capture state machine proposal). |
| ST-32 | Info (reported Low) | lib.rs is 1385 lines | lib.rs | Split into notify/actions, hotkeys, status loop and setup modules. |
| ST-35 | Info (reported Low) | German text mixed into an English codebase, including user-facing log lines |  | No explicit fix in the report (implied: translate to English). |
| ST-37 | Info | SourceKind::Leftovers has no TypeScript counterpart | model.rs, src/lib/types.ts | None proposed. |
| ST-38 | Info | Banner.avatar and corner are always sent but optional in TS | model.rs, src/lib/types.ts | None proposed. |
| ST-39 | Info | UpdateProgress.finished exists only in TS | src/lib/types.ts | None proposed. |

## Unverifiable from source or disputed

| ID | Reported | Title | What is uncertain |
|---|---|---|---|
| ST-22 | Low | exe_name uses a MAX_PATH buffer, so games at longer paths are never detected | game.rs:407-421: `let mut buf = [0u16; MAX_PATH as usize]; let mut len = buf.len() as u32; let result = QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut len); ... result.ok()?;` - a failed call returns None, and foreground(… |
| ST-30 | Low | tao's set_focus can inject a synthetic Alt press via SendInput | tao-0.35.3 src/platform_impl/windows/window.rs:175-186 set_focus calls `force_window_active(window.0)` when the window is visible, not minimized and not foreground. window.rs:1500-1527: `if SetForegroundWindow(handle).as_bool() { return; }`, then two KEYEVENT… |

## What is solid

- No cycle among AppState locks: observed lock orders are lifecycle (always outermost) -> {pipeline, shared->packets, status, recorder, current_game, buffering_game, active_recording, config}; status -> shared -> shared.encoder (state.rs:518-526); status -> recorder (state.rs:681); recorder -> shared (state.rs:711-713); shared -> packets or source; pipeline -> packets -> shared.recorder (pipeline.r…
- game.rs:321 `text.truncate(text.len() - suffix.len())` after `to_lowercase()` is safe: every codepoint was enumerated; only U+212A (Kelvin) lowercases to ASCII, and the cut still lands on a char boundary.
- Other panic sites in the reviewed files are guarded or test-only; `lib.rs:1367 expect` is startup-only.
- `register_hotkeys` is currently called only from sync commands and `setup` (main thread), so the latent plugin `unregister_all`/`shortcuts` deadlock is not reachable today.
- The overlay does not steal focus on show: `MARKER_DONT_FOCUS` persists, so tao uses SW_SHOWNOACTIVATE every time (checked in tao source).
- The console window already works around cross-monitor DPI rescaling by placing itself twice.
- game.rs is fine for anti-cheat: `PROCESS_QUERY_LIMITED_INFORMATION` only, no module enumeration, and it works for elevated games.
- game.rs's naming tests are meaningful (they use real regression data).
- model.rs vs src/lib/types.ts compared field by field (AppConfig 33/33, EngineStatus 14/14, Clip 18/18, Friends, Sounds, Overlay, Accent `color_2`->`color2`, SourceKind tagging, BannerKind, console `Layout`, overlay `Placement`, ShotEdit/Rect, `tools::Status`): no casing or optionality mismatches.

## Improvement opportunities and missing features

| ID | Idea | Why | Effort | Sketch |
|---|---|---|---|---|
| ST-P-1 | Event-driven foreground tracking for game detection | Instant game detection with no injection into the game, replacing 2 s foreground polling. | ? | `SetWinEventHook(EVENT_SYSTEM_FOREGROUND, WINEVENT_OUTOFCONTEXT)` for game detection. |
| ST-P-2 | Wait on the game process handle for game exit | Reliable game-exit detection without polling, removing `still_running` and its PID-reuse checks. | ? | A process `SYNCHRONIZE` handle plus `RegisterWaitForSingleObject` for game exit, replacing `still_running` and its PID-reuse checks. |
| ST-P-3 | Session lock notifications instead of OpenInputDesktop polling | Tells the lock screen from a UAC prompt, so the 10 s LOCK_POLLS grace becomes unnecessary. | ? | `WTSRegisterSessionNotification`, or polling `WTSQuerySessionInformation(WTSSessionInfoEx)` lock flags. |
| ST-P-4 | Monitor change notifications instead of QueryDisplayConfig polling | Removes twice-per-2s `QueryDisplayConfig` polling (a possible stutter source). | ? | `WM_DISPLAYCHANGE` or `RegisterDeviceNotification(GUID_DEVINTERFACE_MONITOR)` for monitors. |
| ST-P-5 | Audio device and session notifications instead of enumeration polling | Removes per-2s endpoint/session enumeration and its stall risk on bad drivers. | ? | `IMMNotificationClient` and `IAudioSessionNotification` for audio devices and sessions. |
| ST-P-6 | Use the full executable path for game detection | Detects windowed games and gives stable names. | ? | Use the full path `QueryFullProcessImageNameW` already returns (game.rs throws it away): `steamapps\common\<Folder>`, Epic, XboxGames, GOG and Riot paths give windowed games and stable names. |
| ST-P-7 | Read the Xbox Game Bar's game list | Leverages Windows' own list of known games. | ? | Read `HKCU\System\GameConfigStore\Children` (`MatchedExeFullPath`). |
| ST-P-8 | Capture state machine with origins, idempotent commands and backoff | Fixes the class of start/stop races, Stream Deck bypass and retry storms in one place. | ? | A capture state machine with `Origin::{User, Automation, StreamDeck, Lock}`, idempotent commands, and backoff. |
| ST-P-9 | Graceful quit and Windows logoff handling | No lost clips on quit or logoff. | ? | Wait for in-flight saves, finalize, then exit. Handle Windows logoff. |
| ST-P-10 | One overlay/UI-effects worker thread fed by a channel | Removes blocking main-thread round trips from worker threads and with them the class of bug in ST-1. | ? | Send banner/UI-effect requests through a channel to a single overlay worker thread. |
| ST-P-11 | Skip banners in exclusive fullscreen | Avoids banners over exclusive-fullscreen games. | ? | Skip banners when `exclusive_fullscreen()` is true. |
| ST-P-12 | Tray-only REC indicator option | A REC badge window over the game for the whole recording can force composition and lose independent flip, adding latency (speculative). | ? | Offer a tray-only REC indicator instead of the on-screen REC badge. |
| ST-P-13 | Pause overlay CSS animations while hidden | Saves CPU: occlusion detection and backgrounding are disabled, and rendering is CPU-only (`--disable-gpu`). | ? | Pause CSS animations while the overlay is hidden. |
| ST-P-14 | Reduce memory by destroying hidden webviews | Lower memory footprint while the app sits in the tray. | ? | Destroy the main webview while hidden in the tray, and create the console lazily. |
| ST-P-15 | Tauri app-command ACL | Today the overlay and console windows can invoke every app command; an ACL limits each window to what it needs. | ? | A Tauri app-command ACL (`app_manifest` in build.rs). |
