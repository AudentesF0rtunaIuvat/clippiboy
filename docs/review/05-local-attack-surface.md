# Local attack surface and command layer

**Scope:** commands.rs, lib.rs, control.rs (Stream Deck port), config.rs, clips.rs, filing.rs, clipboard.rs, thumbs.rs, logging.rs, tauri.conf.json, capabilities, NSIS hooks.

**Findings:** 41 — 29 verified (0 medium or higher), 11 reported but not independently re-verified, 0 unverifiable/disputed, 1 rejected by verification.

Severity shown is the verified severity; where verification changed it, the reported one is in parentheses. See [README](README.md#how-findings-were-verified) for how verification worked.

## Reported, not independently re-verified

The original reviewer checked these against the code, but the independent verification pass was stopped before reaching them. Treat as likely, confirm before fixing.

- **LS-1 · Medium · Local process can crash ClippiBoy or hang its UI through the control port without the token (tiny_http body drain)** — `src-tauri/src/control.rs:156-158`, `src-tauri/src/control.rs:325-336`, `src-tauri/src/control.rs:94-106` (+2 more)  
  The control port (control.rs, tiny_http 0.12.0, bound to 127.0.0.1:47653, on by default) answers requests that fail the token/header check with a 403 on its single serving thread: control.rs:156-158 `if !allowed { return respond(request, 403, …) }`, with `respond` at control.rs:325-336. In tiny_http 0.12.0, request.rs:191-216 wraps any body larger than 1024 bytes in `EqualReader`, and `util/equal_reader.rs` has a `Drop` that does `let mut buf = vec![0; remaining_to_read];` and then blocks reading the rest of the body from the socket. No socket timeouts are set anywhere, and `util/task_pool.rs…  
  *Fix:* Replace tiny_http with a small server you control (std `TcpListener`, or tokio which is already a dependency) with a read timeout of about 2 s, at most 8 KB of headers, rejection of `Content-Length` above 4 KB before building the request, closing the socket rather than draining it, and a cap of about 8 connections; or patch tiny_http. Separately m…
- **LS-11 · Medium · Startup tidying in `setup()` runs on the main thread before the window paints** — `src-tauri/src/lib.rs:1189-1230`  
  `setup()` (lib.rs:1189-1230) runs `edit::repair`, `thumbs::migrate`, `normalize_games`, `filing::tidy` (renames with sleeps) and `discard_leftovers` on the main thread before the window paints. The code comments themselves mention about 3 s.  
  *Fix:* Do the setup tidying on a background thread.
- **LS-12 · Medium · `Library::get` reads the whole library (and does filesystem checks per clip) to find one clip** — `src-tauri/src/clips.rs:186-188`, `src-tauri/src/clips.rs:145`, `src-tauri/src/edit.rs:73-79` (+1 more)  
  clips.rs:186-188 is `Ok(self.list()?.into_iter().find(\|c\| c.id == id))`. `list()` reads every row and every tag, and for each clip calls `edit::has_original`, which costs 2-3 `is_file` checks (clips.rs:145, edit.rs:73-79, :97). Almost every command calls `get` one to three times, under the library mutex, mostly on the main thread.  
  *Fix:* Query with `WHERE id = ?1` plus the tags for that one id, and compute `original_available` only for the clip being returned.
- **LS-15 · Medium · config.json writes can race and corrupt the file; a corrupt file silently resets to defaults** — `src-tauri/src/state.rs:368-402`, `src-tauri/src/control.rs:124`, `src-tauri/src/control.rs:364` (+6 more)  
  `replace_config` writes to disk *outside* the config lock (state.rs:368-402), while `control::ensure_token` and `regenerate_token`, `repair_screens` and `hide_to_tray` write *inside* it (control.rs:124, :364; lib.rs:505, :1139). Both paths write the same `config.json.tmp` (config.rs:181-183), with no fsync. On a parse failure `load()` quietly falls back to defaults (config.rs:167-170). The comment at config.rs:244-248 itself calls this "the most expensive bug this file can have".  
  *Fix:* One write lock around snapshot-and-write, a unique temp file name, `sync_all()` before rename, and keep `config.json.bad` plus a last-known-good copy on a parse failure.
- **LS-2 · Medium · Capabilities give false least privilege: every window can call all 79 app commands (no app ACL manifest)** — `src-tauri/build.rs`, `src-tauri/gen/schemas/acl-manifests.json`, `src-tauri/capabilities/overlay.json` (+2 more)  
  build.rs is plain `tauri_build::build()`: there is no `AppManifest`, and gen/schemas/acl-manifests.json has no `__app__` entry. Tauri 2.11.5 webview/mod.rs:1823 checks the ACL only `if (plugin_command.is_some() \|\| has_app_acl_manifest \|\| !is_local)`, so capabilities gate only plugin commands and every window (main, overlay, console) can call all 79 app commands. overlay.json describes itself as "only listens, controls nothing", yet the overlay can call `delete_clip`, `set_config`, `clipboard_read_text`, `export_clip`, `friends_delete_account` and so on.  
  *Fix:* Use `tauri_build::Attributes::new().app_manifest(AppManifest::new().commands(&[...]))` and grant commands per window. The overlay needs roughly nothing; the console needs only its clip and friends subset.
- **LS-3 · Medium · `export_clip` writes to any renderer-supplied output path (overwrite, UNC NTLM leak, ffmpeg http upload)** — `src-tauri/src/commands.rs:1155-1169`, `src-tauri/src/commands.rs:1094-1097`, `src-tauri/src/export.rs:124`  
  `export_clip` (commands.rs:1155-1169) takes `output: String` straight from the UI. `shrink` (commands.rs:1094-1097) only refuses `output == source`. ffmpeg then runs with `-y` (export.rs:124) and the output as its last argument, with no restriction to local file paths or protocols. Part of the finding that a compromised renderer gets user-level read and write on the whole filesystem (defense in depth).  
  *Fix:* Show the save dialog from Rust and keep the chosen path server-side, or at least require an absolute local drive path: no `\\`, no `scheme:`, an allowed extension. Add `-protocol_whitelist file` to export, trim and thumbnail ffmpeg calls.
- **LS-4 · Medium · `set_clip_dir` / `set_config` accept any clip folder and grant it recursively to the asset protocol** — `src-tauri/src/commands.rs:266-290`, `src-tauri/src/commands.rs:54-98`, `src-tauri/src/lib.rs:1095`  
  `set_clip_dir` (commands.rs:266-290) and `set_config` (commands.rs:54-98) accept any directory, and `allow_clip_dir` grants it *recursively* to the asset protocol (lib.rs:1095). `set_config` does not validate `clip_dir` at all. Speculative: with an empty `clip_dir`, Tauri builds the pattern `\**` (scope/fs.rs:489-498), which may match every canonicalized path.  
  *Fix:* Show the folder dialog from Rust and keep the chosen path server-side; canonicalize and validate `clip_dir` in `set_config`; refuse empty, system or profile-root folders.
- **LS-5 · Medium · `reveal_path` accepts any path, UNC included** — `src-tauri/src/commands.rs:1347`  
  `reveal_path(path)` (commands.rs:1347) accepts any path from the renderer, UNC paths included, with no check that it lies inside the app's known folders. Part of the finding that a compromised renderer gets user-level filesystem access (defense in depth).  
  *Fix:* Make `reveal_path` accept only paths inside the known folders.
- **LS-6 · Medium · `delete_clip` with an empty or colliding id wipes every clip's audio tracks and untouched originals** — `src-tauri/src/commands.rs:680-683`, `src-tauri/src/muxer.rs:470`, `src-tauri/src/stems.rs:44-46` (+5 more)  
  commands.rs:680-683 runs `stems::remove(&id); edit::remove(&id); crate::thumbs::remove(&id);` *before* checking the clip exists. `muxer::sanitize` (muxer.rs:470) maps "" to "", so `stems::dir("")` is `tracks\` and `remove_dir_all` deletes the whole store (stems.rs:44-46, :146-148). `edit::remove("")` does `remove_dir_all(originals\)` (edit.rs:61-63, :114-118), which holds every trimmed clip's only untouched copy and every screenshot original. The library delete is then a no-op and the call returns `Ok`. Because `sanitize` maps `-` to `_`, passing another clip's UUID with underscores destroys…  
  *Fix:* Look up the clip first and return `Err` if it is missing. Take the store paths from `clip.id` as stored. Make `sanitize` return an error for empty or non-UUID input. Commit the row delete before removing files.
- **LS-7 · Medium · `set_config` runs heavy work on the main thread, including MFTEnumEx on every change** — `src-tauri/src/state.rs:367`, `src-tauri/src/commands.rs:117-121`, `src-tauri/src/encode.rs:164-171`  
  Sync `#[tauri::command]` functions run on the event-loop thread; while one runs the UI does not paint, and `window_awake` in the UI-update thread waits on the main thread, stalling the 2 s tick too. `set_config` is sync: `replace_config` calls `encode::resolve`, then `list_encoders`, then `mft::available_encoders()`, which runs MFTEnumEx on *every* change, including slider drags (state.rs:367). It also writes the config, refreshes audio and reads the registry for autostart. On a capture-source change it calls `stop_pipeline` plus `start_buffer_and_notify` (commands.rs:117-121); stopping waits…  
  *Fix:* Mark the command `#[tauri::command(async)]` or use `spawn_blocking`; cache the encoder list.
- **LS-8 · Medium · `file_clip` sleeps in rename retries on the main thread while holding the library lock** — `src-tauri/src/filing.rs:269-285`, `src-tauri/src/commands.rs:747`  
  `file_clip` (a sync command, main thread) calls `filing::move_file`, which retries `rename` with sleeps of 40 to 200 ms, about 0.6 s in total (filing.rs:269-285). It does this *while holding the library lock* (commands.rs:747), and the retries happen exactly when the player holds the file.  
  *Fix:* Mark the command `#[tauri::command(async)]` or use `spawn_blocking`.

## Verified lower-severity issues

| ID | Severity | Title | Where | Fix |
|---|---|---|---|---|
| LS-16 | Low | No recovery when the clip database cannot be opened; new clips are never indexed | src-tauri/src/state.rs:274-280, src-tauri/src/lib.rs:123-127 | Run an integrity check, rename the corrupt file aside, and rebuild by scanning the clip folder. |
| LS-17 | Low | `Library::delete` reports success while the file stays on disk | src-tauri/src/clips.rs:315-330, src-tauri/src/clips.rs | Fail or retry the file removal before deleting the row. |
| LS-22 | Low | Case-sensitive path comparisons in filing cause spurious `x (2).mp4` renames | src-tauri/src/filing.rs:101, src-tauri/src/filing.rs:121 | Not stated in the report (implied: compare paths case-insensitively on Windows). |
| LS-27 | Low | Control token never rotates; another local user can squat the port and harvest it | src-tauri/src/control.rs | Rotate the token each run. |
| LS-29 | Low | Log file has no size limit within a run | src-tauri/src/logging.rs:59-63 | Add a size cap or daily rotation. |
| LS-30 | Low | Unparseable friends-server messages are logged in full (presence data, friend IDs) | src-tauri/src/friends.rs:521 | Truncate or redact unknown messages before logging. |
| LS-32 | Low | Uninstall leaves session token, control token, database, large caches and autostart entry behind | src-tauri/windows/hooks.nsh | Add `NSIS_HOOK_POSTUNINSTALL` behind the same checkbox, remove the autostart entry, and move the large stores to LocalAppData. |
| LS-48 | Low | Missing tests for delete_clip(""), mock-config saves, folder-name edge cases and oversized control-port requests | src-tauri/src/commands.rs, src/store.ts | Add tests for these four cases. |
| LS-18 | Info (reported Low) | `path UNIQUE` with `INSERT OR REPLACE` silently deletes another clip's row and tags | src-tauri/src/clips.rs:38, src-tauri/src/clips.rs:100 | Not stated in the report (implied: avoid INSERT OR REPLACE deleting other clips' rows on a path conflict). |
| LS-20 | Info (reported Low) | Game folder name sanitizer misses some Windows reserved device names | src-tauri/src/filing.rs:41-72 | Not stated in the report (implied: extend the reserved-name handling to these names/patterns). |
| LS-21 | Info (reported Low) | No Unicode normalization of game folder names | src-tauri/src/filing.rs:41-72 | Not stated in the report (implied: Unicode-normalize game names before deriving folder names). |
| LS-23 | Info (reported Low) | `clipboard::read_text` can read past the end of the clipboard block | src-tauri/src/clipboard.rs:163-175, clipboard.rs:158-175 | Bound the read with `GlobalSize(block) / 2`. |
| LS-25 | Info (reported Low) | Control port is on by default and lets any web page detect ClippiBoy | src-tauri/src/control.rs | Keep the control port off until the Stream Deck plugin is detected. |
| LS-26 | Info (reported Low) | Control port does not check the Host header; Origin/Referer/Sec-Fetch-Site heuristic has DNS-rebinding gaps | src-tauri/src/control.rs | Require `Host: 127.0.0.1:<port>` or `localhost:<port>`. |
| LS-28 | Info (reported Low) | Configured port 0 makes the handoff file name port 0 instead of the bound port | src-tauri/src/control.rs | Write the actual bound port to the handoff file. |
| LS-33 | Info (reported Low) | Dead asset-protocol scope entry `$APPDATA/ClippiBoy/**` | src-tauri/tauri.conf.json | Not stated explicitly (implied: remove the dead entry); do not change it to `$DATA`. |
| LS-34 | Info (reported Low) | Asset scope `$VIDEO/**` exposes the whole Videos folder | src-tauri/tauri.conf.json | Narrow the scope to `$VIDEO/ClippiBoy/**`. |
| LS-35 | Info (reported Low) | Unused opener and global-shortcut plugin permissions granted | src-tauri/capabilities/default.json, src-tauri/capabilities/default.json:16 | Remove these permissions. |
| LS-36 | Info (reported Low) | CSP lacks `base-uri`, `form-action` and `object-src` restrictions | src-tauri/tauri.conf.json | Add `base-uri 'none'; form-action 'none'; object-src 'none'`. |
| LS-37 | Info | `msSmartScreenProtection` disabled in WebView2 browser args | src-tauri/tauri.conf.json | None required (informational). |
| LS-38 | Info (reported Low) | Deep-link match is loose: `clippiboy://authX` is accepted | src-tauri/src/friends.rs:147 | Parse with `url::Url` and require host `auth`. |
| LS-40 | Info (reported Low) | `export_for_discord` size computation `megabytes * 1024 * 1024` is unchecked | src-tauri/src/commands.rs:872 | Use `checked_mul` and clamp. |
| LS-41 | Info (reported Low) | `save_clip(Some(seconds))` bypasses the normal save flow | src-tauri/src/commands.rs:375-382 | Remove the `Some` path or route it through `save_clip_and_notify`. |
| LS-42 | Info (reported Low) | Startup failure `.expect("could not start ClippiBoy")` aborts without telling the user | src-tauri/src/lib.rs:1367 | Show a native message box before exiting. |
| LS-43 | Info (reported Low) | Stringly-typed command errors: repeated `.map_err(\|e\| e.to_string())` and "clip not found" | src-tauri/src/commands.rs | Introduce a serializable `CmdError` enum using `thiserror`. |
| LS-44 | Info (reported Low) | Repeated clip lookup logic in commands.rs; no `clip_by_id` helper | src-tauri/src/commands.rs | Add a `clip_by_id(&state, &id)` helper. |
| LS-45 | Info (reported Low) | Three `*_and_notify` functions duplicate the same post-save sequence | src-tauri/src/lib.rs:105-271 | Extract an `announce_saved` function. |
| LS-46 | Info (reported Low) | `spawn_ui_updates` is a 160-line loop driven by magic `tick %` values | src-tauri/src/lib.rs:928-1089 | Use per-cadence scheduled jobs. |
| LS-47 | Info (reported Low) | Mixed German and English in comments, logs and capability descriptions | src-tauri/src/lib.rs:853-857, src-tauri/src/lib.rs:931-958 | Not stated in the report (implied: use one language consistently). |

