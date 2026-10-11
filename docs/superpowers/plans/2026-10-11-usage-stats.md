# Usage Stats and Download Sources Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Count downloads per campaign link and send one anonymous daily ping per install, so downloads, installs and retention can be compared per source.

**Architecture:** The site worker serves the setup file under `/get/<ref>` with the ref in its file name and counts the download in D1. The installer writes its own file name to a text file; the app reads the ref from it and sends it, with daily counts, to a new `POST /t/ping` on the friends server, which upserts two tables. Nothing in this repo reads the tables back.

**Tech Stack:** Cloudflare Workers + D1 (TypeScript, `node --test`), Tauri 2 / Rust (`cargo test`), React + TypeScript, NSIS hooks.

**Spec:** `docs/superpowers/specs/2026-10-11-usage-stats-design.md`

## Global Constraints

- All user-visible text in English. Commits in English, **without** a `Co-Authored-By` line.
- Nothing in this repo, its commits, comments or the site mentions who or what reads the statistics. Neutral wording only ("aggregated statistics").
- Ref pattern `^[a-z0-9-]{1,32}$`; missing or invalid ref counts as `site`.
- `heard` values: `tiktok`, `youtube`, `discord`, `friend`, `reddit`, `other`.
- Feature bits: 0 `console`, 1 `friends`, 2 `send_friend`, 3 `share_link`, 4 `editor`, 5 `stream_deck`.
- Ping body ≤ 1024 bytes; counts 0–10 000; `features` 0–63; `version` ≤ 32 chars; `id` UUID v4.
- Retention 400 days. Ping at most every 20 h. Counters flushed to disk at most once a minute.
- The ping never carries the friends auth header.
- `npm run build` (vite) only via PowerShell; `tsc` works from WSL. `cargo test` via PowerShell on the Windows side (see PLAN.md build note).
- Deploying workers and applying the remote D1 migration is outward-facing: ask Fabi before each.

## Review Focus

- A user who renames the installer or downloads straight from GitHub: app must behave exactly as without a source (no error, wizard asks the question). → Task 3 test `no_source_without_a_ref`.
- Switching usage stats off while a ping is in flight or counters are pending: nothing may be sent afterwards and `usage.json` must be gone. → Task 3 test `switching_off_forgets_everything`.
- Offline for days: counters keep adding up and go out in one ping later, not lost, not sent twice. → Task 3 test `a_failed_send_keeps_the_counts`.
- Garbage or oversized ping bodies (`features: 999`, negative counts, 5 KB body, non-UUID id): 400, nothing written. → Task 1 tests.
- An existing user updating: no ping before the one-time toast was shown. → Task 3 test `nothing_goes_out_before_the_notice`.

---

### Task 1: Friends server — ping route, tables, retention

**Files:**
- Create: `server/migrations/0006_usage.sql` (exact SQL from the spec, section 4)
- Create: `server/src/telemetry.ts`
- Create: `server/src/telemetry.test.ts`
- Modify: `server/src/index.ts` (route before `authenticate`, sweep in `scheduled`)

**Interfaces:**
- Produces (all exported from `server/src/telemetry.ts`, also used by Task 2):
  - `REF_PATTERN: RegExp`, `HEARD: readonly string[]`
  - `type Ping = { id: string; version: string; source: string | null; heard: string | null; clips: number; shots: number; recordings: number; features: number }`
  - `cleanPing(raw: unknown): Ping | null`
  - `utcDay(ms: number): string` → `"YYYY-MM-DD"`
  - `cutoffDay(ms: number, days = 400): string`
  - `refFromPath(pathname: string): string` (`/get/tiktok1` → `tiktok1`, `/get`, `/get/`, `/get/Bad!` → `site`; upper case is lowered before checking)
  - `isBot(userAgent: string | null): boolean` (`bot|crawler|spider`, case-insensitive)
  - `downloadName(version: string, ref: string): string` → `ClippiBoy_${version}_x64-setup.${ref}.exe`
  - `handlePing(request: Request, env: Env): Promise<Response>`
  - `sweepUsage(env: Env, now = Date.now()): Promise<void>`

