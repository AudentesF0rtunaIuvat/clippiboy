# ClippiBoy deep review — 2026-10-17

A full-repository review of ClippiBoy 0.8.3 (commit `417f434`, ~53k lines: Rust/Tauri core, React UI, Cloudflare
Worker, Stream Deck plugin, website, CI). The focus was security, performance, code quality, missing features, and
above all the integrated **screenshot editor**.

Nothing in the application code was changed. This folder is the deliverable. To work through the fixes, by hand or
with a coding agent, start with [FIXING.md](FIXING.md).

## The short version

The codebase is in better shape than its size suggests.
- SQL is parameterised everywhere and there are no HTML-injection sinks in the UI.
- ffmpeg is pinned by hash and updates are signature-checked.
- The control port uses a token with a constant-time compare.
- Peer file names are sanitised, and nearly every `unwrap` sits in tests.
- Sharing links can't be raced past their quota.
- 265 Rust unit tests and 25 server tests pass.

The problems are concentrated in a few themes:

1. **Concurrency on the main thread.** There is one verified deadlock (ST-1), and heavy work runs on the UI thread
   (the toggle-buffer hotkey, `set_config` and other sync commands).
2. **Recording correctness under stress.** A/V drift grows when the encoder falls behind (VP-1). Sleep/resume can
   crash the app (VP-3). Frame slots can be overwritten (VP-2) and a COM reference leaks on every frame (VP-6).
3. **Audio routing edge cases.** Launchers record the game twice (AU-1), 5.1/7.1 devices lose the centre channel
   (AU-3), and clips with one audio track get compounding gain (AU-2).
4. **Edit and file-safety paths.** Non-atomic multi-file commits, races between saving and moving or deleting files,
   and rollback paths that can lose the only copy (AU-4, AU-5, ED-2, ED-3, LS-6).
5. **Screenshot redaction is weaker than it looks** (ED-1), and the editor lacks the safety rails of a mature editor.
6. **Server abuse and moderation.** Suspension isn't enforced, unauthenticated routes are unthrottled, and the Hub
   accepts unlimited sockets.
7. **Release supply chain.** The installer is unsigned, a trial run of the release workflow publishes a real
   release, and actions run on mutable refs next to the signing key.

## Fix first

Ordered by impact over effort. "✔" means verified by the independent pass; "◌" means reported by the area reviewer
(who quoted the code) but not re-checked, because the pass was stopped early to save quota. Severities are the
verified ones.

