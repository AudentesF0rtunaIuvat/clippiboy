ClippiBoy security review (read-only): local attack surface and command layer. Scope covered: commands.rs, lib.rs, control.rs, config.rs, clips.rs, filing.rs, clipboard.rs, thumbs.rs, logging.rs, main.rs, build.rs, tauri.conf.json, capabilities/*.json, windows/hooks.nsh and streamdeck/src/client.ts. To check claims about libraries I read the source of tiny_http 0.12.0, tauri 2.11.5 and tauri-plugin-global-shortcut 2.3.2. Those copies are under <review-scratch>/crates/. No repo files were changed.

I found no remote code execution, no SQL injection (every query uses bound parameters), and no path traversal through game names or names of files received from friends.

## Attack-surface map
- **Control port** (control.rs, tiny_http). Bound to 127.0.0.1:47653. It is **on by default for every install**. Any local process of any Windows user can connect. Requests are refused (403) if they carry no matching `x-clippiboy-token` (a UUIDv4) or if they carry Origin, Referer or Sec-Fetch-Site. There are no CORS headers. The Host header is not checked.
- **Handoff file** `%APPDATA%\ClippiBoy\streamdeck.json`, plus the token in config.json. Readable by the same user only. It lives in the Roaming profile.
- **Tauri IPC**: 79 commands (40 sync + 16 async in commands.rs, plus friends, links, share and telemetry). Trust level is the renderer of the main, overlay and console windows, all local content. Remote origins are blocked by Tauri 2.11 (webview/mod.rs:1823).
- **Capabilities**: these gate only plugin commands. Every app command is callable from every window (see Medium #2).
- **Asset protocol**: `$VIDEO/**`, a dead `$APPDATA/ClippiBoy/**` entry, plus recursive runtime grants for the clip folder, the folder of every clip already in the library, thumbs, tracks, originals and preview.
- **Deep link** `clippiboy://auth?code=` (friends.rs:147). Any web page can trigger it, after the browser's "open app?" prompt.
- **Single-instance arguments**: only `--autostart` is examined. Deep links are forwarded by the plugin.
- **Global hotkeys and tray**: user-trusted.
- **Remote data reaching the filesystem and parsers**: friends' share metadata (game, file name, width/height). Games are named from window titles of local processes. ffmpeg runs over files friends send.
- **Installer hooks**: pre-install waits for the exe to be released. Post-install writes the installer's file name to `%LOCALAPPDATA%\gg.clippiboy.app\install-source.txt`, which telemetry.rs:173 parses strictly.

## Findings, most severe first

### [Medium] [security/bug] A local process can crash ClippiBoy, or hang its UI, through the control port without the token — confirmed
- **Where**: control.rs:156-158 (`if !allowed { return respond(request, 403, …) }`) and `respond` at :325-336. The cause is in tiny_http 0.12.0:
  - request.rs:191-216 wraps any body larger than 1024 bytes in `EqualReader`.
  - `util/equal_reader.rs` has a `Drop` that does `let mut buf = vec![0; remaining_to_read];` and then blocks reading the rest of the body from the socket.
  - No socket timeouts are set anywhere.
  - `util/task_pool.rs` creates one thread per connection with `thread::spawn` and has no upper limit.
- **Mechanism**: the 403 is answered on the single serving thread. Dropping the request then allocates and reads whatever body size the client *declared*. That happens before, and regardless of, the token check.
- **Failure scenario**:
  - Any local process (including another Windows user's) can make that allocation fail. Allocation failure and capacity overflow abort the process, and so does a thread-spawn failure under `panic = "abort"`. The in-memory replay buffer is lost and a running recording is cut off.
  - A client that declares a body and then sends nothing stalls the serving thread forever. Stream Deck keys stop working.
  - `control::stop()` (:94-106) then waits on `thread.join()` for good. It is called from `set_config` (commands.rs:154-158) when the port or on/off changes, and from `regenerate_control_token` (commands.rs:1384). Both are sync commands on the main thread, so the UI freezes permanently.
  - Browsers cannot set Content-Length, so this is a local-only issue.
- **Fix**:
  - Replace tiny_http with a small server you control. A std `TcpListener`, or tokio (already a dependency), with: a read timeout of about 2 s, at most 8 KB of headers, rejection of `Content-Length` above 4 KB *before* building the request, closing the socket rather than draining it, and a cap of about 8 connections.
  - Or patch tiny_http.
  - Separately, make `stop()` not block forever: join with a timeout, or detach.

### [Medium] [security] Capabilities give a false sense of least privilege: every window can call all 79 app commands — confirmed
- **Where**: build.rs is plain `tauri_build::build()`. There is no `AppManifest`, and gen/schemas/acl-manifests.json has no `__app__` entry. Tauri 2.11.5 webview/mod.rs:1823 checks the ACL only `if (plugin_command.is_some() || has_app_acl_manifest || !is_local)`.
- **Effect**: overlay.json describes itself as "only listens, controls nothing", yet the overlay can call `delete_clip`, `set_config`, `clipboard_read_text`, `export_clip`, `friends_delete_account` and so on.
- **Scenario**: an XSS in any window, the overlay included, gets the full command set. React escaping and the absence of `dangerouslySetInnerHTML` make that unlikely today. This is defense in depth.
- **Fix**: use `tauri_build::Attributes::new().app_manifest(AppManifest::new().commands(&[...]))` and grant per window. The overlay needs roughly nothing. The console needs only its clip and friends subset.

### [Medium] [security] A compromised renderer gets user-level read and write on the whole filesystem — confirmed, defense in depth
- **`export_clip`** (commands.rs:1155-1169). `output: String` comes straight from the UI. `shrink` (:1094-1097) only refuses `output == source`. ffmpeg then runs with `-y` (export.rs:124) and the output as its last argument. Consequences:
  - Any writable file can be overwritten.
  - A UNC path makes Windows send NTLM credentials to a remote host.
  - ffmpeg treats an `http://…/x.mp4` output as an upload target, which sidesteps the CSP.
- **`set_clip_dir` / `set_config`** (commands.rs:266-290, :54-98). Any directory is accepted, and `allow_clip_dir` grants it *recursively* to the asset protocol (lib.rs:1095). Example: setting `C:\Users\X` makes browser profiles and `%APPDATA%\ClippiBoy\friends.json` (the session token) readable.
  - `set_config` does not validate `clip_dir` at all.
  - Speculative: with an empty `clip_dir`, Tauri builds the pattern `\**` (scope/fs.rs:489-498), which may match every canonicalized path.
- **`reveal_path(path)`** (:1347) accepts any path, UNC included.
- **Fix**:
  - Show the save and folder dialogs from Rust and keep the chosen path server-side, or at least require an absolute local drive path: no `\\`, no `scheme:`, an allowed extension.
  - Add `-protocol_whitelist file` to export, trim and thumbnail ffmpeg calls.
  - Canonicalize and validate `clip_dir` in `set_config`; refuse empty, system or profile-root folders.
  - Make `reveal_path` accept only paths inside the known folders.

### [Medium] [bug] `delete_clip` with an empty id (or a colliding one) wipes every clip's audio tracks and untouched originals — confirmed
- **Where**: commands.rs:680-683 runs `stems::remove(&id); edit::remove(&id); crate::thumbs::remove(&id);` *before* checking the clip exists.
- **Mechanism**:
  - `muxer::sanitize` (:470) maps "" to "". `stems::dir("")` is `tracks\`, and `remove_dir_all` deletes the whole store (stems.rs:44-46, :146-148).
  - `edit::remove("")` does `remove_dir_all(originals\)` (edit.rs:61-63, :114-118). That holds every trimmed clip's only untouched copy and every screenshot original.
  - The library delete is a no-op and the call returns `Ok`.
  - Because `-` is mapped to `_`, a renderer can also destroy another clip's stores by passing its UUID with underscores.
  - The deletion also happens before the database commit, so a failed `lib.delete` still loses the side files.
- **Fix**: look up the clip first and return `Err` if it is missing. Take the store paths from `clip.id` as stored. Make `sanitize` return an error for empty or non-UUID input. Commit the row delete before removing files.

### [Medium] [perf] Heavy work in sync commands and callbacks runs on the main thread — confirmed
Sync `#[tauri::command]` functions run on the event-loop thread. While one runs, the UI does not paint, and `window_awake` in the UI-update thread waits on the main thread, which stalls the 2 s tick as well.
- **`set_config`** (sync):
  - `replace_config` calls `encode::resolve`, then `list_encoders`, then `mft::available_encoders()`, which runs MFTEnumEx on *every* change, including slider drags (state.rs:367).
  - It also writes the config, refreshes audio and reads the registry for autostart.
  - On a capture-source change it calls `stop_pipeline` plus `start_buffer_and_notify` (commands.rs:117-121). Stopping waits for a save in progress, which takes seconds.
  - `control::restart` waits on the server thread (see the first finding).
- **`file_clip`**: `filing::move_file` retries `rename` with sleeps of 40 to 200 ms, about 0.6 s in total (filing.rs:269-285). It does this *while holding the library lock* (commands.rs:747) and exactly when the player holds the file.
- **Others**:
  - `set_clip_dir`: create and write on what may be a network path.
  - The `list_audio_devices`, `list_audio_processes`, `list_capture_targets` and `list_encoders` commands: COM and MFT enumeration.
  - `copy_clip_file`, `clipboard_*`: up to about 0.5 s of retry sleeps (clipboard.rs:42-47).
  - `open_clip`: ShellExecute.
  - `delete_clip`: `remove_dir_all` plus full library scans.
  - `regenerate_control_token`.
- **Toggle-buffer hotkey**: lib.rs:811-816 calls `toggle_buffer_and_notify` inline. The global-shortcut 2.3.2 handler runs in the hidden window's wndproc on the main thread (lib.rs:416), so the pipeline starts and stops on the UI thread. The tray spawns a thread for the same action (tray.rs:113). The comment at lib.rs:786 ("hotkey arrives on one of its own") is wrong.
- **Startup**: `setup()` (lib.rs:1189-1230) runs `edit::repair`, `thumbs::migrate`, `normalize_games`, `filing::tidy` (renames with sleeps) and `discard_leftovers` on the main thread before the window paints. The code comments themselves mention about 3 s.
- **Fix**: mark these commands `#[tauri::command(async)]` or `spawn_blocking`. Spawn a thread in the toggle hotkey. Cache the encoder list. Do the setup tidying on a background thread.

### [Medium] [perf] `Library::get` reads the whole library to find one clip
- **Where**: clips.rs:186-188 `Ok(self.list()?.into_iter().find(|c| c.id == id))`. `list()` reads every row and every tag. For each clip it calls `edit::has_original`, which costs 2-3 `is_file` checks (clips.rs:145, edit.rs:73-79, :97).
- **Effect**: almost every command calls `get` one to three times, under the library mutex, mostly on the main thread. With 2,000 clips that is about 6,000 filesystem checks per click; Defender makes it worse.
- **Fix**: query with `WHERE id = ?1` plus the tags for that one id, and compute `original_available` only for the clip being returned.

### [Medium/Low] [bug] The UI starts from the mock config, and `set_config` saves whatever whole config it is sent
- **Where**: store.ts:164 has `config: mock.mockConfig`. The main UI is not gated on `ready`. `patchConfig` (store.ts:362-368) sends `{...get().config, ...patch}`. `set_config` (commands.rs:54-68) keeps the real token only when the incoming one is empty. The mock has `token: "mock-token"` and `clipDir: "C:\\Users\\user\\…"` (mock.ts:86, :121).
- **Scenario**: a settings change made before init's `Promise.all` finishes (startup is slow, see the main-thread finding) saves the mock values. Clip folder, audio sources and hotkeys are replaced. After a restart the control token becomes a string published in the repo. More generally, a stale settings page overwrites values the backend changed meanwhile (screen repair, a regenerated token).
- **Fix**: start the UI config as `null` and gate the UI on it. Better, make `set_config` accept a typed patch, or reject a config that does not match the current revision. Never accept `control.token` from the UI.

### [Low] [bug] Stream Deck "stop" (and "start") on `/v1/buffer` is undone by the automation — confirmed
- **Where**: control.rs:225-226 calls `start_buffer_and_notify` and `stop_buffer_and_notify` directly. Only the toggle path calls `state.auto.manual_stop()` and `manual_start()` (lib.rs:333-342).
- **Effect**: with "only in game" on, `poll()` (state.rs:184-191) sees `!suppressed` and restarts the buffer within 2 s.
- **Note**: the shipped plugin sends only "toggle", but "start"/"stop" are part of the API.
- **Fix**: call `manual_*` on those branches too.

### [Low/Medium] [bug] config.json writes can race and corrupt the file; a corrupt file is silently reset to defaults
- **Where**:
  - `replace_config` writes to disk *outside* the config lock (state.rs:368-402).
  - `control::ensure_token` and `regenerate_token`, `repair_screens` and `hide_to_tray` write *inside* it (control.rs:124, :364; lib.rs:505, :1139).
  - Both paths write the same `config.json.tmp` (config.rs:181-183), with no fsync.
- **Effect**: interleaved writes or a power loss can leave invalid JSON. `load()` then quietly falls back to defaults (config.rs:167-170). The next save overwrites the user's settings and a new token is generated. The comment at config.rs:244-248 itself calls this "the most expensive bug this file can have".
- **Fix**: one write lock around snapshot-and-write, a unique temp name, `sync_all()` before rename, and keep `config.json.bad` plus last-known-good on a parse failure.

### [Low] [bug] Database robustness
- **No recovery when the database cannot be opened**: if `Library::open` fails (state.rs:274-280), the library stays `None`. Saves still write files but are never indexed (lib.rs:123-127) and there is no rescan, so the clips are invisible for good. Fix: run an integrity check, rename the corrupt file aside, rebuild by scanning the clip folder.
- **Delete reports success while the file stays**: `Library::delete` ignores a `remove_file` error, then deletes the row (clips.rs:315-330). A file held by an external player becomes an untracked orphan while the UI says "deleted". Fix: fail or retry before deleting the row.
- **Latent row loss**: `path UNIQUE` combined with `INSERT OR REPLACE` (clips.rs:38, :100) silently deletes *another* clip's row and its tags whenever a new clip has the same path.

### [Low] [bug] `free_name` can overwrite an existing file
- **Where**: filing.rs:247-265 checks `exists()` and then renames. After 999 collisions it returns the existing `target`. `std::fs::rename` on Windows *replaces* the destination (filing.rs:272).
- **Race**: share.rs:643 picks a free name *before* a long download, so two parallel receives of the same name share both the `.part` file and the final target.
- **Fix**: create the target with `create_new(true)`, or use a rename that does not replace.

### [Low] [bug] Folder names from game names
No traversal is possible: `/ \ :` and control characters are replaced, and dots and spaces at the end are trimmed (filing.rs:41-72). Remaining gaps:
- The reserved-name list misses `CONIN$`, `CONOUT$`, `COM¹-³`/`LPT¹-³`, and a name like `NUL .x` with a space before the dot. These make folder creation fail, so the clip simply stays where it is.
- No Unicode normalization: NFC and NFD spellings of a name become two folders.
- Path comparisons are case-sensitive (filing.rs:101, :121). If a game is renamed with only a case change, `place` renames the file in place to `x (2).mp4`.
- `prune` removes only empty directories and does not follow junctions. It is safe.

### [Low] [security/bug] `clipboard::read_text` can read past the end of the clipboard block
- **Where**: clipboard.rs:163-175 reads up to 4,000,000 UTF-16 units until it finds a null, without checking `GlobalSize`.
- **Effect**: a local app that puts unterminated `CF_UNICODETEXT` on the clipboard can cause an out-of-bounds read, an access violation and a crash.
- **Fix**: bound the read with `GlobalSize(block) / 2`.

### [Low] [security hardening] ffmpeg parses friend-supplied media automatically
- **Where**: `thumbs::make` / `make_still` (thumbs.rs:28-62) run ffmpeg with no `-protocol_whitelist` and no fixed `-f` over files received from friends, right after they arrive (share.rs:838-841).
- **Risk**: ffmpeg's demuxers are the largest parser attack surface reachable from the network. Exploitability is speculative.
- **Fix**: add `-protocol_whitelist file` and `-f mov`/`image2` matched to the expected kind, and keep the bundled ffmpeg current.

### [Low] [security] Control port hardening
- On by default even without a Stream Deck. Any web page can detect that ClippiBoy is running by probing the fixed port.
- The Host header is unchecked. Against DNS rebinding the Origin, Referer and Sec-Fetch-Site heuristic has gaps: a same-origin GET sent with `no-referrer` carries none of them. Today the token is the real protection.
- The token never changes across runs. Another local user who binds the port while ClippiBoy is down can collect the token from the plugin's polling and later trigger recordings or screenshots of the victim's session.
- Port 0 can be configured; the handoff file then names port 0 instead of the bound one.
- **Fix**: off until the plugin is detected, require `Host: 127.0.0.1:<port>` or `localhost:<port>`, rotate the token each run, write the actual bound port.

### [Low] [privacy] Logging
- No size limit within a run (logging.rs:59-63 rotates only at start). The 5 s health line plus networking warnings grow for weeks while the app sits in the tray.
- friends.rs:521 logs the *entire* server message when parsing fails (presence data, friend IDs).
- share.rs:694 and :747 log Debug-formatted iroh errors, which can include peer IP addresses — and testers are asked to send these logs back.
- No tokens or OAuth codes are logged.
- **Fix**: size cap or daily rotation, truncate or redact unknown messages, keep Debug detail out of info-level logs.

### [Low] [security] Installer and uninstall leave data behind
- hooks.nsh itself is benign: it takes no user input, deletes nothing and runs no commands.
- There is no uninstall hook. Tauri's "delete app data" removes `%APPDATA%\gg.clippiboy.app` and `%LOCALAPPDATA%\gg.clippiboy.app`, but the real data is in `%APPDATA%\ClippiBoy` and survives: friends.json (session bearer token), the control token, clips.db, gigabytes of originals and tracks, logs. The HKCU Run autostart value also stays.
- All of this is in the *Roaming* profile.
- **Fix**: add `NSIS_HOOK_POSTUNINSTALL` behind the same checkbox, remove the autostart entry, and move the large stores to LocalAppData.

### [Low] [config] Asset scope and permissions
- `$APPDATA/ClippiBoy/**` resolves to `%APPDATA%\gg.clippiboy.app\ClippiBoy`, which does not exist (tauri path/desktop.rs:247). The entry is dead. Do not "fix" it to `$DATA`, which would expose config.json and friends.json.
- `$VIDEO/**` exposes the whole Videos folder; `$VIDEO/ClippiBoy/**` is enough.
- `opener:default`, `opener:allow-reveal-item-in-dir` and `global-shortcut:default` are not used by the frontend (it imports no opener or global-shortcut module). Remove them.
- CSP: `script-src` falls back to `'self'`, which is good. `style-src 'unsafe-inline'` is acceptable. Add `base-uri 'none'; form-action 'none'; object-src 'none'`.
- `msSmartScreenProtection` is disabled in additionalBrowserArgs. This is informational because only local content is loaded.

### [Low/Info] [security] Deep links
- PKCE is sound: the server checks sha256(verifier) against the stored challenge (server/src/index.ts:222), so login CSRF is not possible.
- The match is loose (`strip_prefix("clippiboy://auth")` also matches `clippiboy://authX`).
- Any link `take()`s the pending verifier *before* the exchange (friends.rs:289). A web page firing a bogus link during a sign-in cancels it, and each link raises an error toast.
- Second-instance arguments only bring the window up.
- **Fix**: parse with `url::Url`, require host `auth`, consume the verifier only after a successful exchange, and rate-limit the toasts.

### [Low] [bug]
- `export_for_discord`: `megabytes * 1024 * 1024` is unchecked (commands.rs:872). Release builds wrap; debug builds panic. Use `checked_mul` and clamp.
- `save_clip(Some(seconds))` (commands.rs:375-382) skips `begin_save`, tagging, the `clip-saved` event and the banner. The UI never sends `Some`. Remove it or route it through `save_clip_and_notify`.
- `.expect("could not start ClippiBoy")` (lib.rs:1367) aborts without telling the user (for example when WebView2 is missing). Show a native message box first.

### [Quality] commands.rs and lib.rs
- `type Result<T> = Result<T, String>`: 39 `.map_err(|e| e.to_string())` and 21 `"clip not found"` in commands.rs. `thiserror` is already a dependency, so a serializable `CmdError` enum fits.
- Add a `clip_by_id(&state, &id)` helper.
- The three `*_and_notify` functions in lib.rs:105-271 repeat the same index, tag, emit, notify, sound, telemetry and banner sequence. Extract an `announce_saved`.
- `spawn_ui_updates` (lib.rs:928-1089) is a 160-line loop driven by magic `tick %` values; per-cadence scheduled jobs would be clearer.
- Mixed German and English in comments, logs and capability descriptions (lib.rs:853-857, :931-958, commands.rs:762 "blieb liegen", `.clippiboy-schreibtest`, default.json "Rechte des Hauptfensters").
- Suggested split:
  - lib.rs: `actions.rs` (save/shot/record/buffer flows), `hotkeys.rs`, `ticker.rs`, `watchers.rs`, `app.rs` (builder).
  - commands.rs: `commands/{config, clips, editor, screenshot, export, system}.rs`.
- Lock poisoning is a non-issue: parking_lot does not poison, and its `!Send` guards make holding a lock across `.await` a compile error. No mutex is held across IO except the library lock in `file_clip` and `with_library`.

## Hardening recommendations (not tied to one bug)
1. Define an app ACL manifest and give each window only the commands it uses.
2. Treat every IPC path argument as untrusted. Paths come from Rust-side dialogs or are derived from clip ids; never send raw paths from JS.
3. Rebuild the control port with timeouts, size and connection limits, a Host check and per-run tokens, and turn it on only when the plugin exists.
4. Add a `-protocol_whitelist file` and an explicit `-f` to every ffmpeg call, and drop the PATH fallback for `ffmpeg` in `muxer::tool`.
5. Move sync commands off the main thread by default; keep main-thread work for window operations.
6. Make writes crash-safe: config, handoff, friends.json and links.json each get a unique temp file, fsync and rename, plus a backup on parse failure.
7. Validate the clip folder: canonicalize it and refuse an empty path, a drive root, the profile root, the data folder itself, `\\` UNC paths and `\\?\` device paths.
8. Move the large caches (originals, tracks, thumbs, preview, logs) to LocalAppData. Consider DPAPI for friends.json and the control token.
9. Add tests for: `delete_clip("")`, a mock config reaching `set_config`, reserved names and case handling in `folder_name`, and the control port's handling of oversized requests.