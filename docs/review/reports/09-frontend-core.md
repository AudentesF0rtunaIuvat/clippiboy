# ClippiBoy frontend review: core, routes and shared libs

I changed no repo files. `npx tsc --noEmit` passes. I also type-checked with stricter flags through a scratch config (`<review-scratch>/tsconfig.strict.json`): 51 errors, quoted below where relevant.

## Architecture overview
- **Entry points.** `main.tsx` runs `installLockdown()` and `installAccent({turning:true})`, then renders `<App>` under `StrictMode`.
- **Routing.** `App.tsx` keeps the route in `useState`; there is no router. Each route renders inside `<div key={route}>`, so it unmounts on every switch. Global overlays (Toasts, Transfers, Share pickers, Setup, TextMenu) sit under one `MenuProvider`.
- **State.** One Zustand store, `useEngine` in `store.ts` (652 lines), holds config, devices, targets, encoders, clips, audio levels, engine status, update and ffmpeg state, `lastError` and the settings tab.
  - `init()` loads everything with `Promise.all`, then registers about 14 Tauri `listen()` handlers. They are never unlistened.
  - Three small module-singleton stores (`friends.ts`, `share.ts`, `links.ts`) register their listeners at import time.
- **IPC.** `lib/ipc.ts` wraps `invoke`/`listen`. The `api`/`events` objects and `types.ts` mirror `model.rs` by hand (serde camelCase).
- **Mutations.** Most are optimistic (heart, tags, sources, config). Some deliberately are not (hotkeys, clip edits, clip dir).
- **Browser mode.** When `!inTauri`, `mock.ts` supplies the data. In Tauri mode `mockConfig` is also the store's initial config.
- **Animation.** `motion.css` uses `cb-*` keyframes plus a reduced-motion block. `useFlip`/`usePresence`/`useCountUp`/`SlidingIndicator` are the JS motion helpers (WAAPI).
- **Theming.** Tailwind 4 `@theme` tokens. `accent.ts` rewrites 16 CSS custom properties on `<html>` (every 80 ms in RGB mode).
- **Security surface.** CSP in `tauri.conf.json`; the main window has `opener:default`, `dialog:default` and `global-shortcut:default`.
- **What is done well.**
  - `listen()` cleanup in `App.tsx` and `Toasts.tsx` uses the correct cancelled-flag pattern.
  - Every object selector uses `useShallow`, so there are no "getSnapshot should be cached" problems.
  - `SourceMeter` isolates the 20 Hz level updates.
  - Clip reloads are ticketed.
  - Thumbnails use `loading="lazy"`.
  - No XSS sinks.
- **Tooling.** There is no ESLint config, no frontend test, and no pull-request CI.

## Findings (most severe first)

### 1. [High] [bug] The UI's copy of the config goes stale; `patchConfig` sends the whole stale config and ignores the core's corrected reply (confirmed)
- **Where:**
  - `src/store.ts:362-373`:
    ```ts
    const config = { ...get().config, ...patch }; set({ config }); ... await api.setConfig(config);
    ```
  - `src/lib/ipc.ts:47`: `setConfig: (config: AppConfig) => invoke<void>("set_config", { config })`
  - The core command returns `AppConfig` (`commands.rs:55-59`) and rewrites it:
    - When a recording is running, a source change is reverted and the core calls `crate::notify(&app, "error", "Stop the recording before changing the source.")` (`commands.rs:72-81`).
    - The encoder is replaced by `encode::resolve(...)` (`state.rs:367`).
    - Accent colours are cleaned.
  - The core also edits the config by itself and never tells the UI: `repair_screens()` (`lib.rs:472-507`, called on monitor changes at `lib.rs:542` and `:586`) rewrites `recording.target_id`, the overlay screen and the console screen. There is no `config-changed` event.
