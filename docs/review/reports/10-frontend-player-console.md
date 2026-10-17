ClippiBoy review: clip player/editor, sharing UI, setup, console window, overlay window (read-only)

I did not modify any files. Where a finding depends on Tauri, tao or wry behaviour, I checked the vendored sources (tauri 2.11.5, tauri-runtime-wry 2.11.4, tao 0.35.3, wry 0.55.1).

## How the pieces fit

- **ClipPlayer.tsx**
  - A portal over the gallery or the dashboard. It is given `clips[]` plus an `index`, so it locates its clip by position, not by id.
  - It owns the video, the trim state, the save, restore and discard flows, fullscreen, keyboard shortcuts and the open/close animation.
  - While playing, a requestAnimationFrame loop writes the scrubber and clock directly into the DOM and enforces the trim loop.
  - **ClipEditor** (beside it) shows ClipMeta (name, description, game, tags, auto-saved), the per-track gain/mute/solo controls and the trim marks.
  - **useClipMix** plays the extracted audio stems as hidden `<audio>` elements through a WebAudio graph (gain plus hard clip), mutes the video's own audio, and keeps the stems in sync on play, pause, seek and drift.
  - Save calls `applyClipEdit`. The core rewrites the file, the player re-keys the `<video>` element, and a SaveVeil shows a frozen frame while that happens.
- **PlayerControls.tsx** holds the PlayButton, Scrubber (with trim handles) and Volume controls. Both the app player and the console's ClipStage use them.
- **clipMenu / ExportDialog / Share / Links:**
  - The right-click menu is shared between the gallery and the player.
  - "Export a copy" asks for a target size and then calls `export_clip`.
  - Friend P2P transfers and share links are mirrored from Rust events into zustand stores (`share.ts`, `links.ts`, `friends.ts`) in every window.
- **Setup.tsx:** first-run steps (welcome, "where did you hear", screen, audio, hotkeys, ready). Every step writes config immediately; Finish starts the buffer.
- **Console** (console.html, `console.rs`):
  - A full-screen, transparent, always-on-top window, created once at startup and only shown or hidden afterwards.
  - It runs with `--disable-gpu`, renderer backgrounding turned off, and a data folder separate from the main window's.
  - It contains a dock (status, Clip, Record, Screenshot), panels (recent clips, audio levels, performance trend, friends), friend offers and a ClipStage mini-player. It takes focus while open.
  - Escape or a click beside the dock plays the exit animation and then calls `close_console`, which hands the foreground back to the game.
- **Overlay** (overlay.html, `overlay.rs`):
  - A click-through window shown without activation, for banners and the REC badge. It only listens to `overlay-banner`, `engine-status` and `overlay-place`.
  - Rust shows it, places it, coalesces banners with a sequence number, and hides it again.

## Findings, most severe first

### 1. [High] [perf] The hidden console keeps a CSS animation running for the whole session, over the game — likely
- **Where:**
  - Console.tsx:1025 renders `<LiveDot />` in the dock whenever the buffer is running.
  - motion.css:132 `.cb-live-ring { animation: cb-live-ring 2s var(--ease-out-soft) infinite; }`
  - console.css:273 `.cb-ui[data-leaving="true"] { animation: cb-fade-out 160ms ease-in both; pointer-events: none; }`
- **What happens:**
  - When the console closes, the dock is only faded to opacity 0. It stays mounted, so the infinite ring animation keeps running.
  - The window is hidden with `ShowWindow(SW_HIDE)`. Tauri's hide never calls the WebView2 `SetIsVisible(false)` (only `webview.hide()` would).
  - The console also runs with `--disable-renderer-backgrounding --disable-backgrounding-occluded-windows` and `CalculateNativeWinOcclusion` turned off, so nothing throttles it.
  - Result: Chromium keeps producing compositor frames at about 60 fps for days, rasterised and composited on the CPU because of `--disable-gpu`, while the user plays.
  - The `.cb-glow` layer (`will-change: opacity`) also stays alive as a full-screen layer.