| # | ID | Sev | What | Fix in one line |
|---|---|---|---|---|
| 1 | [ST-1](07-app-state-and-concurrency.md) ✔ | High | `current_game` mutex guard lives across `overlay::show`, which waits on the main thread; a sync command on the main thread takes the same lock. The whole app can freeze for good (`lib.rs:216-221`, `lib.rs:356`) | Copy the value into a local before calling `overlay::show`. Never hold an AppState guard across window APIs |
| 2 | [AU-2](04-audio-and-clip-editing.md) ✔ | High | Single-track clips: gain compounds on every save, and Mute removes the audio for good | Always mix from the original or source track; or hide gain/mute when there is one track |
| 3 | [ED-1](01-screenshot-editor.md) ✔ | High | The saved redaction blur is a single box pass that deconvolution can invert. The preview shows a much stronger Gaussian | Three box passes (rounded up), plus a solid "Redact" tool; fix the "what you see is what lands" hint |
| 4 | [VP-3](03-video-pipeline.md) ✔ | High | After sleep/resume or a long stall, the mixer allocates the whole gap at once: an out-of-memory abort, with the buffer lost | Treat gaps over about 1 s as silence; resync the pacer; handle suspend and resume |
| 5 | [VP-1](03-video-pipeline.md) ✔ | High | The pacer never skips ticks and the muxer drops PTS, so an encoder deficit turns into ever-growing A/V drift and eventually silent clips | Skip ticks when behind; mux with real timestamps; show the overrun count |
| 6 | [AU-1](04-audio-and-clip-editing.md) ✔ | High | A launcher (Steam, Battle.net…) tapped as "leftovers" includes the game, so game audio lands twice | Skip leftover roots that are ancestors of a taken pid; use `app_root` for the game |
| 7 | [AU-3](04-audio-and-clip-editing.md) ✔ | High | 5.1/7.1 endpoint loopback keeps only channels 0 and 1, so centre-channel dialogue is lost | `AUTOCONVERTPCM` + `SRC_DEFAULT_QUALITY`, or a proper downmix |
| 8 | [AU-4, AU-5](04-audio-and-clip-editing.md) ✔ | Medium | The first cut's rollback can delete the only copy; a crash during a cross-drive copy marks an untouched clip as trimmed | Delete only after a successful move back; copy to `.part` and rename; keep originals on the clip's volume |
| 9 | [LS-6](05-local-attack-surface.md) ◌ | Medium | `delete_clip("")` deletes the stores of all audio tracks and all originals | Look up the clip first; reject ids that are empty or not UUIDs in `stems::dir` / `edit::dir` |
| 10 | [VP-4, VP-5](03-video-pipeline.md) ✔ | Medium | A save copies the whole clip into one Vec and writes temp files to C: even when clips are on D: | Stream to ffmpeg or a BufWriter; put temp files on the output volume |
| 11 | [VP-2, VP-6](03-video-pipeline.md) ✔ | Medium | NV12 slots get overwritten while still queued; a COM reference leaks on every frame (VRAM and device leak on restarts) | Refcounted slots or one conversion per tick; `ManuallyDrop::into_inner` after the Blt |
| 12 | [ST-2, ST-3](07-app-state-and-concurrency.md) ✔ | Medium | The toggle-buffer hotkey starts and stops the pipeline on the UI thread; a persistent start failure retries every 2 s with sound and banner | Spawn a thread; back off exponentially and suppress after N failures |
| 13 | [FC-1](09-frontend-core.md) ✔ | Medium | The UI's config copy goes stale and the whole stale config is re-sent, so an unrelated toggle can restart the buffer after a monitor change | Apply `set_config`'s reply behind a version guard; add a `config-changed` event or patch-style updates |
| 14 | [FP-5](10-player-console-overlay.md) ✔ | Medium | A running Save or Undo-trim doesn't block paging, closing or deleting | Disable those actions while saving, or key the save state by clip id |
| 15 | [SV-1, SV-38](08-server.md) ✔ | Medium | Suspension isn't enforced on existing sessions, links stay up, and a banned user can erase the ban with `DELETE /me`. The privacy page promises otherwise | Check `banned_at` in `authenticate`; keep bans outside `users`; a cron step that applies suspensions |
| 16 | [SV-2, SV-3, SV-12](08-server.md) ✔ | Medium | Unauthenticated `/t/ping` and `/auth/start` are unthrottled and write to D1 (one with a quadratic scan); a 60 s alarm runs per online user | Rate-limit bindings; `logins(created_at)` and `installs(last_day)` indexes; a longer alarm interval |
| 17 | [SC-1, SC-2](11-supply-chain-ci-site.md) ✔ / [SC-3](11-supply-chain-ci-site.md) ◌ | Medium / High | The installer is unsigned; `workflow_dispatch` publishes a signed release tagged `main`; mutable action refs run next to the signing key | Authenticode signing; an empty `tagName` unless running on a tag; pin actions by SHA and add a release environment |
| 18 | [NW-1, NW-2](06-network-and-p2p.md) ◌ | Medium | P2P sharing trusts the server completely (it can fetch or swap a clip); an offer's size can overflow and fill the disk | Bind the transfer to the receiver's NodeId; `checked_add` and caps; check free space |
| 19 | [LS-1…LS-4](05-local-attack-surface.md) ◌ | Medium | The control port (tiny_http) can be crashed or hung without the token; every window can call all 79 commands; paths from the renderer are trusted | Own minimal server with limits; app ACL manifest; validate paths in Rust |
| 20 | [FP-31](10-player-console-overlay.md) ◌ | Medium | Usage stats are on by default, and a notice that dismisses itself counts as consent (GDPR/TTDSG risk) | Opt-in, or an explicit choice; mention the install id and source fields |