- **Failure scenarios:**
  - **(a)** During a recording the user picks another monitor. The core refuses, but the tile stays "Selected". Every later unrelated toggle (a sound, the watermark) sends that target again and shows the error toast again. Once the recording stops, the next unrelated toggle silently applies the change and restarts the buffer (`commands.rs:110-118`), discarding the buffered footage.
  - **(b)** The user plugs or unplugs a monitor, and `\\.\DISPLAYn` gets renumbered. The next unrelated setting change sends the old `targetId`. `target_changed` is true, so the replay buffer restarts and everything it held is lost.
  - **(c)** The user selects an encoder marked "(not available)" (the option is selectable, `Recording.tsx:353-356`). The UI and Dashboard keep showing it while the core runs a fallback.
- **Fix:**
  1. Type `set_config` as `invoke<AppConfig>` and apply the reply behind a version guard, the same ticket pattern used for `clips-changed` at `store.ts:301`:
     ```ts
     const v = ++cfgVersion; const next = await api.setConfig(config); if (v === cfgVersion) set({ config: next });
     ```
  2. Emit `config-changed` from the core after `repair_screens`, the tray hint and token changes, and listen for it in `init`.
  3. Better still, replace the whole-config write with `patch_config(patch)`, merged on the core side.

### 2. [High] [bug/UX] Sliders write every intermediate value; dragging the buffer slider discards footage (confirmed)
- **Where:**
  - `Recording.tsx:393-409`: `onChange={(seconds) => patchConfig({ buffer: { ...config.buffer, seconds, ... } })}`. The memory-budget slider at `:468-477` does the same.
  - The core applies each value at once: `state.rs:381-388` calls `shared.packets.lock().set_capacity(next.buffer.seconds, memory)`, and `buffer.rs:69-73` `set_capacity` calls `self.trim()`.
  - The same per-move pattern: `Sounds.tsx:257-263` and `:297-303` (100 steps), the console scale in `Console.tsx:229-236`, and the gain slider in `AudioMixer.tsx:384-391` (`upsertSource` on every move).
- **Failure scenarios:**
  - The buffer holds 10 minutes. The user drags the length slider down past 30 s and back to 5 min. The ring was trimmed to 30 s mid-drag and the footage is gone. The memory slider drops old groups of pictures the same way.
  - Each slider tick is a synchronous `set_config` on the Tauri main thread. It runs `replace_config`, which writes the config file (`config::save`), may spawn an audio-refresh thread that enumerates the process tree, and calls `apply_autostart` (registry read), `overlay::reposition` and `console::relayout`. That happens 30-100 times per drag.
  - With optimistic updates and last-reply-wins, the thumb can also jump back while dragging.
- **Fix:** Give `Slider` a local draft value and an `onCommit` callback fired on `pointerup`/`keyup` (or `change`), and write only on commit. Alternatively reuse the `SETTLE_MS` debounce that `Colors.tsx:29-34` already uses. On the core side, debounce `config::save`.

### 3. [High/Medium] [perf] The main window keeps rendering and receiving 20 Hz IPC while the user is gaming (code facts confirmed; magnitude likely, needs measuring)
- **Where:**
  - `tauri.conf.json:27` gives the main window `--disable-features=...CalculateNativeWinOcclusion --disable-backgrounding-occluded-windows --disable-renderer-backgrounding`. According to `overlay.rs:40-56` and the README these flags exist for the banner, which now has its own data dir and browser process, so the main window does not need them.
  - Levels: the core emits `audio-levels` to `main` at 20 Hz whenever the window is visible and not minimized (`lib.rs:964-967`), including while it sits behind the game. `store.ts:272` does `set({ levels })` for every page, even when no meter is on screen. Each `set` runs every subscriber's selector; the gallery alone has one per tile (`NameField`, `Clips.tsx:707`).
  - `NavBar.tsx:176` renders `<LiveDot />` whenever the buffer runs, on every page. It is an infinite animation (`motion.css:132-134`, `cb-live-ring 2s infinite`) inside a `backdropFilter: "blur(20px)"` pill (`NavBar.tsx:49`). The compositor produces frames continuously and likely re-blurs the backdrop each frame.
  - RGB accent mode: `accent.ts:157` runs `setInterval(refresh, 80)` and rewrites 16 root custom properties, which recalculates styles for the whole document (huge with a 1000-tile gallery). It is gated only by `!document.hidden`, which does not change when the window is merely occluded once occlusion detection is off.