- **Related:** the main window has the same flags, so any infinite animation it shows (for example LiveDot) also runs while it sits in the tray.
- **Fix:**
  - After the exit animation (`finishClose` or `console-closed`), render an empty page instead of an invisible dock. A `visible` flag that unmounts `.cb-ui` and the glow fits the authors' goal of "a blank page for next time".
  - Alternatively, at least add `.cb-ui[data-leaving="true"] * { animation-play-state: paused }` and drop `will-change` when hidden.
  - Optionally call `webview.hide()` from Rust after `window.hide()`, and `webview.show()` before showing.

### 2. [High] [bug] Dashboard player follows its clip by position: a newly saved clip silently swaps the clip and discards unsaved edits — confirmed
- **Where:**
  - Dashboard.tsx:80 `const recent = useMemo(() => clips.filter((clip) => !clip.screenshot && !clip.recording).slice(0, 3), [clips]);`
  - Dashboard.tsx:272 `<ClipPlayer clips={recent} index={Math.min(playing, recent.length - 1)} ...`
  - ClipPlayer.tsx:86 `const clip = clips[index];`
- **What happens:**
  - A user is trimming the newest clip from the dashboard and presses the save hotkey, or a recording finishes, or a friend's clip arrives.
  - `onClipSaved` prepends the new clip, so `recent[playing]` is now a different clip.
  - The player's reset effect (ClipPlayer.tsx:287) clears the trim, and `useClipMix` drops the mix.
  - Deletes made from the console shift the list the same way.
- Clips.tsx already solves this with an id playlist plus `shownId`. The dashboard player also never files favorites on close; the gallery does that in `closePlayer`.
- **Fix:** track the clip by id. Pass a playlist snapshot of ids, or reuse the Clips.tsx playlist logic through a shared hook.

### 3. [High] [bug] The console's "Shrink to 10/25 MB and copy" can copy a stale or half-written file — confirmed logic
- **Where:**
  - commands.rs:876 `let ready = std::fs::metadata(&output).map(|meta| meta.len() > 0 && meta.len() <= target_bytes).unwrap_or(false);`
  - `shrink()` runs ffmpeg directly into `output`, with no temporary file and rename.
  - Console.tsx:498 keeps a single busy slot: `setBusy(what)`.
- **Scenarios:**
  - **Stale:** the user exports at 10 MB, trims the clip later, and exports at 10 MB again. The old untrimmed "(10 MB).mp4" is reused and pasted into Discord.
  - **Partial:** the user clicks 10 MB, then "Clip". `busy` switches to "clip", so the 10 MB button is enabled again. A second click sees the first run's partially written file (non-empty, under the limit) and puts a truncated MP4 on the clipboard.
  - **Crash:** after a crash or quit mid-export, the partial file stays and is reused forever.
- **Fix:**
  - Write to `*.part` and rename on success.
  - Treat the cache as valid only if its mtime is later than the source's (or key it on the source size and mtime).
  - In the console, keep busy state per job (a `Set<string>`) and show the `clip-progress` value.

### 4. [Medium-High] [bug] A running Save/Undo-trim does not block paging, closing or deleting — confirmed (UI); speculative (Rust races)
- **Where:**
  - ClipPlayer.tsx:547-582 `save()` (`resumeAt`, `veil.freeze`, `await applyClipEdit`, then in `finally` `setReload((n)=>n+1)` and `setJustSaved(true)`).
  - The keyboard handler (`n`/`p` → `step`, Escape → `close`), the Step buttons, Delete and the close button stay enabled.
  - Line 253 resets `justSaved` only when the clip changes, before the save finishes.
- **What happens:** the user saves clip A (a re-encode can take minutes) and presses `n`.
  - Clip B plays underneath while A's frozen frame and progress cover the stage.
  - When A finishes, `setJustSaved(true)` makes B claim "Saved. That is how the clip sits in the folder now".
  - `setReload` re-creates B's `<video>` element, so B restarts.
  - The editor shows "Saving…" for B, and the progress bar stalls because the listener now filters on B's id.
  - Closing during a save calls Clips.closePlayer, which runs `fileClip` while the edit job may be between ffmpeg and `swap_in`.
  - Deleting during a save runs `delete_clip` while `apply` runs. The core's render lock does not cover filing or deletion.
- **Fix:** while `saving || restoring`, disable step, close, delete and the shortcuts. Alternatively, key `saving` and `justSaved` by clip id and ignore results for a clip that is no longer shown.