Close behind: FC-3 / FP-1 (the main window and the hidden console keep rendering while you game), FP-2 (the
dashboard player follows its clip by position), FP-12 (clips without audio can't be trimmed), AU-11 (device mode
records ClippiBoy's own sounds), and FC-2. FC-2 is real but narrower than first reported: footage is dropped only
when a buffer-slider drag overshoots below the value you finally choose.

## Screenshot editor

The editor has the right core design:
- The object model is non-destructive and every save rebuilds from the untouched original.
- One `paint()` draws vector marks for both the preview and the file, so those cannot drift.
- The watermark survives edits, and the JSON sidecar file is written atomically.

The weak spots and next steps:

- **Redaction is not safe yet (ED-1, verified).** The file's blur is a single box kernel that can be inverted;
  the preview is a stronger Gaussian. A failed thumbnail rebuild also keeps the *unredacted* thumbnail (ED-11).
  Ship a 3-pass blur and a solid Redact tool first.
- **Save safety (reported, not re-verified):**
  - The sidecars (`base.png`, `shot.json`) are committed before the main PNG and are not atomic (ED-2).
  - Saves race with file moves and deletes (ED-3).
  - The editor is usable before its state loads, and a failed load is swallowed (ED-4).
  - The save continuation isn't tied to its clip (ED-5).
  - Marks drawn during a save are thrown away (ED-6).
  - The ✕ close button discards unsaved marks without asking (EF-1).
  - "Remove all marks" has no confirmation or undo (EF-6).
  - Shortcuts stop working after a slider gets focus (EF-5).
- **Performance:**
  - Layer PNGs travel as JSON number arrays (ED-8).
  - Every blur-separated run is a full-resolution canvas (ED-9, about 33 MB each at 4K).
  - A drag repaints the whole run and re-renders the 1650-line viewer (ED-10, verified).
- **Hardening:** a PNG with a huge declared size can abort the app before any check (ED-7). Screenshots from friends
  arrive with sender-claimed dimensions.
- **Features users will expect:**
  - Redo; tool hotkeys; Shift to constrain.
  - Line/double arrow, highlighter, numbered steps, spotlight, pixelate, redact.
  - Copy and "Save & copy" inside the editor; export as PNG/JPEG/WebP with a "fit under 10 MB" option for Discord.
  - Edge handles and typed crop sizes; multi-select and z-order.
  - OCR "copy text" (`Windows.Media.Ocr`); accessibility (dialog semantics, focusable handles).
- **Architecture:**
  - Now: contain the drift between preview and file. That means a 3-pass blur, a feathered oval mask, parity tests
    between the TS and Rust geometry, binary IPC, and a versioned sidecar file.
  - Later: when the second pixel-reading tool (pixelate, magnifier, JPEG export) arrives, move the whole composite
    to one OffscreenCanvas renderer. Rust keeps files, the watermark, the clipboard and thumbnails.

Details: [01-screenshot-editor.md](01-screenshot-editor.md) has all verified findings.
[02-screenshot-editor-roadmap.md](02-screenshot-editor-roadmap.md) has the v2 roadmap: three independent plans
(incremental, architecture-first, gamer-first) and their synthesis.

## Areas

| File | Area | Findings | Verified | Verified ≥ Medium | Not re-verified | Rejected |
|---|---|---|---|---|---|---|
| [01-screenshot-editor.md](01-screenshot-editor.md) | Screenshot editor (bugs + features) | 74 | 57 | 1 | 15 | 1 |
| [02-screenshot-editor-roadmap.md](02-screenshot-editor-roadmap.md) | Editor v2 roadmap | — | — | — | — | — |
| [03-video-pipeline.md](03-video-pipeline.md) | Capture, encode, buffer, mux | 41 | 23 | 6 | 13 | 1 |
| [04-audio-and-clip-editing.md](04-audio-and-clip-editing.md) | Audio, trim, export | 43 | 24 | 9 | 15 | 2 |
| [05-local-attack-surface.md](05-local-attack-surface.md) | Commands, control port, files, config | 41 | 29 | 0 | 11 | 1 |
| [06-network-and-p2p.md](06-network-and-p2p.md) | P2P share, friends, links, ffmpeg download, updater | 27 | 18 | 0 | 5 | 3 |
| [07-app-state-and-concurrency.md](07-app-state-and-concurrency.md) | State, threads, locks, game detection, tray, overlay | 35 | 21 | 3 | 12 | 0 |
| [08-server.md](08-server.md) | Cloudflare Worker + site proxy | 57 | 50 | 6 | 6 | 0 |
| [09-frontend-core.md](09-frontend-core.md) | Store, routes, settings, shared UI | 46 | 39 | 3 | 5 | 1 |
| [10-player-console-overlay.md](10-player-console-overlay.md) | Player/editor UI, console, overlay | 61 | 37 | 4 | 20 | 4 |
| [11-supply-chain-ci-site.md](11-supply-chain-ci-site.md) | CI/release, deps, site, Stream Deck | 44 | 36 | 2 | 7 | 1 |
| [12-gap-hunt.md](12-gap-hunt.md) | Areas no reviewer owned | 15 | — | — | 15 | — |
| [findings.json](findings.json) | Every finding, machine-readable (for turning into issues) | 484 | | | | |
| [reports/](reports/) | The 11 original reviewer reports, unabridged | | | | | |
| [data/verified/](data/verified/) | Every finding with the verifiers' evidence, corrections and fix assessments | | | | | |
| [repro/](repro/README.md) | Reproductions written during verification | | | | | |
| [FIXING.md](FIXING.md) | How to work through the findings, including a prompt for a Claude Code session | | | | | |

The many low and info items are mostly real but small: stale comments, German/English mixing, missing tests, and
minor UX and accessibility gaps. Each area file lists them in a table, together with "What is solid" and an
"Improvement opportunities" table.

## Build and test health

These were run in a Linux container on 2026-10-17. On Linux, code behind `cfg(windows)` is not compiled.

- `tsc --noEmit` passes, and so does the `vite build` (main bundle 232 kB, 69.5 kB gzipped).
- The server's `tsc` passes, and `npm test` passes 25/25.
- `cargo test --lib` passes 265/265. `cargo clippy` reports 24 warnings and no errors.
- **The declared MSRV is wrong.** `rust-version = "1.77"`, but the code uses `LazyLock` (1.80) and
  `ErrorKind::CrossesDevices` (1.85), and iroh 1.x needs 1.91.
- `large_enum_variant` on `share::Signal`.
- Dead-code warnings on Linux come from items used only by Windows code. Gate them with `cfg(windows)`.
- **No CI quality gate.** CI never runs `cargo test`, clippy, fmt, the server tests or `tsc` on pull requests.
  There is no ESLint and no frontend test runner. A ready-made `ci.yml` is in
  [11-supply-chain-ci-site.md](11-supply-chain-ci-site.md).
- The committed `src-tauri/gen/schemas/linux-schema.json` is stale; a Linux build regenerates it.

## How findings were verified

1. **Review.** Eleven reviewers ran in parallel, each owning one slice and reading its files in full. They were
   told to quote code and give a concrete failure scenario for every claim. Two of them covered the screenshot
   editor.
2. **Structure.** Each report was turned into individually checkable claims: 525 in total, 469 after merging 49
   groups of duplicates found across reports.
3. **Adversarial verification.** Each verifier started from "the reviewer is wrong until the code proves
   otherwise", and could also check dependency sources (Tauri, tao, tiny_http, iroh…). There were three lenses:
   - *code truth*: does the code really do this?
   - *reachability*: who can trigger it, and from where?
   - *impact and fix*: how bad is it, and is the fix right?

   How many lenses a claim got depended on its reported severity:
   - Critical and high: all three.
   - Medium: code truth and reachability, with impact as the tiebreaker.
   - Low and info: code truth only.

   A claim counts as verified on a majority vote. Its severity is the impact lens's verdict where there is one,
   otherwise the median.
4. **Stopped early.** Verification was stopped at about 75% to limit quota. 360 of 484 claims have independent
   votes; 124 (mostly medium) are marked "not independently re-verified". Of the claims that were checked:
   - 14 were rejected and one is disputed.
   - Many were confirmed with a correction; the correction is shown under "What verification found".
   - Verifiers tended to lower severities. Most reported highs ended up medium.
5. **Gap hunt.** A gap-hunting pass looked for areas nobody owned. It finished two areas (hotkey accelerators,
   recording crash recovery) before being stopped; those 15 findings were checked only by their finder.
6. **Editor panel.** Three architects independently proposed an editor v2 plan, and a judge merged them.