- **Fix:**
  1. Drop the three occlusion/backgrounding flags from the main window.
  2. Add a "dormant" mode in the frontend driven by `onHidden`, `blur` and `focus` from `lib/hidden.ts`: set `html[data-dormant]` with `[data-dormant] [class*="cb-"]{animation-play-state:paused}` and stop the accent interval.
  3. Make levels opt-in: a ref-counted `api.watchLevels(on)` while a `Meter` is mounted, or a separate tiny levels store so the main store does not notify everyone 20 times a second.
  4. Throttle RGB mode to 2-4 Hz, or animate it with `filter: hue-rotate()` on the few accent surfaces.

### 4. [Medium] [perf] The clip gallery is unvirtualized and unmemoized, and each tile pays for an uncached `toLocaleDateString` (confirmed, measured)
- **Where:**
  - `format.ts:24`: `new Date(ts).toLocaleDateString("en-US", { day: "numeric", month: "long", year: "numeric" })`, called once per tile per render (`Clips.tsx:625`). In Node/V8 this took 63-81 ms per 1000 calls, against about 1 ms with a cached `Intl.DateTimeFormat` (bench at `<review-scratch>/bench.mjs`).
  - `visible` (`Clips.tsx:199-225`) and every tile are rebuilt inline on every render: each search keystroke, `clips-changed`, `links` change, rename or confirm state change.
  - Each tile carries 2-4 `Pill`s with `backdrop-blur-md` (`Card.tsx:38`), plus the heart and action buttons with `backdrop-blur-md` (`Clips.tsx:557`, `:1180`). Hundreds of backdrop-filter surfaces get re-blurred while scrolling.
  - On the first visit, every tile gets `cb-tile-in`, including off-screen ones.
- **Fix:**
  - Hoist `const DATE = new Intl.DateTimeFormat(...)` in `format.ts`.
  - Extract a `ClipTile` wrapped in `React.memo`, with stable callbacks keyed by id, and memoize `visible` with `useMemo([clips, query, filter])`.
  - Add `content-visibility:auto; contain-intrinsic-size:auto 300px` to tiles as a cheap win, or virtualize rows (for example `@tanstack/react-virtual`). `useFlip` would then animate only mounted rows.
  - Replace `backdrop-blur` on tile chrome with solid `bg-black/60`.
  - Pass `updateClip` into `NameField` instead of subscribing per tile.

### 5. [Medium] [bug] The mixer source label saves on every keystroke, and a stale reply can drop typed characters (confirmed; character loss likely under load)
- **Where:** `AudioMixer.tsx:333-337`: `onChange={(e) => upsertSource({ ...source, label: e.target.value })}`. `store.ts:418-423` then does `set({ config: await api.updateAudioSource(source) })` unconditionally.
- **Failure scenario:** Each keystroke means one IPC call, `replace_config`, a config file write, and an audio-refresh thread. If a reply arrives after the next keystroke (the main thread busy with a file dialog or a slow disk), the store snaps the label back to the older value. The controlled input jumps and the next keystroke is appended to the stale text, so characters are lost. The same last-reply-wins problem affects gain, mute and solo.
- **Fix:** Keep a draft label and commit on blur/Enter, as `NameField` already does (`Clips.tsx:697-771`). Add the version guard from finding 1 to `upsertSource`/`removeSource`.