### 5. [Medium] [security] The console and overlay capabilities do not restrict the app's custom commands, and both windows may emit events — confirmed
- **Where:**
  - build.rs is a plain `tauri_build::build()`, so there is no app ACL manifest.
  - tauri webview/mod.rs:1823 only checks the ACL for app commands `if (plugin_command.is_some() || has_app_acl_manifest || !is_local)`.
  - overlay.json: `"permissions": ["core:event:default"]`, with the description "only listens, controls nothing".
- **Effect:**
  - Every command in `generate_handler!` can be called from the overlay and the console, including `delete_clip`, `set_config`, `set_clip_dir`, `reveal_path`, `install_update` and `friends_delete_account`.
  - `core:event:default` includes `allow-emit` and `allow-emit-to`, so the overlay can send `share-state`, `focus-clip`, `update-available` or `main-hidden` to the main window.
  - The console capability grants `core:window:allow-hide/show/set-focus`, which its JavaScript never uses, and `core:default` (tray, menu and more).
  - This is defense in depth only. I found no XSS sink (below), and the CSP blocks inline scripts.
- **Fix:**
  - Switch to `tauri_build::try_build(Attributes::new().app_manifest(AppManifest::new().commands(&[...])))` and add `permissions/*.toml` with per-window sets: none for the overlay; for the console only `list_clips`, `engine_status`, `get_config`, `save_clip`, `toggle_recording`, `take_screenshot`, `close_console`, `show_clip_in_app`, `export_for_discord`, `copy_clip_file`, `set_clip_favorite`, `file_clip`, `update_clip`, `delete_clip`, `link_exists`, `update_audio_source`, the share commands and the friends accept/remove/state commands.
  - Give the overlay `core:event:allow-listen` and `core:event:allow-unlisten` only.
  - Drop the unused window permissions from the console.

### 6. [Medium] [bug/a11y] The player's global key handler ignores other layers: Escape closes the player behind menus and dialogs, and arrow keys take over every slider — confirmed
- **Where:**
  - ClipPlayer.tsx:463-469 skips text inputs only, deliberately not range sliders: `(target instanceof HTMLInputElement && target.type !== "range") || target instanceof HTMLTextAreaElement`.
  - Line 499 looks up `handlers[event.key]` with no check of `event.defaultPrevented` and no check of modifier keys.
  - Line 492 Escape only respects `askingDelete` and `exporting`.
- **Conflicts:**
  - The context menu (Menu.tsx:156) handles Escape and arrow keys in the capture phase with `preventDefault` but no `stopPropagation`. Escape closes the menu and the player; ArrowDown moves the menu selection and lowers the volume.
  - SharePicker (Share.tsx:36), opened from the player's menu, closes on Escape and so does the player. Space, `n` and `p` act on the player behind it.
  - ClipEditor's local "Free up space" ConfirmDelete (`discarding`, ClipEditor.tsx:84) cancels on Escape, and the player closes too.
  - With a track-gain slider or the volume slider focused, ArrowLeft/Right seek ±5 s and ArrowUp/Down change the master volume. Keyboard users cannot move any slider; Home/End are taken as well.
  - Ctrl+N and Ctrl+P page between clips, Ctrl+F toggles fullscreen, and key repeat is not filtered.
- **Fix:**
  - Return early when `event.defaultPrevented`, when Ctrl/Meta/Alt is held, or when the target is any `input`, `select`, `textarea`, `[contenteditable]`, or `[role=slider]` that is not the scrubber.
  - Give dialogs and menus a shared "modal stack" (a zustand counter, or `stopPropagation` in Menu, SharePicker and ConfirmDelete), and lift `discarding` up to the player.

### 7. [Medium] [bug/UX] Unsaved trim and mix are lost without warning, and any reload of the video element resets the unsaved trim — confirmed
- **Where:**
  - ClipPlayer.tsx:756-759 runs `restoreTrim(clip.edit, length)` and then `if (restored) setTrim(restored)` on every `loadedmetadata`.
  - ClipPlayer.tsx:799 retries a failed load with `setReload(n+1)`.
  - Closing and paging have no check of `dirty`. ShotViewer, by contrast, asks "Discard unsaved marks?".