## Claims rejected by verification

| ID | Claim | Why it was rejected |
|---|---|---|
| LS-31 | Debug-formatted iroh errors in logs can leak peer IP addresses | The lines do log Debug-formatted errors: src-tauri/src/share.rs:694 `log::info!("share: attempt {} for {id}: {err}", ...)` and :747 log a String built by `net()` (share.rs:1171-1173 `format!("{err:?}")`), from fetch() (:783-821). None of the error types reachable there carry a peer IP address. iroh… |

## What is solid

- No remote code execution was found.
- No SQL injection: every query uses bound parameters.
- No path traversal through game names or names of files received from friends.
- Remote origins are blocked from Tauri IPC by Tauri 2.11 (webview/mod.rs:1823); IPC is reachable only from local content in the main, overlay and console windows.
- Control port refuses (403) requests without a matching `x-clippiboy-token` (UUIDv4) or carrying Origin, Referer or Sec-Fetch-Site, and sends no CORS headers; the token is the real protection today.
- The handoff file `%APPDATA%\ClippiBoy\streamdeck.json` and the token in config.json are readable by the same user only.
- Single-instance arguments: only `--autostart` is examined; deep links are forwarded by the plugin; second-instance arguments only bring the window up.
- Installer post-install writes the installer file name to install-source.txt, which telemetry.rs:173 parses strictly.
- hooks.nsh is benign: it takes no user input, deletes nothing and runs no commands; pre-install waits for the exe to be released.
- React escaping and the absence of `dangerouslySetInnerHTML` make XSS unlikely today.
- Game folder names: `/ \ :` and control characters are replaced and trailing dots and spaces trimmed (filing.rs:41-72), so no traversal is possible.
- `prune` removes only empty directories and does not follow junctions; it is safe.
- No tokens or OAuth codes are logged.
- CSP: `script-src` falls back to 'self' (good); `style-src 'unsafe-inline'` is acceptable.
- Deep-link PKCE is sound: the server checks sha256(verifier) against the stored challenge (server/src/index.ts:222), so login CSRF is not possible.
- Lock poisoning is a non-issue: parking_lot does not poison, and its `!Send` guards make holding a lock across `.await` a compile error; no mutex is held across IO except the library lock in `file_clip` and `with_library`.
- Browsers cannot set Content-Length, so the control-port body-drain crash is local-only.