### 6. [Medium] [bug] Mock data is the real store's initial config and its fallback in Tauri mode (code path confirmed; failure unlikely but damaging)
- **Where:** `store.ts:164` has `config: mock.mockConfig`, and `store.ts:263` has `load("Reading the settings", api.getConfig, get().config)`.
- **Failure scenarios:**
  - Until `getConfig` returns, the Dashboard shows fake values ("3 audio sources", "1080p60 · NVENC").
  - If `get_config` fails, the app marks itself `ready` with the mock config. The next `patchConfig` then writes fake PIDs as audio sources, `clipDir` `C:\Users\user\Videos\ClippiBoy`, the mock hotkeys and `token: "mock-token"` into the user's real config. The core keeps a token only if it arrives empty, so the Stream Deck stops working.
- **Fix:** Use `config: AppConfig | null` (or a `loaded` flag), don't render config-dependent routes until it is loaded, never fall back to mock in Tauri, and make `patchConfig` refuse to run before load.

### 7. [Medium] [UX] Errors only show at the bottom of Settings and are never cleared (confirmed)
- **Where:** Every failing action calls `set({ lastError })`: `updateClip`, `setFavorite`, `setClipTags`, `patchConfig`, `clearGame`, `toggleBuffer`, `refreshTargets` and others. The only place it is shown is `settings/index.tsx:80-84`. There is no `lastError: null` anywhere after the initial state (`store.ts:188`).
- **Failure scenarios:** A failed rename or heart in the gallery gives no feedback. A startup error such as "Reading applications failed" sits under every Settings tab forever.
- **Fix:** Add a store-level `notify(kind, msg)` that feeds `Toasts`, which should also get `role="status"`/`aria-live`. Make `lastError` dismissible, or drop it.