- [ ] **Step 1: Write failing tests** in `telemetry.test.ts` (style of `shares.test.ts`):
  - `cleanPing` accepts `{ id: crypto.randomUUID(), version: "0.8.3", source: "tiktok1", heard: "discord", clips: 3, shots: 1, recordings: 0, features: 5 }` unchanged.
  - Returns `null` for: id `"abc"`, a v1-style UUID, `features: 64`, `clips: -1`, `clips: 10001`, `clips: 1.5`, `version` of 33 chars, `source: "Bad!"`, `heard: "myspace"`, a non-object.
  - Missing `source`/`heard` become `null`.
  - `utcDay(Date.UTC(2026, 9, 11, 23, 59))` → `"2026-10-11"`; `cutoffDay(Date.UTC(2026, 9, 11), 400)` → `"2025-09-06"`.
  - `refFromPath` cases above; `isBot("Discordbot/2.0")` true, `isBot(null)` false; `downloadName("0.8.3","site")` → `"ClippiBoy_0.8.3_x64-setup.site.exe"`.

- [ ] **Step 2: Run** `cd server && npm test` — Expected: FAIL, module `./telemetry.ts` not found.

- [ ] **Step 3: Implement** `telemetry.ts`. `handlePing`: reject `content-length` > 1024 or text > 1024 with 400 before parsing; `cleanPing` → 400 `{ error: "Bad ping." }`; then `env.DB.batch` with the two upserts:

```sql
INSERT INTO installs (id, source, heard, first_day, last_day, version) VALUES (?1, ?2, ?3, ?4, ?4, ?5)
ON CONFLICT(id) DO UPDATE SET source = COALESCE(installs.source, excluded.source),
  heard = COALESCE(installs.heard, excluded.heard), last_day = excluded.last_day, version = excluded.version;
INSERT INTO install_days (id, day, clips, shots, recordings, features) VALUES (?1, ?2, ?3, ?4, ?5, ?6)
ON CONFLICT(id, day) DO UPDATE SET clips = clips + excluded.clips, shots = shots + excluded.shots,
  recordings = recordings + excluded.recordings, features = features | excluded.features;
```
  Answer `json({ ok: true })`. `sweepUsage` deletes `install_days`/`downloads` with `day < cutoff` and `installs` with `last_day < cutoff`.

- [ ] **Step 4: Wire up** in `index.ts`: `if (path === "/t/ping" && method === "POST") return handlePing(request, env);` next to the share-page routes (before `authenticate`); `scheduled` becomes `ctx.waitUntil(Promise.all([sweep(env), sweepUsage(env)]))`.

- [ ] **Step 5: Run** `npm test` and `npx tsc --noEmit` in `server/` — Expected: all pass. Then `npx wrangler d1 migrations apply clippiboy-friends --local` and `npx wrangler dev --compatibility-date 2026-05-28`; `curl -X POST localhost:8787/t/ping -d '<valid body>'` → `{"ok":true}`; second identical call → `install_days.clips` is 6.

- [ ] **Step 6: Commit** `Server: anonymous usage pings (POST /t/ping) with two tables kept 400 days`

---

### Task 2: Site — `/get/<ref>` downloads

