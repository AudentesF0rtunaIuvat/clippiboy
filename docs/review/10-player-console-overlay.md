# Clip player/editor UI, sharing UI, setup, console and overlay windows

**Scope:** ClipPlayer, ClipEditor, ClipMeta, clipMenu, ExportDialog, Share, Links, Setup, notices, PlayerControls, src/console/*, src/overlay/*.

**Findings:** 61 — 37 verified (4 medium or higher), 20 reported but not independently re-verified, 0 unverifiable/disputed, 4 rejected by verification.

Severity shown is the verified severity; where verification changed it, the reported one is in parentheses. See [README](README.md#how-findings-were-verified) for how verification worked.

## Verified issues (medium and above)

### FP-1 · Medium (reported High) · Hidden console keeps an infinite CSS animation (and full-screen glow layer) running for the whole session over the game

- **Where:** `Console.tsx:1025`, `motion.css:132`, `console.css:273`
- **Verification:** partial (code: partly confirmed; impact: partly confirmed; reachability: partly confirmed)

**What verification found:** The core is true: the console page keeps `<LiveDot />` mounted under an opacity-0 `.cb-ui`, so its infinite `cb-live-ring` animation keeps going while the window is hidden. Tauri's hide only calls ShowWindow(SW_HIDE) and never sets the WebView2 controller to IsVisible=false. With CalculateNativeWinOcclusion disabled and backgrounding turned off, nothing in the source would make Chromium throttle it. Four details are wrong. (1) Scope is larger than claimed: `closing` starts as true and console::create runs unconditionally at startup, so the ring runs from app launch whenever the buffer is on. That includes a console that was never opened and users who turned the console off. (2) The cost is overstated. The animated subtree sits under an opacity-0 effect, so Chromium most likely ticks the animation at vsync and asks for BeginFrames without drawing anything; that would be wasted CPU wakeups, not 60 fps of CPU raster plus composite. This depends on Chromium/WebView2 behaviour and cannot be checked here. (3) The glow is not a per-frame cost. Its animations are finite (300 ms in, 160 ms out), so afterwards it is only a static, invisible full-screen layer that holds memory. (4) The main window does not have the same flags: it lacks `--disable-gpu`. Its NavBar LiveDot is visible content in a window sent to the tray (window.hide only), so whatever it costs lands on the GPU the game is using.

**Original claim:** When the console closes, the dock is only faded to opacity 0 by `.cb-ui[data-leaving="true"] { animation: cb-fade-out 160ms ease-in both; pointer-events: none; }` (console.css:273) and stays mounted. Console.tsx:1025 renders `<LiveDot />` in the dock whenever the buffer is running, and its ring uses `.cb-live-ring { animation: cb-live-ring 2s var(--ease-out-soft) infinite; }` (motion.css:132), so the infinite animation keeps running while the console is hidden. The window is hidden with `ShowWindow(SW_HIDE)`; Tauri's hide never calls WebView2 `SetIsVisible(false)` (only `webview.hide()` would). The console runs with `--disable-renderer-backgrounding --disable-backgrounding-occluded-windows` and `CalculateNativeWinOcclusion` off, so nothing throttles it, and with `--disable-gpu` the frames are rasterised and composited on the CPU. The full-screen `.cb-glow` layer (`will-change: opacity`) also stays alive. Related: the main window has the same flags, so any infinite animation it shows (e.g. LiveDot) also runs while it sits in the tray.

**Scenario:** The user plays a game for hours or days with the buffer running; the hidden console's Chromium keeps producing compositor frames at about 60 fps, rasterised and composited on the CPU, while the game is in the foreground.

**Fix:** After the exit animation (`finishClose` or `console-closed`) render an empty page instead of an invisible dock: a `visible` flag that unmounts `.cb-ui` and the glow. At minimum add `.cb-ui[data-leaving="true"] * { animation-play-state: paused }` and drop `will-change` when hidden. Optionally call `webview.hide()` from Rust after `window.hide()` and `webview.show()` before showing.

**Verifier on the fix:** Unmounting the dock or glow (or showing the static dot) once the exit has played is correct and sufficient for the console. The proposed CSS shortcut `.cb-ui[data-leaving="true"] * { animation-play-state: paused }` is harmful: it also freezes `.cb-dock[data-leaving] { animation: cb-dock-out ... }` (console.css:427) and `.cb-sheet` cb-sheet-out (:226) at their first frame, which kills the slide-out exit. Target only the ring instead, e.g. `.cb-ui[data-leaving="true"] .cb-live-ring { animation-play-state: paused; }`, which matches the existing pattern at console.css:324/437/449, or render `<Liv…

### FP-12 · Medium · Clips without audio, or whose track query failed, can never be trimmed

- **Where:** `ClipPlayer.tsx:535`, `stems.rs`, `useClipMix.ts:163`, `ClipEditor.tsx:123`
- **Verification:** confirmed (reachability: confirmed; code: confirmed)

**Problem:** ClipPlayer.tsx:535 `const dirty = duration > 0 && mix.tracks.length > 0 && !sameEdit(...)` requires at least one track. stems.rs `tracks()` returns `in_file` (an empty list) when the file has no audio stream, and useClipMix.ts:163 turns a failed `clipTracks` call into an empty list (`.catch(() => undefined)`). Setup allows "no audio source" ("your clips are silent"); such clips, and any clip while ffprobe is unavailable (ffmpeg still downloading or failed), keep Save disabled forever. The editor then also says "This clip carries a single audio track" (ClipEditor.tsx:123), which is wrong when the query failed.

**Scenario:** User chose "no audio source" in Setup (or ffmpeg is still downloading), sets a trim, and Save never becomes enabled.

**Fix:** Use `!mix.loading` instead of `tracks.length > 0`, and show a separate "Tracks could not be read" message on error.

**Verifier on the fix:** Swapping in `!mix.loading` alone is unsafe. After a failed query (tracks=[]) on a clip with a stored edit, sameEdit sees a length mismatch, so the clip shows as dirty with nothing changed. Saving then sends an empty mix, and stems::levels resets every track to 0 dB and unmuted, wiping the stored mix. The fix should keep an empty list from a successful query apart from an error, compare only the trim when the list is legitimately empty, and block a save (or send clip.edit.tracks) after an error.

### FP-2 · Medium (reported High) · Dashboard player tracks its clip by position; a newly saved clip silently swaps the clip and discards unsaved edits

- **Where:** `Dashboard.tsx:80`, `Dashboard.tsx:272`, `ClipPlayer.tsx:86`, `ClipPlayer.tsx:287`, `src/routes/Dashboard.tsx:80-83`, `src/routes/Dashboard.tsx:271-279` (+1 more)
- **Verification:** partial (code: partly confirmed; reachability: partly confirmed; impact: partly confirmed)
- **Also reported as:** FC-8 (Dashboard player tracks a position in a live list and never files clips)

**What verification found:** The mechanism is real. The Dashboard player is indexed by position into `recent`, the top 3 non-screenshot, non-recording clips. A new buffer clip saved by the global hotkey, tray, Stream Deck or the console, or an accepted friend's clip that finishes arriving, is prepended to `clips`. Deleting a clip that sits before the shown one (from the console, say) shifts the list too. Either way the same index now points at a different clip; the `[clip?.id]` reset clears the unsaved trim and useClipMix drops the mix. If the shown clip itself is deleted elsewhere, the player silently moves to a neighbour, where Clips.tsx would close. One trigger the claim lists is wrong: a finished (or recovered) recording does NOT shift the list, because recordings carry `recording: true` and Dashboard.tsx:81 filters them out. The same goes for screenshots. Only unsaved, in-player edits (trim marks, mix sliders) are lost; nothing on disk is affected. The merged FC-8 point also holds: a favorite toggled in the Dashboard player never calls fileClip, so the file stays in its old folder until it is next opened from the gallery. The code itself calls that folder placement cosmetic (store.ts:556).

**Original claim:** Dashboard.tsx:80 `const recent = useMemo(() => clips.filter((clip) => !clip.screenshot && !clip.recording).slice(0, 3), [clips]);` and Dashboard.tsx:272 `<ClipPlayer clips={recent} index={Math.min(playing, recent.length - 1)} ...` pass a position, and ClipPlayer.tsx:86 resolves `const clip = clips[index];`. When `onClipSaved` prepends a new clip (save hotkey, a finished recording, or a friend's clip arriving), `recent[playing]` becomes a different clip; the player's reset effect (ClipPlayer.tsx:287) clears the trim and `useClipMix` drops the mix. Deletes made from the console shift the list the same way. Clips.tsx already solves this with an id playlist plus `shownId`.

**Scenario:** The user is trimming the newest clip from the dashboard and presses the save hotkey (or a recording finishes / a friend's clip arrives); the player silently switches to another clip and the unsaved trim and mix are discarded.

**Fix:** Track the clip by id: pass a playlist snapshot of ids, or reuse the Clips.tsx playlist logic through a shared hook.

**Verifier on the fix:** Tracking by id is correct, but a naive fix that looks the id up in `recent` is not enough: a new clip can push the clip on screen from slot 2 to slot 3, out of `slice(0, 3)`. Minimal fix: when the player opens, freeze the playlist as `recent.map(c => c.id)`, derive the clips from the store by id, keep the shown id in state, and close when that id disappears unless this player deleted it. Factoring the Clips.tsx shownId/coming/deletedHere logic into a shared hook does the same and would also take care of FC-8's fileClip-on-close.

### FP-5 · Medium (reported High) · A running Save/Undo-trim does not block paging, closing or deleting

- **Where:** `ClipPlayer.tsx:547-582`, `ClipPlayer.tsx:253`
- **Verification:** confirmed (code: confirmed; impact: partly confirmed; reachability: confirmed)

**What verification found:** The UI part is correct. If you page to clip B while clip A is saving or undoing a trim, B gets A's outcome: a false 'Saved. That is how the clip sits in the folder now' (when B has no unsaved changes), its video element restarts once, the editor shows 'Saving…' and its progress stops moving, and A's frozen frame covers B while B's audio plays. B's own Save is blocked until A's job ends. The Rust races are real but small. Closing during a save only calls fileClip(A) from Clips.tsx (Dashboard does not file). While ffmpeg holds the input the move normally fails and is logged as a no-op (commands.rs:760-763), and tidy files it at the next start. If the move does land in a gap (for example while apply waits on WORKING), the save fails with an error notice ('The clip file is no longer there.') and the clip stays intact at its new path. Deleting during a save (Delete plus confirm while saving) removes the DB row and the originals store mid-job. The job can then still write the edited file, and on a first cut also an archived original, back to disk. store() then fails with 'clip not found', leaving undeleted orphan files and a notice, with no loss of user footage. Paging back to A during its own save also re-opens A's file under the running job, so swap_in can fail and roll back. None of this loses data or crashes the app, so the claimed 'high' is overstated.

**Original claim:** `save()` at ClipPlayer.tsx:547-582 (`resumeAt`, `veil.freeze`, `await applyClipEdit`, then in `finally` `setReload((n)=>n+1)` and `setJustSaved(true)`) runs while the keyboard handler (`n`/`p` -> `step`, Escape -> `close`), the Step buttons, Delete and the close button stay enabled. ClipPlayer.tsx:253 resets `justSaved` only when the clip changes, before the save finishes. Paging to clip B during A's save: B plays under A's frozen frame; when A finishes, `setJustSaved(true)` makes B show "Saved. That is how the clip sits in the folder now"; `setReload` re-creates B's `<video>` so B restarts; the editor shows "Saving..." for B and the progress bar stalls because the listener now filters on B's id. Closing during a save calls Clips.closePlayer, which runs `fileClip` while the edit job may be between ffmpeg and `swap_in`; deleting during a save runs `delete_clip` while `apply` runs. The core's render lock does not cover filing or deletion. (UI part confirmed; Rust races speculative.)

**Scenario:** The user saves clip A (a re-encode can take minutes) and presses `n`; clip B is wrongly reported as saved, restarts, and shows a stalled progress bar. Closing or deleting during the save races `fileClip`/`delete_clip` against the edit job.

**Fix:** While `saving || restoring`, disable step, close, delete and the shortcuts; alternatively key `saving` and `justSaved` by clip id and ignore results for a clip that is no longer shown.

**Verifier on the fix:** Disabling step, close and delete while saving or restoring fixes it, but doing it button by button misses the backdrop click, Escape, the context-menu delete and the external auto-close path in Clips.tsx. A better minimal fix is a single busy guard (a ref mirroring saving||restoring) at the top of `step`, `close` and `removeClip`, plus `disabled` on the Step and Delete buttons for the visuals. If closing during a long re-encode should still work, record the clip id in save()/restoreOriginal() instead, and in `finally` only call setJustSaved/setReload/setSettling when that id is still on scree…

## Reported, not independently re-verified

The original reviewer checked these against the code, but the independent verification pass was stopped before reaching them. Treat as likely, confirm before fixing.

- **FP-16 · Medium · Optimistic deleteClip never rolls back on failure** — `store.ts:484`  
  store.ts:484 `deleteClip` removes the clip from the store optimistically and never rolls back if `api.deleteClip` throws, so the clip disappears from the UI but stays on disk and in the database until the next reload.  
  *Fix:* Roll back `deleteClip` on failure and show the error.
- **FP-19 · Medium · Stems are not paused while the video is seeking** — `useClipMix.ts`  
  useClipMix does not pause the stem `<audio>` elements on the video's `seeking` event, so audio from the old position keeps playing until `seeked`.  
  *Fix:* Pause the stems on `seeking`.
- **FP-21 · Medium · Single-track clips: track gain and mute have no audible preview but are saved** — `useClipMix.ts:128`, `useClipMix.ts:280`  
  useClipMix.ts:128 `const separate = tracks.length > 1 && ...`; the audio-elements effect returns early when not separate, and useClipMix.ts:280 sets `video.current.volume = separate ? 0 : master;`. With one track, ClipEditor still renders a TrackRow (mute, solo, -30...+12 dB); moving it changes nothing in the preview, but Save bakes it into the file.  
  *Fix:* Apply the single track's gain to the video element (attenuation via `volume`, or route the video through the same WebAudio graph for boost), or hide the row.
- **FP-22 · Medium · Waveform is not refreshed after a trim and is misaligned with the new timeline** — `ClipPlayer.tsx:316-326`  
  ClipPlayer.tsx:316-326 fetches `api.clipWaveform(clipId)` with dependencies `[clipId]` only, and the URL has no cache-busting. Rust (`preview.rs`, `is_newer`) would regenerate the image, but the frontend never asks again after save or restore, so the old full-length waveform is stretched over the trimmed clip, and the WebView may serve the cached PNG under the same URL.  
  *Fix:* Add `reload` (or `clip.sizeBytes`) to the dependencies and append `?v=${clip.sizeBytes}`, as is already done for thumbnails.
- **FP-23 · Medium · Frame step is hard-coded to 30 fps** — `ClipPlayer.tsx:59`, `ClipPlayer.tsx:478`  
  ClipPlayer.tsx:59 `const FRAME = 1 / 30;` is used at line 478 `".": () => seek(FRAME)`. Recordings default to 60 fps, so a "single frame" step moves two frames.  
  *Fix:* Carry fps in `Clip`, or derive it from `requestVideoFrameCallback` metadata.
- **FP-24 · Medium · Trim marks and the selection loop use currentTime instead of the presented frame** — `ClipPlayer.tsx`  
  Marks set while playing take `currentTime` (the audio clock), not the presented frame, and the end-of-selection loop check uses rAF plus `currentTime`.  
  *Fix:* Use `rVFC.mediaTime` (requestVideoFrameCallback) for marks and for the end-of-selection loop check instead of rAF plus `currentTime`.
- **FP-25 · Medium · Time display and trim readout show whole seconds only** — `PlayerControls.tsx:15`, `ClipEditor.tsx`  
  PlayerControls.tsx:15 `clock()` floors to seconds, and ClipEditor shows `clock(trim.start) – clock(trim.end)`, so the handle tooltip and trim readout cannot show the sub-second precision a cut needs.  
  *Fix:* Display m:ss.cc or frames.
- **FP-26 · Medium · Export dialog: shown size can differ from the exported target size** — `ExportDialog.tsx:177-181`  
  ExportDialog.tsx:177-181 updates the target only for valid input: `setCustom(text); ... if (Number.isFinite(value) && value >= 1) setTargetMb(value);`. Typing "0.5" keeps the previous 25 MB target while the field shows 0.5. Typing "7" and then clearing the field leaves `targetMb = 7` with no preset highlighted and an empty field, yet Export is enabled.  
  *Fix:* Derive `targetMb` from `custom` when it is non-empty and disable Export for invalid input.
- **FP-27 · Medium · Export cannot be cancelled** — `ExportDialog.tsx`  
  There is no way to cancel a long export: Cancel is disabled while busy, and Rust has no cancel command.  
  *Fix:* Add a cancel that kills ffmpeg.
- **FP-28 · Medium · Export uses the file on disk, not the unsaved selection, without saying so** — `ExportDialog.tsx`  
  ExportDialog exports the file on disk, not the unsaved trim selection, and does not tell the user.  
  *Fix:* Add a hint when `dirty`.
- **FP-29 · Medium · Player keeps reacting behind the Export dialog and the dialog follows the player's clip** — `ExportDialog.tsx`  
  While ExportDialog is open, `n`/`p` still page the player. ExportDialog's `clip` prop follows the player's clip, so mid-run the dialog shows the next clip's numbers and the progress stalls.  
  *Fix:* Capture the clip at mount.
- **FP-31 · Medium · Usage stats are on by default and a self-dismissing notice counts as consent** — `config.rs:66`, `Setup.tsx:88`, `Setup.tsx:113` (+4 more)  
  config.rs:66 `usage_stats: true`; Setup.tsx:88 `useState(true)` then line 113 `setUsageStats(config.usageStats)` - a pre-ticked toggle on the last setup step. UsageNotice marks itself as read after 10 s with the window focused: `if (due && shownMs >= SHOWN_MS) void patchConfig({ usageStatsNoticed: true })`. telemetry.rs `send_if_due` sends a persistent UUID `id` plus `source` and `heard`. Existing users who update are opted in by a notice that dismisses itself. Under GDPR and German TTDSG §25, a pre-ticked box is not valid consent (CJEU Planet49).  
  *Fix:* Make it opt-in (default `false`, toggle unticked), or at least require an explicit choice.
- **FP-32 · Medium · Usage notice text omits the persistent install ID and attribution fields** — `UsageNotice`, `telemetry.rs`  
  The UsageNotice text ("the version, how many clips, which features. No content, no account.") does not mention that telemetry.rs `send_if_due` sends a persistent install UUID `id` and the attribution fields `source` and `heard`.  
  *Fix:* List the install ID and the source/heard fields in the notice.
- **FP-33 · Medium · Console mini-player (ClipStage) has no error state** — `Console.tsx:1314-1340`  
  In Console.tsx:1314-1340 the `<video>` has no `onError`, and the `<img>` path has none either. A clip deleted or moved by the main window, which `kept.current` keeps showing, leaves a black box with live controls.  
  *Fix:* Add an `onError` state ("File not found"); better, share one `useVideoTransport` hook between ClipPlayer and ClipStage.
- **FP-34 · Medium · Console seek shortcut can throw before metadata is loaded** — `Console.tsx:1274`  
  Console.tsx:1274 `jump(element, element.currentTime + 5)` has no `Number.isFinite(duration)` guard; `jump` computes `Math.min(x, NaN)`, so assigning `currentTime = NaN` throws a TypeError before metadata is loaded.  
  *Fix:* Guard the seek with `Number.isFinite(duration)`.
- **FP-35 · Medium · Console player shortcuts die once the volume slider has focus** — `Console.tsx:1270`  
  Console.tsx:1270 skips all inputs including range inputs, so once the volume slider has focus every console player shortcut is dead (the opposite of the main player's choice).  
  *Fix:* Share one `useVideoTransport`/shortcut logic between ClipPlayer and ClipStage with consistent input filtering.
- **FP-49 · Medium · ClipPlayer, ExportDialog and SharePicker have no dialog role or focus management** — `ClipPlayer.tsx`, `ExportDialog.tsx`, `Share.tsx` (+1 more)  
  ClipPlayer, ExportDialog and SharePicker have no dialog role, no focus trap, and do not move focus in or restore it on close.  
  *Fix:* Add dialog roles and focus management (move focus in, trap, restore).
- **FP-7 · Medium · Overlay capability `core:event:default` lets the overlay emit events to the main window** — `overlay.json`  
  overlay.json grants `"permissions": ["core:event:default"]` with the description "only listens, controls nothing". `core:event:default` includes `allow-emit` and `allow-emit-to`, so the overlay can send `share-state`, `focus-clip`, `update-available` or `main-hidden` to the main window.  
  *Fix:* Give the overlay `core:event:allow-listen` and `core:event:allow-unlisten` only.
- **FP-8 · Medium · Console capability grants unused window permissions and broad `core:default`** — `console capability`  
  The console capability grants `core:window:allow-hide/show/set-focus`, which its JavaScript never uses, and `core:default` (tray, menu and more).  
  *Fix:* Drop the unused window permissions from the console capability (and restrict it to the listed command set).
- **FP-9 · Medium · Player's global key handler ignores menus/dialogs and hijacks keys on sliders** — `ClipPlayer.tsx:463-469`, `ClipPlayer.tsx:499`, `ClipPlayer.tsx:492` (+3 more)  
  ClipPlayer.tsx:463-469 skips only text inputs, deliberately not range sliders: `(target instanceof HTMLInputElement && target.type !== "range") \|\| target instanceof HTMLTextAreaElement`. Line 499 looks up `handlers[event.key]` without checking `event.defaultPrevented` or modifier keys; line 492's Escape only respects `askingDelete` and `exporting`. Conflicts: the context menu (Menu.tsx:156) handles Escape and arrows in the capture phase with `preventDefault` but no `stopPropagation`, so Escape closes the menu and the player, and ArrowDown moves the menu selection and lowers the volume. ShareP…  
  *Fix:* Return early when `event.defaultPrevented`, when Ctrl/Meta/Alt is held, or when the target is any `input`, `select`, `textarea`, `[contenteditable]` or `[role=slider]` other than the scrubber; give dialogs and menus a shared modal stack (zustand counter, or `stopPropagation` in Menu, SharePicker and ConfirmDelete) and lift `discarding` up to the p…

## Verified lower-severity issues

| ID | Severity | Title | Where | Fix |
|---|---|---|---|---|
| FP-10 | Low (reported Medium) | Unsaved trim and mix are lost without warning on close or paging | ClipPlayer.tsx | Add a dirty guard (an inline "Discard changes?" chip, as ShotViewer has) for close, paging and delete, and a Ctrl+S save shortcut. |
| FP-11 | Low (reported Medium) | Any reload of the video element resets the unsaved trim via restoreTrim on loadedmetadata | ClipPlayer.tsx:756-759, ClipPlayer.tsx:799 | Restore the trim only on the first metadata load for a clip id, or after save and restore (use a "trimRestoredFor" ref). |
| FP-13 | Low (reported Medium) | Clip menu actions in the main window fail silently and give no success feedback | clipMenu.tsx:136, clipMenu.tsx:170 | Route every menu action through a helper that shows a success toast or `lastError`. |
| FP-14 | Low (reported Medium) | Offer Accept/Decline buttons have no error handling | Offers.tsx:29, Share.tsx:298 | Route the actions through a helper that shows a success toast or `lastError`. |
| FP-15 | Low (reported Medium) | Friends.tsx swallows errors | Friends.tsx:78 | Show the error via a toast or `lastError` instead of swallowing it. |
| FP-3 | Low | Dashboard player never files favorites on close | Dashboard.tsx | Reuse the Clips.tsx playlist/close logic (including `closePlayer` filing) through a shared hook. |
| FP-36 | Low | Trim tolerance mismatch: UI says dirty/"Saved" for an end cut the core ignores | ClipPlayer.tsx:1069, edit.rs:43 | Share one tolerance (250 ms) for `dirty`, the "trimmed" flag and the core. |
| FP-38 | Low | Solo is a listening aid in the UI but is saved as mutes | useClipMix.ts:391-401 | Exclude solo from the saved state and from `dirty`, or label it clearly ("Solo is saved as mute"). |
| FP-39 | Low | Dragging a trim handle re-renders the whole player and editor on every pointer move | PlayerControls.tsx:146 | Keep the trim in a ref during the drag and paint the handles directly (as for the playhead), commit on `pointerup`; read trim from a ref inside the rAF loop and key handler; handle `pointercancel` and use `setPointerCap… |
| FP-40 | Low | Tauri event listener leak in ClipPlayer when cleanup runs before listen() resolves | ClipPlayer.tsx:303-314 | Add a `useTauriEvent(name, cb)` hook with a `cancelled` flag and use it everywhere. |
| FP-43 | Low | Waveform CSS url() is unquoted | PlayerControls.tsx:185 | Write `` `url("${waveform}")` ``. |
| FP-44 | Low | Scrubber slider lacks own key handling and live ARIA values | PlayerControls.tsx:162-178 | Give the scrubber arrow keys and a time text (`aria-valuetext`). |
| FP-45 | Low | Trim handles are pointer-only and have German labels in an English UI | PlayerControls.tsx:252 | Make the handles keyboard-operable (arrow = 1 frame, shift = 1 s) and translate the labels. |
| FP-46 | Low | Track mute/solo buttons have confusing or missing accessible names | ClipEditor.tsx | Keep a stable label with `aria-pressed`, and include the track name in Solo labels. |
| FP-48 | Low | Space on a focused button toggles playback instead of activating the button | ClipPlayer.tsx | Skip the global Space shortcut when a button or other interactive control is focused. |
| FP-50 | Low | Setup step dots put aria-label on a plain div with no role | Setup.tsx | Give the element an appropriate role (or use visually hidden text). |
| FP-53 | Low | REC badge blink composites at 60 fps on the CPU for the whole recording | overlay.css:321 | Use `steps(2)` or an opacity toggle. |
| FP-54 | Low | SaveVeil canvases keep video-resolution size after every save | dom.ts | Reset the canvases to 0x0 when the veil hides. |
| FP-57 | Low | Friends "Latest clip" sends the newest item in one click without confirmation | Friends.tsx:135-144 | Add a confirm or undo, or restrict to clips. |
| FP-61 | Low | FfmpegNotice countdown never counts down and Retry has no busy state | FfmpegNotice | Tick the countdown and show a busy state on Retry. |
| FP-63 | Low | Player volume and mute are not persisted | ClipPlayer.tsx:109, Console.tsx:1210 | Add a small localStorage preference. |
| FP-64 | Low | `playing` stays true during save; Space calls play() on a source-less element | ClipPlayer.tsx, useClipMix.ts | Call `setPlaying(false)` in `save()`. |
| FP-65 | Low | ClipPlayer.tsx is a 1100-line component duplicating logic with ShotViewer and ClipStage | ClipPlayer.tsx | Split into `useOpenCloseFlight` (shared with ShotViewer), `useVideoTransport` (playing, time, duration, rAF paint, toggle, seek, trim loop; shared with ClipStage), `usePlayerShortcuts` with a modal stack, `useSaveFlow`… |
| FP-66 | Low | Console.tsx is a 1500-line file with a ~1000-line Console() component | Console.tsx | Split into `useConsoleLifecycle` (open/close tickets, `untilSized`, `finishClose`), `Dock`, `ClipsSheet`, `PerfSheet`, `ClipStage`. |
| FP-67 | Low | Comments and some strings mix German and English | src/ | Standardise on one language for comments and strings. |
| FP-69 | Low | Misleading comments about clip file locking ("fails quietly" and "let go of the file") | Console.tsx, ClipPlayer.tsx | Correct the comments to reflect that the move succeeds. |
| FP-41 | Info (reported Low) | Close timeout scheduled inside a setState updater fires twice under StrictMode | ClipPlayer.tsx:191-195 | Move the timeout out of the updater. |
| FP-42 | Info (reported Low) | Overlay timer cleanup clears the initial array, not the current timers | Overlay.tsx:112-115 | Clear `timers.current` at cleanup time (read the ref when cleaning up). |
| FP-51 | Info (reported Low) | Whole Console tree re-renders on every 1 Hz status event, even while hidden | Console.tsx:349-353, Trouble.tsx:41-42 | Compare payloads before calling setState, or skip while `hidden.current`. |
| FP-58 | Info (reported Low) | Offers cannot distinguish same-named friends and do not strip bidi characters | Offers.tsx | Show an avatar; strip bidi control characters from sender-controlled fields. |
| FP-60 | Info (reported Low) | Duplicated helpers: zuletzt() and three clock() copies | Friends.tsx:232, PlayerControls.tsx | Consolidate into shared helpers. |
| FP-62 | Info (reported Low) | Setup finish does not mention that ffmpeg may still be missing | Setup.tsx | Mention ffmpeg status on the finish step. |
| FP-68 | Info (reported Low) | Custom CSS properties require repeated `as CSSProperties` casts | src/ | Add one typed helper for custom CSS properties. |

## Claims rejected by verification

| ID | Claim | Why it was rejected |
|---|---|---|
| FP-37 | mark() does not clamp the end to duration and silently moves the start | src/components/ClipPlayer.tsx:447-456 `const at = element.currentTime; setTrim(current => which === "start" ? { start: Math.min(at, current.end - 0.5), end: current.end } : { start: current.start, end: Math.max(at, current.start + 0.5) });`. `at` is currentTime, which is at most the duration. Every… |
| FP-47 | Slider has no focus ring | src/index.css:64-67 defines an unlayered `:focus-visible { outline: 2px solid var(--color-accent); outline-offset: 2px; }`. Tailwind v4 (node_modules/tailwindcss 4.3.3) puts `outline-none` (`--tw-outline-style:none; outline-style:none`) in `@layer utilities`. The compiled dist/assets/accent-CTvuYMT… |
| FP-52 | backdrop-blur in the transparent GPU-less console costs CPU and blurs nothing | Console.tsx:635-642: the blur sits on a 40x40 hover pill inside a clip tile, over the tile's own thumbnail: `<img src=... className="h-full w-full object-cover" />` then `<span className="absolute inset-0 ... bg-black/35 opacity-0 ... group-hover:opacity-100"><span className="grid h-10 w-10 ... bg-… |
| FP-56 | Fake RESTING mock status is the console's initial state in production | Console.tsx:52-53 documents RESTING as the pre-core placeholder for `npm run dev`. In the app it never reaches the screen. (1) The console page loads at startup into a hidden window (console.rs:54-55 'Create the console window, hidden. Called once at startup', lib.rs:1241), and on mount Console.tsx… |

## What is solid

- No XSS sinks: there is no `dangerouslySetInnerHTML`, `innerHTML`, `window.open` or opener use in `src/`; friend names, statuses, games and clip titles are rendered as React text; avatars are limited by `img-src https://cdn.discordapp.com`; share links are only copied, never opened.
- The CSP blocks inline scripts.
- No object URLs: there are no `URL.createObjectURL` calls, so nothing leaks there.
- The overlay never takes focus: tao keeps `MARKER_DONT_FOCUS` from `.focused(false)`, so every `show()` uses `SW_SHOWNOACTIVATE`; `set_ignore_cursor_events` makes it click-through; `to_top` uses `SWP_NOACTIVATE`.
- Overlay banners are coalesced by a sequence ticket and the frontend replaces rather than queues, so nothing accumulates.
- The console hands focus back correctly (`take_foreground` before hide; `Focused(false)` clears `PREVIOUS`).
- The audio-levels stream goes only to windows that are actually visible (`window_awake` plus `emit_to`).
- Element fullscreen works: tauri-runtime-wry handles `ContainsFullScreenElementChanged` by making the window fullscreen.
- Open video elements do not lock clip files: Tauri's asset protocol opens the file per range request (at most 1 MB) with Rust's default Windows share flags, which allow deletion.
- Double save is prevented by the core's render lock plus the disabled button.
- Incoming file names from friends are sanitised in `share.rs` (`safe_file_name`).
- Offers show `friendName` taken from the local friend list rather than a sender-supplied name.
- ExportDialog.tsx:84-95 handles Tauri `listen()`/unlisten cleanup correctly.
- Thumbnails already use `?v=${clip.sizeBytes}` cache-busting.

## Improvement opportunities and missing features

| ID | Idea | Why | Effort | Sketch |
|---|---|---|---|---|
| FP-P-1 | Frame- and fps-aware stepping with an editable m:ss.ff timecode readout | Precise cuts at the clip's real frame rate with in/out points that can be typed. | ? | Use `requestVideoFrameCallback` for fps-aware stepping; show an m:ss.ff readout with editable in/out timecodes (ClipPlayer, PlayerControls, ClipEditor). |
| FP-P-2 | Drag-to-scrub with hover thumbnail preview | Faster navigation to the right moment in a clip. | ? | Scrubber hover/drag preview using a thumbnail sprite or WebCodecs (PlayerControls Scrubber). |
| FP-P-3 | Keyboard-nudgeable trim handles | Keyboard users and precise editing. | ? | Arrow keys move the focused trim handle (PlayerControls trim handles). |
| FP-P-4 | Unsaved-changes guard plus Ctrl+S; undo/redo for trim and mix | Prevents losing edits and allows recovering from mistakes. | ? | Dirty guard on close/paging/delete, Ctrl+S shortcut, undo/redo history for trim and mix state (ClipPlayer, useClipMix). |
| FP-P-5 | Cancel for save, restore and export | Users can abort long ffmpeg jobs. | ? | Add a Rust cancel command that kills ffmpeg and wire Cancel buttons in the save/restore flow and ExportDialog. |
| FP-P-6 | Playback speed, J/K/L shuttle and a loop toggle | Standard editor transport controls; looping is currently forced inside a selection. | ? | Add playback-rate control and J/K/L shuttle (`useClipMix` already follows `ratechange`); add a loop toggle in ClipPlayer. |
| FP-P-7 | "Export selection" honouring the unsaved trim | Export exactly what the user selected without saving first, with expectations set. | ? | Export the unsaved trim selection, show estimated size and time, and offer "open folder" afterwards (ExportDialog, export_clip). |
| FP-P-8 | Autocomplete for the Game field | Avoids typo folders. | ? | Add autocomplete to the Game field in ClipMeta, as Tags already have. |
| FP-P-9 | Persisted player volume | Volume and mute survive reopening the player. | ? | Store volume/mute in a small localStorage preference (ClipPlayer, Console ClipStage). |
| FP-P-10 | Split and multi-segment cuts | Cut out the middle of a clip or keep several segments. | ? | Extend the edit model and trim UI to multiple segments (ClipEditor, edit.rs). |
| FP-P-11 | Choose the thumbnail frame | Users pick a representative thumbnail for a clip. | ? | Add a "use this frame as thumbnail" action in the player. |
| FP-P-12 | Unmount or pause console content while hidden | Removes the CPU cost of the hidden console over the game (see FP-1). | ? | Unmount `.cb-ui` and the glow after the exit animation, or pause animations while hidden (Console.tsx, console.css). |
| FP-P-13 | Per-job busy state with progress and cancel for "Shrink to 10/25 MB" | Users see progress for each shrink job and can cancel it. | ? | Track busy per job in Console.tsx, show `clip-progress`, add a cancel command. |
| FP-P-14 | Error state in ClipStage | A missing file is reported instead of a black box. | ? | Add `onError` handling to ClipStage's video/img (Console.tsx). |
| FP-P-15 | Arrow-key navigation across the dock and a visible focus ring | Keyboard/controller-friendly console. | ? | Roving tabindex with arrow keys across dock buttons; visible focus styles (Console.tsx, console.css). |
| FP-P-16 | Undo toast after delete; confirm or undo for one-click "Latest clip" sends | Recover from accidental deletes and sends. | ? | Undo toast after delete in the console; confirm/undo step on the Friends "Latest clip" button. |
| FP-P-17 | Offers with avatar and expiry countdown | Users can tell senders apart and know how long an offer stays valid. | ? | Show friend avatar and an expiry countdown on offers (Offers.tsx). |
| FP-P-18 | Option to turn off decorative motion | Saves CPU on low-end PCs. | ? | A setting that disables the live ring and rim animations. |
| FP-P-19 | Short queue for important overlay banners | Important banners are not lost when replaced by newer ones. | ? | Queue important banners instead of replacing them (Overlay.tsx, overlay.rs). |
| FP-P-20 | steps() blink for the REC badge | Cheaper rendering during recordings. | ? | Change `cb-rec-blink` to `steps(2)` timing (overlay.css). |
| FP-P-21 | Second guard against overlay focus | Extra protection against the overlay stealing focus from the game. | ? | Optionally set `WS_EX_NOACTIVATE` or `focusable(false)` on the overlay window (overlay.rs). |
| FP-P-22 | "Do not disturb while recording a manual session" mode for friend notices | No friend banners interrupt a manual recording. | ? | Suppress friend notice banners while a manual recording runs. |