### 8. [Medium] [bug] The Dashboard player tracks a position in a live list and never files clips (confirmed)
- **Where:** `Dashboard.tsx:80-83` derives `recent = clips.filter(...).slice(0, 3)`, and `:271-279` passes `index={Math.min(playing, recent.length - 1)}` with `onClose={() => setPlaying(null)}`.
- **Failure scenarios:**
  - The user is watching a recent clip when a new one arrives (hotkey, console, a friend's clip). `clip-saved` prepends it, so the same index now points at a different clip and the player switches mid-playback. `Clips.tsx:344-376` handles exactly this case with `shownId`, but the Dashboard does not.
  - Favorites or game edits made in the Dashboard player never call `fileClip`, so files stay in the wrong folder until the next start.
- **Fix:** Extract the gallery's playlist logic (`playlist`, `shownId`, `coming`, `touched`, `deletedHere`) into a `usePlayerSession()` hook and use it in both places.

### 9. [Medium] [bug/perf] Clearing a game makes one IPC write per clip, and the change events undo the optimistic update mid-loop (likely)
- **Where:** `store.ts:570-578` loops `await api.updateClip(...)` then `await get().fileClip(...)`. Each `update_clip` call runs `announce_change`, which emits `clips-changed` (`commands.rs:941`). The listener at `store.ts:302-310` then re-reads the full clip list.
- **Failure scenario:** Removing a game with 200 clips means 200 writes, 200 full-list IPC reads and gallery re-renders. Re-reads restore the game on clips not yet processed, so the chip comes back and counts down. A failure partway through leaves the UI out of sync with no rollback.
- **Fix:** Add a single `clear_game(game)` core command that runs in one transaction and announces once.

### 10. [Medium] [a11y] Keyboard and screen-reader gaps (confirmed unless noted)
- **Hotkey recorder:** recording starts only on `onMouseDown` (`settings/Hotkeys.tsx:329-333`); Enter/Space fire `click`, so keyboard users cannot assign hotkeys. Add an `onClick`/`onKeyDown` path. The recorder button and the "Unbind" buttons are not tied to their row labels.
- **Unnamed toggles:** 24 of 33 `<Toggle>`s have no `label`, because `Row` renders its label in a `<p>` (`settings/shared.tsx:54-71`; for example `Behaviour.tsx:226`, Banner, Console, Friends, Sounds, `QuickSetup` at `AudioMixer.tsx:578`). Add a `ToggleRow` that wires `aria-labelledby`.
- **Unnamed selects:** Resolution, Frame rate, Quality, Encoder and Keyframe interval (`Recording.tsx:292-372`) have no `label`.
- **Mixer buttons:** the M/S/⧉ `MiniToggle`s (`AudioMixer.tsx:608-633`) have no `aria-pressed`/`aria-label`; ⧉ only has a `title`. The label `<input>` (`:333`) is unnamed.
- **Hidden tile actions:** the heart, folder and delete buttons use `opacity-0 group-hover:opacity-100` (`Clips.tsx:560`, `:572`) with no `focus-visible`/`group-focus-within` reveal, so keyboard focus lands on invisible buttons.
- **Clickable divs:** Dashboard clip cards and Stat cards (`Dashboard.tsx:227-231`, `:300-303`) are `div onClick` with no role, `tabIndex` or keyboard handling.
- **Focus loss on confirm:** clicking delete swaps the trigger for `ConfirmDelete`, so focus falls to `<body>`. Move focus to its Cancel button (the safe default).
- **Menu position:** a menu opened by keyboard-activating "More" (`Friends.tsx:448`) gets `clientX/Y = 0` and appears top-left. Use the button's rect when `event.detail === 0`.
- **Menu keyboard:** no Home/End/Tab handling and no `aria-activedescendant`.
- **Missing ARIA state:** `Segmented` has no `aria-pressed` or radiogroup semantics; the NavBar has no `aria-current="page"`; Toasts have no live region.
- **Contrast:** `--color-ink-faint: #5c5c68` measures 2.84:1 on surface, 2.63:1 on elevated and 3.04:1 on base, failing WCAG AA. It is used 83 times, mostly for small hint text. `#80808c` gives 4.79:1 and 4.44:1.
- **Reduced motion:** `motion.css:377` covers only `cb-*` animations. Tailwind `transition-*`, `SlidingIndicator` and `animate-pulse` (`Friends.tsx:164`) ignore it. Add `motion-reduce:transition-none` or a global rule.
- **Keyboard reorder (likely):** `nudge` with ArrowDown makes React move the focused row's DOM node (`AudioMixer.tsx:201-208`), so focus is lost after one step. Re-focus the grip after the move.

### 11. [Medium] [quality] No lint, no frontend tests, no pull-request CI (confirmed)
- **State today:**
  - There is no ESLint config, yet five `eslint-disable-next-line react-hooks/exhaustive-deps` comments exist (`ClipPlayer.tsx:210`, `Setup.tsx:128`, `Share.tsx:295`, `ShotViewer.tsx:362`, `:414`).
  - `.github/workflows` only has release, ffmpeg and pages; nothing runs `tsc` on pull requests.
  - `tsconfig` lacks `noUncheckedIndexedAccess`. Enabling it gives 51 errors; the ones in my slice are `Menu.tsx:166`, `:175`, `AudioMixer.tsx:184`, `:206`, `Clips.tsx:519`, `Friends.tsx:107` and `Recording.tsx:37`.
  - `vite.config.ts` is not type-checked.
- **Minimal setup:**
  - **Lint:** `eslint`, `typescript-eslint`, `eslint-plugin-react-hooks` (recommended-latest) and `eslint-plugin-jsx-a11y` in a flat config. Enable `@typescript-eslint/no-floating-promises`; it would flag `upsertSource(s);` (`AudioMixer.tsx:242`) and `inTauri && api.revealClip()` (`Clips.tsx:602`).
  - **Tests:** `vitest` with `jsdom`, `@testing-library/react`, and `mockIPC`/`clearMocks` from `@tauri-apps/api/mocks` for store tests.
  - **Pure functions to test first:** `format.ts`; Hotkeys `accelerator`/`risky`/`supported`; Recording `fit`/`fpsChoices`/`memoryBudgetMb` (move them into `lib/`); `screenChoice`; `slotFor`/`usePresence`; `timeLeft`; `lastSeenText`.
  - **CI:** a `ci.yml` on `pull_request` that runs `npm ci && tsc --noEmit && eslint . && vitest run`, plus `cargo test` on Windows.
  - **Scripts:** `typecheck`, `lint`, `test`. Turn on `noUncheckedIndexedAccess` and `noImplicitReturns`.

### 12. [Medium] [quality] Big components and inverted dependencies
- **Size:** `Clips.tsx` has 1187 lines. Split it into `ClipTile` (memo), `FilterBar` (`GameFilters`, `Chip`, `ConfirmChip`), `NameField`, `SearchField`, a `useGalleryFilter` hook (counts and visible list) and `usePlayerSession`. `AudioMixer.tsx` (866 lines): split into `SourceRow` (memo), `useSourceDrag`, and move `QuickSetup`/`AddSourcePanel` out.
- **Layering:** components import from routes: `Setup.tsx:11-14` takes `TargetCard`, `QuickSetup`, `Hotkeys` and `Row`, and `Share.tsx:11` takes `Avatar`. Move these into `components/`.
- **Duplication:** `Row` exists twice (`Recording.tsx:492` and `settings/shared.tsx:54`).
- **Odd pattern:** `report()` in `routes/Friends.tsx:14-17` sends errors through a window `CustomEvent`.

### 13. [Low] [bug] `init()` is not idempotent (confirmed; dev only)
- `App.tsx:31-33` calls `void init()` in an effect, and StrictMode runs it twice in dev. Every `listen()` in `store.ts:272-332` registers twice and is never removed: events are handled twice and `clips-changed` triggers two reads. In browser mode there are two 80 ms intervals (`store.ts:228`). Vite HMR repeats this for the module-level listeners in `friends.ts`, `share.ts`, `links.ts` and `hidden.ts`.
- **Fix:** a module-level `initPromise ??= ...` guard, and keep the unlisten handles.

### 14. [Low] [bug] Startup snapshots can overwrite newer pushes (likely)
- `friends.ts:42-43` and `share.ts:40-41` register `listen` and fire the initial `invoke` in parallel. If a push arrives before the snapshot reply, the older snapshot wins (`setState(view, true)`).
- `useLinks.refresh` (`links.ts:63`) has no ticket and can race itself.
- **Fix:** ignore the snapshot if an event has already arrived, or add a version field.

### 15. [Low] [bug] Text-field menu "Paste" into number inputs prepends and throws (confirmed by DOM semantics)
- `dom.ts:48` includes `"number"` in the text types. In `TextMenu.tsx:31-37`, `selectionStart` is `null` (treated as 0), so the pasted text is prepended, and then `setSelectionRange` throws `InvalidStateError` (an unhandled rejection from the async `onSelect`).
- This affects the RGB fields in `ColorPicker.tsx:222`. Paste also bypasses `maxLength`.
- **Fix:** handle number fields separately (replace the whole value) and wrap the call in try/catch.

### 16. [Low] [bug] Record and buffer buttons have no in-flight guard (likely)
- `store.ts:454-471` and `:432-442`. Double-clicking "Record" either gives a spurious "A recording is already running." (from the core's lifecycle lock in `state.rs:975`) or stops the just-started recording. The optimistic buffer flip can flicker until the next 1 Hz status.
- **Fix:** add `recordingBusy`/`bufferBusy` state and disable the buttons while a call is in flight.

### 17. [Low] [UX] The page scroll position carries over between routes (confirmed)
- `<main className="... overflow-y-auto">` (`App.tsx:65`) keeps its `scrollTop` across the keyed route swap; nothing resets it. Leaving a scrolled gallery opens Settings scrolled down, and coming back to the gallery loses its own scroll position.
- **Fix:** reset or restore the scroll position per route.

### 18. [Low] [quality] IPC contract drift and dead code (confirmed)
- `ipc.ts:150-151`: `screenshotHasOriginal` invokes `screenshot_has_original`, which is not registered in `lib.rs:1283-1365`. It is unused.
- `setConfig` is typed `void` but the command returns a config (finding 1).
- The `ipc.ts:53-54` doc says "All four at once" for five hotkeys.
- The `store.ts:490` comment about saving on every keystroke is outdated; `ClipMeta` saves on blur.
- `EngineStatus` fields `droppedFrames`, `fps`, `freeBytes`, `gameSeconds` and `encoder` are dropped by `store.ts:273-285` and never shown.
- Everything else in my slice matches `model.rs`/`commands.rs`, including camelCase argument names (`startMs`, `targetBytes`, `clipId`) and serde's `accent_color_2` → `accentColor2`. `mock.ts` is typed, so `tsc` keeps it in sync.

### 19. [Low] [security] Lockdown review and least privilege
- **What exists:**
  - No XSS sinks: no `dangerouslySetInnerHTML`, `innerHTML`, `href` or `window.open` in `src`.
  - Friend names, statuses, game names and transfer names render as text.
  - Avatar `<img src>` is limited by CSP `img-src` to `cdn.discordapp.com`.
  - Share-link URLs are never rendered.
- **What `lockdown.ts` blocks, and how well:**
  - It matches on physical `event.code`, so on QWERTZ keyboards Ctrl+`+`/Ctrl+`-` (codes `BracketRight` and `Slash`) are not caught. It relies on Tauri's own zoom disable.
  - Devtools are absent in release builds anyway (no `devtools` cargo feature).
  - Context menu and drop handling are effective (`dragDropEnabled:false` plus `preventDefault`).
  - **Recommendation (unverified):** WebView2 `AreBrowserAcceleratorKeysEnabled=false` would be the robust lock. Check whether your wry/Tauri version exposes it.
- **Least privilege:** `capabilities/default.json:16,19` grant `opener:default` (lets page JavaScript open any http(s) URL) and `global-shortcut:default`. The frontend uses neither; the npm deps `@tauri-apps/plugin-opener` and `plugin-global-shortcut` are unused too. Remove them.

### 20. [Low] [perf] Smaller performance items
- `Menu.tsx:143-192` re-subscribes five window listeners on every render, because the effect depends on `selectable` and `run`, which are new each render.
- `Meter` (`Controls.tsx:163-197`) keeps a 60 ms `setInterval` per meter forever and animates `width` (layout). Use a ref plus `transform: scaleX` driven by rAF.
- `SlidingIndicator` transitions `width`.
- `accent.ts:142` sends the 64 KB tray icon as a JSON number array (`Array.from(pixels)`); send it as binary.

### 21. [Low] [privacy/UX] The usage-stats notice marks itself as read
- `UsageNotice.tsx:40-41` sets `usageStatsNoticed: true` after 10 s of focused display, and stats default to on (`model.rs` `usage_stats` default `yes`).
- Consider requiring an explicit "Got it" or a choice.

## Missing features / UX opportunities
- **Gallery:**
  - Sorting by date, size, duration or name, and date grouping.
  - Multi-select with bulk delete, tag, move or export.
  - Arrow-key navigation through the grid, with Enter to open and Delete to remove.
  - Ctrl+F to focus search (lockdown currently swallows Ctrl+F) and a `#tag` search syntax.
  - Keep filter and scroll position across route switches.
  - Delete to the Recycle Bin: `Library::delete` uses `remove_file` and ignores its errors, so a locked file leaves an orphan on disk.
  - Drag a clip out into Discord or Explorer.
  - Storage auto-cleanup (keep N GB, never delete favorites).
- **Health panel:** show dropped frames, the real encoder, free disk space and the game session (all already in `EngineStatus`).
- **Notification history:** an error center, since toasts disappear after 3-6 s.
- **Recording page:** an "Auto" choice for the memory budget (once dragged it can't go back to 0); disable unavailable encoders; show estimated RAM use.
- **Mixer:** double-click to reset gain to 0 dB, numeric gain entry, peak/clip indicators.
- **Settings:** search, reset to defaults, import/export.
- **Dates:** use `navigator.language` instead of hard-coded `en-US`.
- **Dormant mode:** pause animations and level traffic while the window is unfocused or hidden.