**Files:**
- Modify: `server/site-proxy/index.ts`
- Modify: `server/site-proxy/wrangler.jsonc` (add `d1_databases` binding `DB` → `clippiboy-friends`, same id as `server/wrangler.jsonc`)
- Modify: `site/index.html` (both `data-download` hrefs → `/get/site`)
- Modify: `.github/scripts/bake-release.mjs:95` (stop overwriting `data-download` hrefs)
- Modify: `site/main.js:352` (instead of the GitHub URL: `/get/<ref>`, where `ref` is the page's own `?ref=` if it matches the pattern, else `site`)

**Interfaces:**
- Consumes: `refFromPath`, `isBot`, `downloadName`, `utcDay`, `REF_PATTERN` from `../src/telemetry.ts`.

- [ ] **Step 1: Implement** in `index.ts`, before the GitHub Pages fetch: on `/get` or `/get/...`:
  1. `fetch("https://api.github.com/repos/EinFabo/clippiboy/releases/latest", { headers: { "User-Agent": "clippiboy-site" }, cf: { cacheEverything: true, cacheTtl: 300 } })`, take the asset matching `/-setup\.exe$/i`. On failure: 302 to `https://github.com/EinFabo/clippiboy/releases/latest` (no count).
  2. Fetch `browser_download_url` (follows redirects), return its body with `Content-Type: application/octet-stream`, `Content-Length` from upstream, `Content-Disposition: attachment; filename="<downloadName(version without leading v, ref)>"`, `Cache-Control: no-store`.
  3. If `GET` and `!isBot(ua)`: `ctx.waitUntil(DB.prepare("INSERT INTO downloads (day, ref, count) VALUES (?, ?, 1) ON CONFLICT(day, ref) DO UPDATE SET count = count + 1").bind(utcDay(Date.now()), ref).run())`. `fetch` needs the `ctx` parameter now.

- [ ] **Step 2: Site** changes as listed under Files. `main.js` keeps showing version and size from the release data.

- [ ] **Step 3: Verify locally**: `npx wrangler dev` in `server/site-proxy` with `--local` D1 → `curl -sI localhost:8787/get/test` shows the `Content-Disposition` with `.test.exe` and no row; `curl -s -o /dev/null localhost:8787/get/test` adds `downloads(today, 'test', 1)`; `/get/Bad!` counts as `site`. `npx tsc --noEmit` in `server/`.

- [ ] **Step 4: Commit** `Site: downloads go through clippiboy.com/get/<ref>, counted per link and day`

---

### Task 3: App core — `telemetry.rs`

**Files:**
- Create: `src-tauri/src/telemetry.rs`
- Modify: `src-tauri/src/lib.rs` (`pub mod telemetry;`, `telemetry::start(handle)` next to `updater::watch`, register commands)
- Modify: `src-tauri/src/model.rs` (`AppConfig`: `#[serde(default = "yes")] pub usage_stats: bool`, `#[serde(default)] pub usage_stats_noticed: bool`)
- Modify: `src-tauri/src/config.rs` (defaults for a fresh config: `usage_stats: true`, `usage_stats_noticed: false`)
- Modify: `src-tauri/src/state.rs:366` (`replace_config` calls `crate::telemetry::on_config(&next)`)
- Modify: `src/lib/types.ts`, `src/lib/mock.ts` (`usageStats`, `usageStatsNoticed`)

**Interfaces:**
- Produces:
  - `pub enum Feature { Console = 0, Friends = 1, SendFriend = 2, ShareLink = 3, Editor = 4, StreamDeck = 5 }`
  - `pub fn clip()`, `pub fn shot()`, `pub fn recording()`, `pub fn used(feature: Feature)` — cheap, lock + bump + mark dirty, no I/O.
  - `pub fn source_from_file_name(name: &str) -> Option<String>`
  - `pub fn start(app: &tauri::AppHandle)`
  - `pub fn on_config(config: &AppConfig)`
  - Tauri commands: `usage_source() -> Option<String>`, `usage_set_heard(heard: Option<String>)`, `usage_notice(app)` (if `setup_done && !usage_stats_noticed`: set and save the flag like `tray_hint_shown` in `lib.rs:1125-1138`, then `notify(app, "info", "ClippiBoy now sends anonymous usage stats. Turn it off in Settings → Behaviour.")`).
- Internals the tests pin: `struct Counts { clips: u32, shots: u32, recordings: u32, features: u8 }` with `fn sent(&mut self, what: &Counts)` (subtract counts, clear sent bits); `struct UsageFile { id: Option<String>, heard: Option<String>, pending: Counts, last_sent: u64 }`; `fn due(file: &UsageFile, config: &AppConfig, now: u64) -> bool`.

- [ ] **Step 1: Write failing tests** in `#[cfg(test)] mod tests`:
  - `source_from_file_name`: `"ClippiBoy_0.8.3_x64-setup.tiktok1.exe"` → `Some("tiktok1")`; `"ClippiBoy_0.8.3_x64-setup.tiktok1 (1).exe"` → `Some("tiktok1")`; `"CLIPPIBOY_0.8.3_X64-SETUP.TikTok1.EXE"` → `Some("tiktok1")`; `no_source_without_a_ref`: `"ClippiBoy_0.8.3_x64-setup.exe"` → `None`; 33-char ref → `None`; `"…-setup.a_b.exe"` → `None`.
  - `a_failed_send_keeps_the_counts`: pending `{3,1,0,0b101}`, snapshot taken, two more clips counted, send fails → pending `{5,1,0,0b101}`; then send succeeds with that snapshot after one more clip and bit 1 → pending `{1,0,0,0b010}`.
  - `nothing_goes_out_before_the_notice`: `due` false when `usage_stats_noticed` is false, true once set and `last_sent` ≥ 20 h ago, false at 19 h.
  - `switching_off_forgets_everything`: `on_config` with `usage_stats: false` on a temp dir with `usage.json` → file gone, a following `clip()` does not recreate it, `due` false.

- [ ] **Step 2: Run** `cargo test --lib telemetry` (PowerShell) — Expected: FAIL to compile, module missing.

- [ ] **Step 3: Implement.** State in a `static USAGE: Mutex<Usage>` (`parking_lot`), path `app_local_data_dir()/usage.json`, set by `start`. `start` spawns one thread: every 60 s flush if dirty; every hour (first round 2 min after start, so it does not compete with startup) check `due` and send. Send: build the ping JSON from a snapshot, `source` = `source_from_file_name` of `app_local_data_dir()/install-source.txt` (trimmed; missing file → `None`), `POST {friends::SERVER}/t/ping` with a module-local `reqwest::Client` (15 s timeout), block_on like `updater::watch`. On 200: `pending.sent(&snapshot)`, `last_sent = now`, flush. Else `log::info!` and leave it. Create `id` as `uuid::Uuid::new_v4()` on first send. Path-taking inner functions so tests use a temp dir.

- [ ] **Step 4: Run** `cargo test --lib` (PowerShell) — Expected: all green, count = previous + new.

- [ ] **Step 5: Commit** `App: anonymous daily usage ping with counts kept across restarts`

---

### Task 4: App — counting at the events

**Files:**
- Modify: `src-tauri/src/lib.rs` (after `sounds::play(app, SoundKind::ClipSaved)` ~130 → `telemetry::clip()`; `Screenshot` ~177 → `shot()`; `RecordingSaved` ~250 → `recording()`)
- Modify: `src-tauri/src/console.rs:160` (`open` → `used(Feature::Console)`)
- Modify: `src-tauri/src/friends.rs:441` (where `connected = true` → `used(Feature::Friends)`)
- Modify: `src-tauri/src/share.rs:241` (`share_send` success → `used(Feature::SendFriend)`)
- Modify: `src-tauri/src/links.rs:330` (`link_create` success → `used(Feature::ShareLink)`)
- Modify: `src-tauri/src/commands.rs:1002` (`apply_clip_edit` success → `used(Feature::Editor)`)
- Modify: `src-tauri/src/control.rs:219` and `:245` (a known action → `used(Feature::StreamDeck)`)

**Interfaces:** Consumes Task 3's `clip/shot/recording/used/Feature`.

- [ ] **Step 1: Add the calls** — only on the success paths, never on errors.
- [ ] **Step 2: Verify** `cargo check` (WSL) and `cargo test --lib` (PowerShell) — green.
- [ ] **Step 3: Commit** `App: count clips, screenshots, recordings and used features for the usage ping`

---

### Task 5: Installer hook

**Files:**
- Modify: `src-tauri/windows/hooks.nsh`

- [ ] **Step 1: Add** `!macro NSIS_HOOK_POSTINSTALL`: `CreateDirectory "$LOCALAPPDATA\gg.clippiboy.app"`; `${IfNot} ${FileExists} "$LOCALAPPDATA\gg.clippiboy.app\install-source.txt"` → `FileOpen`/`FileWrite $EXEFILE`/`FileClose`. Plain ASCII, comment style like the existing macro; the comment says the app parses the name, the hook does not.
- [ ] **Step 2: Verify** with a release build (`npm run tauri build` via PowerShell, Dev-Build note: background only): copy the setup to `…-setup.test.exe`, install on the dev machine with `install-source.txt` removed first → the file contains `ClippiBoy_<v>_x64-setup.test.exe`; run the plain setup again → file unchanged.
- [ ] **Step 3: Commit** `Installer: remember its own file name once, for the download source`

---

### Task 6: UI — wizard, setting, notice

**Files:**
- Modify: `src/components/Setup.tsx` (new step after the welcome, shown only when `usage_source()` returned `null`; switch in the last step "Almost done"; finishing or skipping sets `usageStatsNoticed: true` together with `setupDone: true` at :103 and :123)
- Modify: `src/routes/settings/Behaviour.tsx` (a `Row` with `Toggle` bound to `usageStats`, hint text below)
- Modify: `src/components/Toasts.tsx:44` (after `listen` resolves and not cancelled: `invoke("usage_notice")`)

**Interfaces:** Consumes commands `usage_source`, `usage_set_heard`, `usage_notice` and config fields `usageStats`, `usageStatsNoticed` from Task 3.

Exact copy:
- Step title "Where did you hear about ClippiBoy?", hint "Helps us see what works. Optional." Choices: TikTok, YouTube, Discord, A friend, Reddit, Other; a "Skip" link. A pick calls `usage_set_heard` and advances, like the audio pick at :219.
- Switch label "Send anonymous usage stats", hint "Counts, never content: version, how many clips, which features. No account, no IP." with a link "Privacy" → `https://clippiboy.com/privacy.html`.

- [ ] **Step 1: Implement**, then update the step count text at :181 ("Four quick steps") to match.
- [ ] **Step 2: Verify** `npx tsc --noEmit` (WSL) and `npm run build` (PowerShell) — no errors. Browser preview with `mock.ts`: wizard shows the question when `usage_source` mock returns `null`.
- [ ] **Step 3: Commit** `Setup asks where you heard of ClippiBoy; usage stats switch in setup and Behaviour, with a one-time notice after the update`

---

### Task 7: Privacy page

**Files:**
- Modify: `site/privacy.html` (new section "Usage statistics" per spec section 5; mention it in the summary list near :57)

- [ ] **Step 1: Write** the section: what is sent (the ping fields in plain words), what not (no clips, names, friends, account link; IP addresses are not stored), downloads via `/get/…` counted per link and day only, kept 400 days, how to switch off. Wording "aggregated statistics" — no word about where they are looked at.
- [ ] **Step 2: Verify** the wording against Global Constraints.
- [ ] **Step 3: Commit** `Privacy: anonymous usage statistics and download counts`

---

### Task 8: Ship

- [ ] **Step 1:** Full checks: `server` `npm test` + `tsc`; `cargo test --lib` (PowerShell); `npm run build` (PowerShell).
- [ ] **Step 2: Ask Fabi**, then: `npx wrangler d1 migrations apply clippiboy-friends --remote`, deploy `clippiboy-friends`, deploy `clippiboy-site`. Check `curl -sI https://clippiboy.com/get/test` and one ping with a throwaway UUID; delete those test rows afterwards.
- [ ] **Step 3:** Fabi tests the dev build (wizard, setting, notice) and a download via `/get/test` → `installs.source = 'test'` in D1. Then release as usual; update `PLAN.md` item 113.
