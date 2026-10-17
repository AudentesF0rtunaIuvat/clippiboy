# Frontend core: store, routes, settings, shared UI

**Scope:** main.tsx, App.tsx, store.ts, src/lib/*, routes/* and settings/*, components/ui/*, Toasts, NavBar, TitleBar, TextMenu.

**Findings:** 46 — 39 verified (3 medium or higher), 5 reported but not independently re-verified, 1 unverifiable/disputed, 1 rejected by verification.

Severity shown is the verified severity; where verification changed it, the reported one is in parentheses. See [README](README.md#how-findings-were-verified) for how verification worked.

## Verified issues (medium and above)

### FC-1 · Medium (reported High) · UI config copy goes stale: patchConfig sends the whole stale config and ignores the core's corrected reply

- **Where:** `src/store.ts:362-373`, `src/lib/ipc.ts:47`, `src-tauri/src/commands.rs:55-59`, `src-tauri/src/commands.rs:72-81`, `src-tauri/src/state.rs:367`, `src-tauri/src/lib.rs:472-507` (+4 more)
- **Verification:** confirmed (impact: confirmed; code: confirmed; reachability: confirmed)

**Problem:** patchConfig in src/store.ts:362-373 does `const config = { ...get().config, ...patch }; set({ config }); ... await api.setConfig(config);`, and src/lib/ipc.ts:47 types it as `setConfig: (config: AppConfig) => invoke<void>("set_config", { config })` (also listed under the report's IPC-contract-drift finding: setConfig is typed void but the command returns a config). The core command actually returns AppConfig (commands.rs:55-59) after rewriting it: while a recording runs, a source change is reverted and the core calls `crate::notify(&app, "error", "Stop the recording before changing the source.")` (commands.rs:72-81); the encoder is replaced by `encode::resolve(...)` (state.rs:367); accent colours are cleaned. The UI discards that reply and keeps (and later resends) its own uncorrected whole config. The core also edits the config on its own and never tells the UI: `repair_screens()` (lib.rs:472-507, called on monitor changes at lib.rs:542 and :586) rewrites `recording.target_id`, the overlay screen and the console screen; there is no `config-changed` event.

**Scenario:** (a) During a recording the user picks another monitor; the core refuses but the tile stays "Selected". Every later unrelated toggle (a sound, the watermark) resends that target and shows the error toast again; once the recording stops, the next unrelated toggle silently applies the change and restarts the buffer (commands.rs:110-118), discarding buffered footage. (b) The user plugs/unplugs a monitor and `\\.\DISPLAYn` is renumbered; the next unrelated setting change sends the old `targetId`, `target_changed` is true, the replay buffer restarts and everything it held is lost. (c) The user selects an encoder marked "(not available)" (selectable, Recording.tsx:353-356); UI and Dashboard keep showing it while the core runs a fallback.

**Fix:** 1) Type `set_config` as `invoke<AppConfig>` and apply the reply behind a version guard (same ticket pattern as `clips-changed` at store.ts:301): `const v = ++cfgVersion; const next = await api.setConfig(config); if (v === cfgVersion) set({ config: next });`. 2) Emit `config-changed` from the core after `repair_screens`, the tray hint and token changes, and listen for it in `init`. 3) Better: replace the whole-config write with a core-side merged `patch_config(patch)`.

**Verifier on the fix:** Fix 1 (type set_config as AppConfig and apply the reply behind a version ticket) is correct and fixes (a) and (c); fix 2 is needed for (b), but the tray-hint and token parts are already guarded in the core (commands.rs:66, :82). A smaller core-only fix for the buffer loss in (b): in set_config, when the incoming target_stable_id equals previous.target_stable_id (and is Some), keep previous.target_id before computing target_changed (or run capture::repair on the incoming recording/overlay/console choice only when the device name differs). That way a stale page cannot trigger a restart, without…

### FC-10 · Medium · Hotkey recorder cannot be started from the keyboard; recorder/Unbind buttons not tied to row labels

- **Where:** `src/routes/settings/Hotkeys.tsx:329-333`
- **Verification:** confirmed (code: confirmed; reachability: confirmed)

**Problem:** In settings/Hotkeys.tsx:329-333 recording starts only on `onMouseDown`; Enter/Space fire `click`, not mousedown, so keyboard users cannot assign hotkeys. The recorder button and the "Unbind" buttons are not associated with their row labels.

**Scenario:** A keyboard-only user tabs to a hotkey recorder and presses Enter or Space; nothing starts recording, so they cannot assign hotkeys. Screen readers announce the buttons without the row's label.

**Fix:** Add an `onClick`/`onKeyDown` path to start recording, and tie the recorder and Unbind buttons to their row labels.

**Verifier on the fix:** Do not simply add onClick next to the existing onMouseDown toggle: a mouse click would fire both and toggle twice. Move the toggle to onClick, keep onMouseDown only for stopPropagation (so the window mousedown listener at Hotkeys.tsx:310 does not end recording), and give the recorder and Unbind buttons names that include the row label (e.g. aria-label=`Unbind ${label}` / aria-labelledby the Row label id).

### FC-3 · Medium (reported High) · Main window keeps rendering and receiving 20 Hz IPC while the user is gaming

- **Where:** `src-tauri/tauri.conf.json:27`, `src-tauri/src/overlay.rs:40-56`, `src-tauri/src/lib.rs:964-967`, `src/store.ts:272`, `src/routes/Clips.tsx:707`, `src/components/NavBar.tsx:176` (+3 more)
- **Verification:** partial (reachability: partly confirmed; impact: partly confirmed; code: confirmed)

**What verification found:** The code facts are correct and the situation happens in ordinary use. When the main window is left open (not closed to tray, not minimized) behind a game, the core still sends 20 Hz audio-levels to it, because window_awake only checks visible/minimized and not covered. The renderer also never learns it is covered, because CalculateNativeWinOcclusion and backgrounding are turned off for the main window, which no longer needs those flags now that banner and console use their own data dir. So the NavBar LiveDot ring keeps the compositor making frames at the display rate, and in opt-in RGB mode the root custom properties are rewritten every 80 ms, forcing a style recalc of the whole document. The cost is overstated, though. The per-tile NameField selector is a trivial stable-function lookup that never re-renders, so the 20 Hz IPC adds only an event parse and a few thousand no-op selector calls per second. Re-running the backdrop blur every frame is plausible but unproven (in RGB mode the hero gradient behind the pill changes, so there it does re-blur). The only CPU cost that matters is RGB mode on a large, non-virtualised gallery, and RGB is not the default. Medium/low perf issue, not high.

**Original claim:** tauri.conf.json:27 gives the main window `--disable-features=...CalculateNativeWinOcclusion --disable-backgrounding-occluded-windows --disable-renderer-backgrounding`; per overlay.rs:40-56 and the README these flags exist for the banner, which now has its own data dir and browser process. The core emits `audio-levels` to `main` at 20 Hz whenever the window is visible and not minimized (lib.rs:964-967), including when behind the game, and store.ts:272 does `set({ levels })` on every page even with no meter on screen; each `set` runs every subscriber's selector (the gallery has one per tile, `NameField`, Clips.tsx:707). NavBar.tsx:176 renders `<LiveDot />` whenever the buffer runs, an infinite animation (motion.css:132-134, `cb-live-ring 2s infinite`) inside a `backdropFilter: "blur(20px)"` pill (NavBar.tsx:49), so the compositor produces frames continuously and likely re-blurs each frame. In RGB accent mode accent.ts:157 runs `setInterval(refresh, 80)` rewriting 16 root custom properties (whole-document style recalc), gated only by `!document.hidden`, which does not change when the window is merely occluded once occlusion detection is off.

**Scenario:** While the user is gaming with ClippiBoy's main window open behind the game, the main window keeps animating, re-blurring, recalculating styles every 80 ms in RGB mode (huge with a 1000-tile gallery) and processing 20 Hz level IPC that notifies every store subscriber, costing CPU/GPU during gameplay. Code facts confirmed; magnitude needs measuring.

**Fix:** 1) Drop the three occlusion/backgrounding flags from the main window. 2) Add a frontend "dormant" mode driven by `onHidden`, `blur`, `focus` from lib/hidden.ts: set `html[data-dormant]` with `[data-dormant] [class*="cb-"]{animation-play-state:paused}` and stop the accent interval. 3) Make levels opt-in (ref-counted `api.watchLevels(on)` while a `Meter` is mounted, or a separate tiny levels store). 4) Throttle RGB mode to 2-4 Hz or animate it with `filter: hue-rotate()` on the few accent surfaces.

**Verifier on the fix:** Step 1 alone is the right minimal fix. Change main's additionalBrowserArgs to "--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection" (keep wry's defaults) and drop CalculateNativeWinOcclusion and both backgrounding switches. WebView2 then marks the page hidden when it is covered, minimized or in the tray, which stops animation frames and makes the existing document.hidden gate in accent.ts work as its docstring intends. This looks safe: the frontend does no network work (no fetch/WebSocket in src; that runs in Rust), its timers are 30 s and up or UI-only, and hidden.ts already pause…

## Reported, not independently re-verified

The original reviewer checked these against the code, but the independent verification pass was stopped before reaching them. Treat as likely, confirm before fixing.

- **FC-4 · Medium · Clip gallery is unvirtualized and unmemoized; each tile pays for an uncached toLocaleDateString** — `src/lib/format.ts:24`, `src/routes/Clips.tsx:625`, `src/routes/Clips.tsx:199-225` (+3 more)  
  format.ts:24 does `new Date(ts).toLocaleDateString("en-US", { day: "numeric", month: "long", year: "numeric" })` once per tile per render (Clips.tsx:625); measured in Node/V8 at 63-81 ms per 1000 calls versus about 1 ms with a cached `Intl.DateTimeFormat`. `visible` (Clips.tsx:199-225) and every tile are rebuilt inline on every render (each search keystroke, `clips-changed`, `links` change, rename or confirm-state change). Each tile carries 2-4 `Pill`s with `backdrop-blur-md` (Card.tsx:38) plus heart/action buttons with `backdrop-blur-md` (Clips.tsx:557, :1180), so hundreds of backdrop-filter…  
  *Fix:* Hoist `const DATE = new Intl.DateTimeFormat(...)` in format.ts; extract a `ClipTile` wrapped in `React.memo` with stable id-keyed callbacks and memoize `visible` with `useMemo([clips, query, filter])`; add `content-visibility:auto; contain-intrinsic-size:auto 300px` to tiles or virtualize rows (e.g. `@tanstack/react-virtual`), so `useFlip` animate…
- **FC-5 · Medium · Mixer source label saves on every keystroke; a stale reply can drop typed characters** — `src/routes/AudioMixer.tsx:333-337`, `src/store.ts:418-423`  
  AudioMixer.tsx:333-337 has `onChange={(e) => upsertSource({ ...source, label: e.target.value })}`, and store.ts:418-423 then does `set({ config: await api.updateAudioSource(source) })` unconditionally (last reply wins, no version guard). The same last-reply-wins problem affects gain, mute and solo.  
  *Fix:* Keep a draft label and commit on blur/Enter, as `NameField` already does (Clips.tsx:697-771). Add the version guard from FC-1 to `upsertSource`/`removeSource`.
- **FC-6 · Medium · Mock data is the real store's initial config and its fallback in Tauri mode** — `src/store.ts:164`, `src/store.ts:263`, `src/store.ts:362-368` (+3 more)  
  store.ts:164 initialises the store with `config: mock.mockConfig`, and store.ts:263 loads with `load("Reading the settings", api.getConfig, get().config)`, i.e. the mock config is the fallback if `get_config` fails, also in Tauri mode.  
  *Fix:* Use `config: AppConfig \| null` (or a `loaded` flag), don't render config-dependent routes until loaded, never fall back to mock in Tauri, and make `patchConfig` refuse to run before load.
- **FC-7 · Medium · Errors only show at the bottom of Settings and are never cleared** — `src/routes/settings/index.tsx:80-84`, `src/store.ts:188`, `src/components/ShotViewer.tsx:639-641` (+1 more)  
  Every failing store action calls `set({ lastError })` (`updateClip`, `setFavorite`, `setClipTags`, `patchConfig`, `clearGame`, `toggleBuffer`, `refreshTargets` and others), but `lastError` is only displayed in settings/index.tsx:80-84, and nothing ever sets `lastError: null` after the initial state (store.ts:188).  
  *Fix:* Add a store-level `notify(kind, msg)` that feeds `Toasts` (which should also get `role="status"`/`aria-live`). Make `lastError` dismissible, or drop it.
- **FC-9 · Medium · Clearing a game makes one IPC write per clip and change events undo the optimistic update mid-loop** — `src/store.ts:570-578`, `src-tauri/src/commands.rs:941`, `src/store.ts:302-310`  
  `clearGame` in store.ts:570-578 loops `await api.updateClip(...)` then `await get().fileClip(...)` per clip. Each `update_clip` runs `announce_change`, which emits `clips-changed` (commands.rs:941), and the listener at store.ts:302-310 re-reads the full clip list each time.  
  *Fix:* Add a single `clear_game(game)` core command that runs in one transaction and announces once.

## Verified lower-severity issues

| ID | Severity | Title | Where | Fix |
|---|---|---|---|---|
| FC-11 | Low (reported Medium) | Most Toggles have no accessible name | src/routes/settings/shared.tsx:54-71, src/routes/settings/Behaviour.tsx:226 | Add a `ToggleRow` that wires `aria-labelledby` to the row label. |
| FC-12 | Low (reported Medium) | Recording page selects have no labels | src/routes/Recording.tsx:292-372 | Give each select an accessible label. |
| FC-13 | Low (reported Medium) | Mixer M/S/⧉ buttons and label input are unnamed and lack state | src/routes/AudioMixer.tsx:608-633, src/routes/AudioMixer.tsx:333 | Add `aria-pressed` and `aria-label` to the MiniToggles and a label to the input. |
| FC-14 | Low (reported Medium) | Gallery tile actions stay invisible when keyboard-focused | src/routes/Clips.tsx:560, src/routes/Clips.tsx:572 | Reveal the actions on `focus-visible` / `group-focus-within`. |
| FC-15 | Low (reported Medium) | Dashboard clip cards and Stat cards are clickable divs without keyboard support | src/routes/Dashboard.tsx:227-231, src/routes/Dashboard.tsx:300-303 | Use buttons/links, or add role, tabIndex and key handling. |
| FC-16 | Low (reported Medium) | Focus is lost to <body> when delete is swapped for ConfirmDelete | src/components/ui/ConfirmDelete.tsx | Move focus to ConfirmDelete's Cancel button (the safe default). |
| FC-17 | Low (reported Medium) | Menu opened by keyboard on Friends "More" appears top-left | src/routes/Friends.tsx:448 | Use the button's bounding rect when `event.detail === 0`. |
| FC-18 | Low (reported Medium) | Menu lacks Home/End/Tab handling and aria-activedescendant | src/components/ui/Menu.tsx | Add Home/End/Tab handling and `aria-activedescendant`. |
| FC-19 | Low (reported Medium) | Missing ARIA state on Segmented, NavBar and Toasts | src/components/ui/Controls.tsx, src/components/NavBar.tsx | Add radiogroup/`aria-pressed` semantics to Segmented, `aria-current="page"` to the active NavBar item and an `aria-live` region for Toasts. |
| FC-2 | Low (reported High) | Sliders write every intermediate value; dragging the buffer slider discards footage | src/routes/Recording.tsx:393-409, src/routes/Recording.tsx:468-477 | Give `Slider` a local draft value and an `onCommit` callback fired on `pointerup`/`keyup` (or `change`) and write only on commit; alternatively reuse the `SETTLE_MS` debounce from Colors.tsx:29-34. On the core side, deb… |
| FC-20 | Low (reported Medium) | --color-ink-faint fails WCAG AA contrast | src/styles/tokens.css | Use `#80808c`, which gives 4.79:1 and 4.44:1. |
| FC-21 | Low (reported Medium) | Reduced-motion rule covers only cb-* animations | src/styles/motion.css:377, src/routes/Friends.tsx:164 | Add `motion-reduce:transition-none` or a global reduced-motion rule. |
| FC-23 | Low (reported Medium) | No ESLint config although eslint-disable comments exist; floating promises go undetected | src/components/ClipPlayer.tsx:210, src/components/Setup.tsx:128 | Add a flat ESLint config with `eslint`, `typescript-eslint`, `eslint-plugin-react-hooks` (recommended-latest) and `eslint-plugin-jsx-a11y`, and enable `@typescript-eslint/no-floating-promises`. |
| FC-24 | Low (reported Medium) | No frontend tests | package.json | Add `vitest` with `jsdom`, `@testing-library/react`, and `mockIPC`/`clearMocks` from `@tauri-apps/api/mocks` for store tests; test the listed pure functions first (move Recording helpers into `lib/`). |
| FC-25 | Low (reported Medium) | No pull-request CI: nothing runs tsc on PRs | .github/workflows/release.yml, .github/workflows/ffmpeg.yml | Add a `ci.yml` on `pull_request` running `npm ci && tsc --noEmit && eslint . && vitest run`, plus `cargo test` on Windows. |
| FC-28 | Low (reported Medium) | Oversized components: Clips.tsx (1187 lines) and AudioMixer.tsx (866 lines) | src/routes/Clips.tsx, src/routes/AudioMixer.tsx | Split Clips.tsx into `ClipTile` (memo), `FilterBar` (`GameFilters`, `Chip`, `ConfirmChip`), `NameField`, `SearchField`, a `useGalleryFilter` hook (counts and visible list) and `usePlayerSession`. Split AudioMixer.tsx in… |
| FC-29 | Low (reported Medium) | Inverted dependencies: components import from routes | src/components/Setup.tsx:11-14, src/components/Share.tsx:11 | Move these into `components/`. |
| FC-30 | Low (reported Medium) | Row component exists twice | src/routes/Recording.tsx:492, src/routes/settings/shared.tsx:54 | Keep a single shared Row in components/. |
| FC-33 | Low | Startup snapshots in friends/share stores can overwrite newer pushes | src/lib/friends.ts:42-43, src/lib/share.ts:40-41 | Ignore the snapshot if an event has already arrived, or add a version field. |
| FC-34 | Low | useLinks.refresh has no ticket and can race itself | src/lib/links.ts:63 | Add a ticket/version guard to refresh. |
| FC-35 | Low | Text-field menu Paste into number inputs prepends and throws | src/lib/dom.ts:48, src/components/TextMenu.tsx:31-37 | Handle number fields separately (replace the whole value) and wrap the call in try/catch. |
| FC-36 | Low | Record and buffer buttons have no in-flight guard | src/store.ts:454-471, src/store.ts:432-442 | Add `recordingBusy`/`bufferBusy` state and disable the buttons while a call is in flight. |
| FC-37 | Low | Page scroll position carries over between routes | src/App.tsx:65 | Reset or restore the scroll position per route. |
| FC-50 | Low | Memory-budget slider cannot return to automatic (0) once dragged | src/routes/Recording.tsx:468-477 | Add an "Auto" choice for the memory budget. |
| FC-26 | Info (reported Medium) | tsconfig lacks noUncheckedIndexedAccess (51 errors when enabled) | tsconfig.json, src/components/ui/Menu.tsx:166 | Turn on `noUncheckedIndexedAccess` and `noImplicitReturns` and fix the reported errors. |
| FC-27 | Info (reported Medium) | vite.config.ts is not type-checked | vite.config.ts | Include vite.config.ts in a type-checked tsconfig (e.g. a node tsconfig) and run it in a `typecheck` script. |
| FC-31 | Info (reported Medium) | Friends route reports errors through a window CustomEvent | src/routes/Friends.tsx:14-17 | Route errors through a store-level notify function. |
| FC-32 | Info (reported Low) | init() is not idempotent; listeners register twice and are never removed | src/App.tsx:31-33, src/store.ts:272-332 | A module-level `initPromise ??= ...` guard, and keep the unlisten handles. |
| FC-38 | Info (reported Low) | screenshotHasOriginal invokes an unregistered command (dead code) | src/lib/ipc.ts:150-151, src-tauri/src/lib.rs:1283-1365 | Remove the wrapper or register the command. |
| FC-39 | Info (reported Low) | ipc.ts hotkey doc says "All four at once" for five hotkeys | src/lib/ipc.ts:53-54 | Correct the comment. |
| FC-40 | Info (reported Low) | Outdated comment in store.ts about saving on every keystroke | src/store.ts:490 | Update or remove the comment. |
| FC-42 | Info (reported Low) | lockdown.ts matches physical event.code, missing zoom shortcuts on QWERTZ | src/lib/lockdown.ts | Match on `event.key` as well; (unverified) WebView2 `AreBrowserAcceleratorKeysEnabled=false` would be the robust lock if the wry/Tauri version exposes it. |
| FC-44 | Info (reported Low) | Menu re-subscribes five window listeners on every render | src/components/ui/Menu.tsx:143-192 | Memoize `selectable`/`run` or read them through refs so the effect subscribes once. |
| FC-45 | Info (reported Low) | Meter keeps a 60 ms interval forever and animates width | src/components/ui/Controls.tsx:163-197 | Use a ref plus `transform: scaleX` driven by requestAnimationFrame. |
| FC-46 | Info (reported Low) | SlidingIndicator transitions width | src/components/ui/SlidingIndicator.tsx | Animate with `transform` (scaleX/translateX) instead of width. |
| FC-47 | Info (reported Low) | Tray icon sent as a 64 KB JSON number array | src/lib/accent.ts:142 | Send the pixels as binary. |

## Unverifiable from source or disputed

| ID | Reported | Title | What is uncertain |
|---|---|---|---|
| FC-22 | Medium | Keyboard reorder in the mixer loses focus after one step | reachability: confirmed; code: refuted |

## Claims rejected by verification

| ID | Claim | Why it was rejected |
|---|---|---|
| FC-41 | EngineStatus fields are dropped by the store and never shown | The store does pick only some fields: src/store.ts:273-285 sets bufferActive, recording, recordingSeconds, recordingBytes, bufferedSeconds, screenFallback, bufferBytes, rateControl, detectedGame, and leaves out droppedFrames/encoder/fps/gameSeconds/freeBytes. But the in-game console window (src-tau… |

## What is solid

- `npx tsc --noEmit` passes.
- `listen()` cleanup in App.tsx and Toasts.tsx uses the correct cancelled-flag pattern.
- Every object selector uses `useShallow`, so there are no "getSnapshot should be cached" problems.
- `SourceMeter` isolates the 20 Hz level updates.
- Clip reloads are ticketed (`clips-changed` at store.ts:301).
- Thumbnails use `loading="lazy"`.
- No XSS sinks: no `dangerouslySetInnerHTML`, `innerHTML`, `href` or `window.open` in `src`.
- Friend names, statuses, game names and transfer names render as text.
- Avatar `<img src>` is limited by CSP `img-src` to `cdn.discordapp.com`.
- Share-link URLs are never rendered.
- Devtools are absent in release builds (no `devtools` cargo feature).
- Context menu and drop handling lockdown is effective (`dragDropEnabled:false` plus `preventDefault`).
- Apart from the listed drift, the IPC wrappers in the slice match model.rs/commands.rs, including camelCase argument names (`startMs`, `targetBytes`, `clipId`) and serde's `accent_color_2` -> `accentColor2`; `mock.ts` is typed so `tsc` keeps it in sync.
- `NameField` (Clips.tsx:697-771) keeps a draft and commits on blur/Enter.
- The gallery player (Clips.tsx:344-376) correctly handles new clips arriving during playback via `shownId`.
- Colors.tsx:29-34 already debounces writes with `SETTLE_MS`.

## Improvement opportunities and missing features

| ID | Idea | Why | Effort | Sketch |
|---|---|---|---|---|
| FC-P-1 | Frontend lint setup | Catches hook-dependency bugs, floating promises and a11y issues automatically. | S | Flat ESLint config with `eslint`, `typescript-eslint`, `eslint-plugin-react-hooks` (recommended-latest) and `eslint-plugin-jsx-a11y`; enable `@typescript-eslint/no-floating-promises` (would flag `upsertSource(s);` AudioMixer.tsx:242 and `inTauri && api.revealClip()` Clips.tsx:602). |
| FC-P-2 | Frontend test setup | Regression safety for store logic and pure helpers. | M | `vitest` with `jsdom`, `@testing-library/react`, `mockIPC`/`clearMocks` from `@tauri-apps/api/mocks` for store tests. Test first: `format.ts`; Hotkeys `accelerator`/`risky`/`supported`; Recording `fit`/`fpsChoices`/`memoryBudgetMb` (moved into `lib/`); `screenChoice`; `slotFor`/`usePresence`; `time… |
| FC-P-3 | Pull-request CI workflow | Every PR is type-checked, linted and tested before merge. | S | `.github/workflows/ci.yml` on `pull_request` running `npm ci && tsc --noEmit && eslint . && vitest run`, plus `cargo test` on Windows. |
| FC-P-4 | npm scripts and stricter tsconfig | Standard entry points for checks and stronger type safety. | S | Add `typecheck`, `lint`, `test` scripts to package.json; turn on `noUncheckedIndexedAccess` and `noImplicitReturns` in tsconfig.json. |
| FC-P-5 | Disable WebView2 browser accelerator keys | A robust lock on browser shortcuts instead of JS key matching (unverified). | ? | Set WebView2 `AreBrowserAcceleratorKeysEnabled=false`; check whether the wry/Tauri version in use exposes it. |
| FC-P-6 | Gallery sorting and date grouping | Find clips faster in large libraries. | M | Sort by date, size, duration or name; group by date. |
| FC-P-7 | Gallery multi-select with bulk actions | Manage many clips at once. | M | Multi-select with bulk delete, tag, move or export. |
| FC-P-8 | Keyboard navigation through the gallery grid | Faster, accessible browsing. | M | Arrow-key navigation through the grid, Enter to open, Delete to remove. |
| FC-P-9 | Ctrl+F search focus and #tag search syntax | Quicker search; tags become filterable from the search box. | S | Ctrl+F focuses search (lockdown currently swallows Ctrl+F); support a `#tag` search syntax. |
| FC-P-10 | Keep gallery filter and scroll position across route switches | Users return to where they left off. | S | Persist filter state and scroll position per route. |
| FC-P-11 | Delete clips to the Recycle Bin | Recoverable deletes and no orphaned files. | M | Replace `Library::delete`'s `remove_file` (whose errors are ignored) with a move to the Recycle Bin, handling errors. |
| FC-P-12 | Drag a clip out into Discord or Explorer | Easy sharing via drag and drop. | M | Support dragging clip files out of the gallery. |
| FC-P-13 | Storage auto-cleanup | Prevents the clip folder filling the disk. | M | Keep N GB; never delete favorites. |
| FC-P-14 | Health panel | Users see recording health at a glance. | S | Show dropped frames, the real encoder, free disk space and the game session (all already in `EngineStatus`). |
| FC-P-15 | Notification history / error center | Errors remain reviewable after toasts disappear (3-6 s). | M | An error center listing past notifications. |
| FC-P-16 | Recording page improvements | Fewer misconfigurations. | S | An "Auto" choice for the memory budget; disable unavailable encoders; show estimated RAM use. |
| FC-P-17 | Mixer gain controls | Precise and quick level adjustments. | S | Double-click to reset gain to 0 dB, numeric gain entry, peak/clip indicators. |
| FC-P-18 | Settings search, reset and import/export | Easier configuration management. | M | Settings search, reset to defaults, import/export. |
| FC-P-19 | Locale-aware dates | Dates in the user's own format. | S | Use `navigator.language` instead of hard-coded `en-US` in format.ts. |
| FC-P-20 | Dormant mode | No UI cost while gaming. | M | Pause animations and level traffic while the window is unfocused or hidden. |