- **Scenarios:**
  - Escape, `n`/`p` or the close button drop a carefully set trim and mix. Only the "Not saved yet" line warned.
  - The source changes under the open player when another window files the clip. Example: the user hearts the same clip in the console, so Console.tsx:752 calls `fileClip`. The rename succeeds, because the asset protocol opens files per request with Rust's default share flags, which allow deletion. The store reloads with the new `clip.path`, the `<video>` src changes, `loadedmetadata` resets the trim, and playback restarts.
  - A single transient load error does the same.
- **Fix:**
  - Restore the trim only on the first metadata load for a clip id, or after save and restore (use a "trimRestoredFor" ref).
  - Add a dirty guard (an inline "Discard changes?" chip, as ShotViewer has) for close, paging and delete, and a Ctrl+S save shortcut.

### 8. [Medium] [bug] Clips without audio, or whose track query failed, can never be trimmed — confirmed
- **Where:**
  - ClipPlayer.tsx:535 `const dirty = duration > 0 && mix.tracks.length > 0 && !sameEdit(...)`
  - stems.rs `tracks()` returns `in_file` (an empty list) when the file has no audio stream.
  - useClipMix.ts:163 turns a failed `clipTracks` call into an empty track list (`.catch(() => undefined)`).
- **Effect:**
  - Setup allows "no audio source" ("your clips are silent"). Such clips, and any clip while ffprobe is unavailable (ffmpeg still downloading or failed), keep Save disabled forever.
  - The editor then also says "This clip carries a single audio track" (ClipEditor.tsx:123), which is wrong when the query failed.
- **Fix:** use `!mix.loading` instead of `tracks.length > 0`, and show a separate "Tracks could not be read" message on error.

### 9. [Medium] [UX] Clip actions in the main window fail silently and give no success feedback — confirmed
- **Where:**
  - clipMenu.tsx:136 `onSelect: () => void api.openClip(clip.id)`, and likewise 170 (`copyClipImage`), 180 (`copyClipFile`), 209 (`clipboardWriteText`), 216 (`revealClip`).
  - ClipPlayer.tsx:959 `onClick={() => inTauri && api.revealClip(clip.id)}`.
  - There is no `unhandledrejection` handler anywhere.
- **Effect:**
  - "Copy clip" when the file is missing rejects with "The clip file is no longer there." The rejection is unhandled and nothing is shown.
  - On success, nothing confirms the copy. The console says "Copied — Ctrl+V in Discord"; the app says nothing.
- **Also unhandled:**
  - The Accept/Decline buttons in Offers.tsx:29 and Share.tsx:298 (`action.finally(...)` with no catch).
  - Friends.tsx:78 swallows errors with `.catch(() => {})`.
  - store.ts:484 `deleteClip` removes the clip optimistically and never rolls back if `api.deleteClip` throws, so the clip disappears from the UI but stays on disk and in the database until the next reload.
- **Fix:** route every menu action through a helper that shows a success toast or `lastError`. Roll back `deleteClip` on failure.

### 10. [Medium] [bug] The stem-preview silence watchdog mistakes real silence for a broken audio graph — likely
- **Where:** useClipMix.ts:246-251 `quiet = samples.some((value) => value !== 0) ? 0 : quiet + 1; if (quiet >= SILENT_TICKS) { ... setGraphMode("off"); }` (8 ticks of 250 ms, i.e. 2 s).
- **What happens:**
  - The check exists to catch a CORS-silent `MediaElementSource`. But stems are often digitally zero: game loopback in menus or pauses, a noise-gated microphone.
  - Two seconds of real silence while playing switches the clip permanently to the `volume` fallback. The +dB preview is lost (all other tracks get quieter instead).
  - All audio elements are rebuilt mid-playback, which is audible as a glitch.
- **Also:**
  - The tolerance is coarse: `align()` re-seeks only above 0.12 s (line 317), checked once per second (line 353). Up to 120 ms of lip-sync error stays, and each correction is a hard seek.
  - Seeking does not pause the stems on `seeking`, so audio from the old position plays until `seeked`.
- **Fix:**
  - Detect the CORS case deterministically, e.g. `fetch(url, { mode: "cors", method: "HEAD" })` once, or decide only during the first seconds and only if the video's own audio is not silent.
  - Correct drift by nudging `playbackRate` (±1–2 %) and hard-seek only above about 250 ms. Pause the stems on `seeking`.