## Improvement opportunities and missing features

| ID | Idea | Why | Effort | Sketch |
|---|---|---|---|---|
| LS-P-1 | Define an app ACL manifest and per-window command grants | Restores least privilege so an XSS in the overlay or console cannot call destructive commands. | ? | build.rs: `tauri_build::Attributes::new().app_manifest(AppManifest::new().commands(&[...]))`; capabilities/*.json grant each window only the commands it uses (overlay roughly none, console its clip and friends subset). |
| LS-P-2 | Treat every IPC path argument as untrusted | Prevents a compromised renderer from reading or writing arbitrary files. | ? | Paths come from Rust-side dialogs or are derived from clip ids; never send raw paths from JS (export_clip, set_clip_dir/set_config clip_dir, reveal_path). |
| LS-P-3 | Rebuild the control port with limits, Host check and per-run tokens, enabled only when the plugin e… | Removes the local DoS, fingerprinting, DNS-rebinding gaps and token harvesting. | ? | control.rs: timeouts, size and connection limits, Host check, per-run tokens; turn the port on only when the Stream Deck plugin is detected. |
| LS-P-4 | Harden every ffmpeg invocation | Shrinks the parser and protocol attack surface reachable from friend-supplied files and renderer arguments. | ? | Add `-protocol_whitelist file` and an explicit `-f` to every ffmpeg call (export, trim, thumbs), and drop the PATH fallback for `ffmpeg` in `muxer::tool`. |
| LS-P-5 | Move sync commands off the main thread by default | Keeps the UI responsive; main thread reserved for window operations. | ? | Make commands `#[tauri::command(async)]` or use `spawn_blocking` by default; keep main-thread work for window operations. |
| LS-P-6 | Crash-safe writes for all persisted JSON files | Prevents silent settings and token loss on races or power loss. | ? | config, handoff (streamdeck.json), friends.json and links.json each get a unique temp file, fsync and rename, plus a backup on parse failure. |
| LS-P-7 | Validate the clip folder | Stops the asset protocol from being granted over sensitive directories. | ? | Canonicalize the clip folder and refuse an empty path, a drive root, the profile root, the data folder itself, `\\` UNC paths and `\\?\` device paths. |
| LS-P-8 | Move large caches to LocalAppData and protect secrets with DPAPI | Keeps gigabytes of caches out of the roaming profile and protects the session and control tokens at rest. | ? | Move originals, tracks, thumbs, preview and logs to LocalAppData; consider DPAPI for friends.json and the control token. |
| LS-P-9 | Split lib.rs and commands.rs into focused modules | Makes the two largest files easier to navigate and change. | ? | lib.rs: `actions.rs` (save/shot/record/buffer flows), `hotkeys.rs`, `ticker.rs`, `watchers.rs`, `app.rs` (builder). commands.rs: `commands/{config, clips, editor, screenshot, export, system}.rs`. |