- **Minor, confirmed:** if `createMediaElementSource` throws after `new AudioContext()` succeeded (lines 207-228), the AudioContext is never closed.

### 11. [Medium] [bug/UX] Single-track clips: the track's gain and mute have no audible preview but are saved — confirmed
- **Where:**
  - useClipMix.ts:128 `const separate = tracks.length > 1 && ...`
  - The audio-elements effect returns early when not separate.
  - useClipMix.ts:280 `video.current.volume = separate ? 0 : master;`
- **Effect:** with one track, ClipEditor still renders a TrackRow (mute, solo, −30…+12 dB). Moving it changes nothing in the preview, but Save bakes it in.
- **Fix:** apply the single track's gain to the video element (attenuation via `volume`, or route the video through the same WebAudio graph for boost), or hide the row.

### 12. [Medium] [bug] The waveform is not refreshed after a trim, so it is misaligned with the new timeline — confirmed
- **Where:** ClipPlayer.tsx:316-326 fetches `api.clipWaveform(clipId)` with dependencies `[clipId]` only, and the URL has no cache-busting.
- **Effect:** Rust (`preview.rs`, `is_newer`) would regenerate the image, but the frontend never asks again after save or restore. The old full-length waveform is stretched over the trimmed clip, and the WebView may serve the cached PNG under the same URL.
- **Fix:** add `reload` (or `clip.sizeBytes`) to the dependencies and append `?v=${clip.sizeBytes}`, as is already done for thumbnails.

### 13. [Medium] [bug] Frame step is hard-coded to 30 fps, and the time display shows whole seconds only — confirmed
- **Where:**
  - ClipPlayer.tsx:59 `const FRAME = 1 / 30;`, used at 478: `".": () => seek(FRAME)`.
  - PlayerControls.tsx:15 `clock()` floors to seconds; ClipEditor shows `clock(trim.start) – clock(trim.end)`.
- **Effect:**
  - Recordings default to 60 fps, so "single frame" steps two frames.
  - Marks set while playing take `currentTime` (the audio clock), not the presented frame.
  - The handle tooltip and the trim readout cannot show the sub-second precision the cut needs.
- **Fix:**
  - Carry fps in `Clip`, or derive it from `requestVideoFrameCallback` metadata.
  - Use `rVFC.mediaTime` for marks, and for the end-of-selection loop check instead of rAF plus `currentTime`.
  - Display m:ss.cc or frames.

### 14. [Medium] [UX] Export dialog: the size shown can differ from the size exported, there is no cancel, and the player keeps reacting behind it — confirmed
- **Where:**
  - ExportDialog.tsx:177-181 updates the target only for valid input: `setCustom(text); ... if (Number.isFinite(value) && value >= 1) setTargetMb(value);`
- **Mismatches:**
  - Typing "0.5" keeps the previous 25 MB target while the field shows 0.5.
  - Typing "7" and then clearing the field leaves `targetMb = 7` with no preset highlighted and an empty field, yet Export is enabled.
- **Also:**
  - There is no way to cancel a long export: Cancel is disabled while busy, and Rust has no cancel command.
  - It exports the file on disk, not the unsaved selection, without saying so.
  - While it is open, `n`/`p` page the player. ExportDialog's `clip` prop follows the player's clip, so mid-run the dialog shows the next clip's numbers and the progress stalls.
  - It has no `role="dialog"`, `aria-modal`, initial focus or focus trap.
- **Fix:** derive `targetMb` from `custom` when it is non-empty and disable Export for invalid input; capture the clip at mount; add a cancel (kill ffmpeg); add a hint when `dirty`.

### 15. [Medium] [privacy/compliance] Usage stats are on by default, and seeing the notice counts as consent — likely a compliance risk
- **Where:**
  - config.rs:66 `usage_stats: true`.
  - Setup.tsx:88 `useState(true)`, then line 113 `setUsageStats(config.usageStats)`. This is a pre-ticked toggle on the last step.
  - UsageNotice marks itself as read after 10 s with the window focused: `if (due && shownMs >= SHOWN_MS) void patchConfig({ usageStatsNoticed: true })`.
  - telemetry.rs `send_if_due` sends a persistent UUID `id` plus `source` and `heard`.
- **Effect:**
  - Existing users who update are opted in by a notice that dismisses itself.
  - The notice text ("the version, how many clips, which features. No content, no account.") does not mention the persistent install ID or the attribution fields.
  - Under GDPR and the German TTDSG §25 (the codebase reads as German), a pre-ticked box is not valid consent (CJEU Planet49).
- **Fix:** make it opt-in (default `false`, toggle unticked), or at least require an explicit choice. List the install ID and the source/heard fields in the notice.

### 16. [Medium] [bug] The console mini-player has no error state, and one key handler can throw — confirmed
- **Where:**
  - Console.tsx:1314-1340: the `<video>` has no `onError`, and the `<img>` path has none either.
  - Console.tsx:1274 `jump(element, element.currentTime + 5)` with no `Number.isFinite(duration)` guard. `jump` computes `Math.min(x, NaN)`, so `currentTime = NaN` throws a TypeError before metadata is loaded.
  - Console.tsx:1270 skips all inputs including range inputs: once the volume slider has focus, every console player shortcut is dead (the opposite of the main player's choice).
- **Effect:** a clip deleted or moved by the main window, which `kept.current` keeps showing, leaves a black box with live controls.
- **Fix:** add an `onError` state ("File not found") and guard the seek. Better, share one `useVideoTransport` hook between ClipPlayer and ClipStage (see 25).

### 17. [Low] [bug] Trim thresholds: the UI says dirty and then "Saved" for an end cut the core ignores — confirmed
- **Where:** ClipPlayer.tsx:1069 `Math.abs(x - y) <= 50` versus edit.rs:43 `EDGE_TOLERANCE_MS: u64 = 250`.
- **Effect:**
  - Trimming 60–250 ms off the end enables Save. The core treats it as uncut, and the reload restores the full range, while the UI said "Saved".
  - Separately, `mark()` (ClipPlayer.tsx:451-455) does not clamp the end to the duration (`Math.max(at, current.start + 0.5)`), and silently moves the start to `end − 0.5` when the playhead is past it.
- **Fix:** share one tolerance (250 ms) for `dirty`, the "trimmed" flag and the core; clamp the end to `[start + 0.5, duration]`.

### 18. [Low] [UX] Solo is a listening aid in the UI but is saved as mutes — confirmed
- **Where:** useClipMix.ts:391-401 `muted: !audible(state, anySolo)`.
- **Effect:** soloing the game track to check something and then pressing Save permanently mutes every other track in the file. It is recoverable, because the stems remain, but surprising.
- **Fix:** exclude solo from the saved state and from `dirty`, or label it clearly ("Solo is saved as mute").

### 19. [Low] [perf] Dragging a trim handle re-renders the whole player and editor on every pointer move — confirmed
- **Where:**
  - PlayerControls.tsx:146 `onTrim(...)` per `pointermove` calls `setTrim` in ClipPlayer.
  - Each change re-renders ClipPlayer, ClipEditor, ClipMeta, Tags and all TrackRows.
  - It also tears down and re-creates the rAF effect (dependencies `[playing, trim.start, trim.end, paint]`) and the keydown listener.
- **Effect:** the authors note that WebView2 drops video frames when React renders at frame rate, which applies while dragging during playback.
- **Fix:** keep the trim in a ref during the drag and paint the handles directly (as is done for the playhead), then commit on `pointerup`. Read the trim from a ref inside the rAF loop and the key handler. Also handle `pointercancel` and use `setPointerCapture`.

### 20. [Low] [bug] Listener leak and side effects in ClipPlayer — confirmed (pattern)
- **Where:**
  - ClipPlayer.tsx:303-314 `.then((off) => { unlisten = off; }); return () => unlisten?.();`. If cleanup runs before `listen()` resolves (StrictMode, which `main.tsx` uses, or fast paging), the listener is never removed. ExportDialog.tsx:84-95 does this correctly.
  - ClipPlayer.tsx:191-195 schedules `goodbye.current = window.setTimeout(onClose, LEAVE_MS)` inside a `setLeaving` updater function. StrictMode calls updaters twice, so in development `onClose` fires twice.
  - Overlay.tsx:112-115 `const running = timers.current; ... running.forEach(clearTimeout)` clears the initial array, not the timers that later replaced it.
- **Fix:** add a `useTauriEvent(name, cb)` hook with a `cancelled` flag and use it everywhere; move the timeout out of the updater.

### 21. [Low] [bug] The waveform CSS `url()` is unquoted — confirmed
- **Where:** PlayerControls.tsx:185 ``backgroundImage: `url(${waveform})` ``
- **Effect:** `convertFileSrc` uses `encodeURIComponent`, which leaves `(` and `)` unescaped. A Windows profile such as `C:\Users\Max (Work)\…` breaks the declaration and the waveform never shows.
- **Fix:** write `` `url("${waveform}")` ``.

### 22. [Low] [a11y] Scrubber, trim handles, dialogs and labels — confirmed
- **Scrubber** (PlayerControls.tsx:162-178):
  - It is `role="slider"` with `tabIndex={0}` but has no key handling of its own (it relies on the global arrow keys).
  - `aria-valuenow` is a percentage that is not updated while playing (the rAF loop writes the DOM directly), and there is no `aria-valuetext`.
- **Trim handles:** pointer-only, with German labels in an English UI (line 252 ``aria-label={`${label} des Ausschnitts`}``).
- **Track buttons:** the mute buttons flip both the label and `aria-pressed`; the Solo buttons have no track context.
- **Dialogs and focus:**
  - The Slider has `outline-none` (Controls.tsx:116), so there is no focus ring.
  - Space on a focused button toggles playback instead of activating the button.
  - ClipPlayer, ExportDialog and SharePicker have no dialog role, no focus trap, and do not move or restore focus.
  - The Setup step dots carry `aria-label` on a plain div with no role.
- **Fix:** give the scrubber arrow keys and a time text, make the handles keyboard-operable (arrow = 1 frame, shift = 1 s), translate the labels, add focus management.

### 23. [Low] [perf] Smaller costs in the console and overlay — confirmed / likely
- The whole Console tree re-renders on every `engine-status`, `audio-errors` and `audio-warnings` event (1 Hz each, always fresh objects), including while hidden (Console.tsx:349-353, Trouble.tsx:41-42). Compare payloads before calling setState, or skip while `hidden.current`.
- `backdrop-blur` inside the transparent, GPU-less console (Console.tsx:642, Friends.tsx:157) costs CPU and, as overlay.css itself notes, blurs nothing behind the window.
- The REC badge blink (`cb-rec-blink 1.4s infinite`, overlay.css:321) composites at 60 fps on the CPU for the whole recording. Use `steps(2)` or an opacity toggle.
- SaveVeil's two canvases keep their size at video resolution (dom.ts `freezeFrame`) after every save: about 30 MB at 1440p and 66 MB at 4K. Reset them to 0×0 when the veil hides.

### 24. [Low] [bug/UX] Miscellaneous — confirmed
- **Init races:** share.ts:39-42 and friends.ts initialise with `invoke(...)` racing `listen(...)`, so an older snapshot can overwrite a newer event. Fetch after listening, or keep a version counter.
- **Fake initial status:** Console.tsx:54-69 `RESTING` mock status (`game: "ARC Raiders"`, 96 s buffered) is the initial state in production too. Use null and a skeleton.
- **One-click send:** the Friends "Latest clip" button (Friends.tsx:135-144) sends the newest item, which may be a screenshot or a 1-hour recording, in one click mid-game with no confirmation.
- **Offer identity:** offers show only `friendName` (taken from the local friend list, which is good) with no avatar, so two friends with the same display name cannot be told apart. The sender controls `t.name` and `t.game`; stripping bidi control characters is worth doing.
- **Unused code:** dead wrapper ipc.ts:150 `screenshotHasOriginal` calls a command that is not registered. `zuletzt()` (Friends.tsx:232) duplicates `lastSeenText`; there are three `clock()` copies (PlayerControls, ClipEditor, Setup).
- **Stale texts:** FfmpegNotice's "trying again in X" never counts down, and "Retry now" has no busy state. Setup's finish does not mention that ffmpeg may still be missing.
- **Volume:** player volume and mute are not persisted (ClipPlayer.tsx:109, Console.tsx:1210). Adding a tiny localStorage preference would fix it.
- **Playing state during save:** while saving, the `pause` event never arrives (see the comment in useClipMix). `playing` stays true, the rAF loop runs throughout, and Space calls `play()` on a source-less element, which is an unhandled rejection. Call `setPlaying(false)` in `save()`.

### 25. [Low] [quality] Structure — confirmed
- **ClipPlayer.tsx** (1100 lines) could be split into:
  - `useOpenCloseFlight`, shared with ShotViewer, which duplicates `LEAVE_MS`, the close animation and the `Step` component;
  - `useVideoTransport` (playing, time, duration, the rAF paint, toggle, seek, the trim loop), shared with ClipStage, which re-implements paint, rAF, toggle and seek;
  - `usePlayerShortcuts`, with a modal stack;
  - `useSaveFlow` (save, restore, discard, veil, reload);
  - `FullscreenBar`.
- **Console.tsx** (1500 lines; Console() alone is about 1000) could be split into `useConsoleLifecycle` (open/close tickets, `untilSized`, `finishClose`), `Dock`, `ClipsSheet`, `PerfSheet`, `ClipStage`.
- Comments and some strings mix German and English. Custom CSS properties need `as CSSProperties` casts; one typed helper would do.

## Checked and found fine

- **No XSS sinks.** There is no `dangerouslySetInnerHTML`, `innerHTML`, `window.open` or opener use in `src/`. Friend names, statuses, games and clip titles are React text. Avatars are limited by `img-src https://cdn.discordapp.com`. Share links are only copied, never opened.
- **No object URLs.** There are no `URL.createObjectURL` calls, so nothing leaks there.
- **The overlay never takes focus.** tao keeps `MARKER_DONT_FOCUS` from `.focused(false)`, so every `show()` uses `SW_SHOWNOACTIVATE`. `set_ignore_cursor_events` makes it click-through, and `to_top` uses `SWP_NOACTIVATE`. Banners are coalesced by a sequence ticket and the frontend replaces rather than queues, so nothing accumulates.
- **The console hands focus back correctly** (`take_foreground` before hide; `Focused(false)` clears `PREVIOUS`).
- **The audio-levels stream** goes only to windows that are actually visible (`window_awake` plus `emit_to`).
- **Element fullscreen works:** tauri-runtime-wry handles `ContainsFullScreenElementChanged` by making the window fullscreen.
- **Open video elements do not lock clip files.** Tauri's asset protocol opens the file per range request (at most 1 MB) with Rust's default Windows share flags, which allow deletion. The "let go of the file" step in save is not needed for locking (though the `?v=` cache-busting is), and the console's comment that filing a clip open in the main window "fails quietly" is likely wrong: the move succeeds (see 7).
- **Double save:** the core's render lock plus the disabled button prevent it.
- **Incoming file names from friends** are sanitised in `share.rs` (`safe_file_name`).

## Missing features and UX opportunities

**Player and editor**
- Frame- and fps-aware stepping with `requestVideoFrameCallback`, and an m:ss.ff readout with editable in/out timecodes.
- Drag-to-scrub with a hover thumbnail preview (sprite or WebCodecs).
- Keyboard-nudgeable trim handles.
- An unsaved-changes guard plus Ctrl+S; undo/redo for trim and mix.
- Cancel for save, restore and export (kill ffmpeg).
- Playback speed and J/K/L shuttle (`useClipMix` already follows `ratechange`); a loop toggle (looping is currently forced inside a selection).
- "Export selection" that honours the unsaved trim, with an estimated size and time and "open folder" afterwards.
- Autocomplete for the Game field (as Tags have) to avoid typo folders.
- Persisted volume; split and multi-segment cuts; choosing the thumbnail frame.

**Console**
- Unmount or pause content while hidden (finding 1).
- Per-job busy state with progress for "Shrink to 10/25 MB", plus cancel.
- An error state in ClipStage.
- Arrow-key navigation across the dock and a visible focus ring.
- An undo toast after delete; a confirm or undo for one-click "Latest clip" sends.
- Offers with avatar and expiry countdown.
- An option to turn off decorative motion (live ring, rim) for low-end PCs.

**Overlay**
- A short queue for important banners instead of replacing them.
- A `steps()` blink for the REC badge.
- Optionally `WS_EX_NOACTIVATE` or `focusable(false)` as a second guard against focus.
- A "do not disturb while recording a manual session" mode for friend notices.
