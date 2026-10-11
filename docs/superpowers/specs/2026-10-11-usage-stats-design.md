# Anonymous usage stats and download sources

Status: approved design, 2026-10-11.

## Goal

Know, per download link, how many people download ClippiBoy, how many of them
install it, and how many still use it a day, a week and a month later, and
roughly how (clips per day, which features). Everything anonymous: no account,
no name, no IP address is stored, and it can be switched off.

Example outcome: a link `clippiboy.com/get/tiktok1` posted under one video shows
after a week as "tiktok1: 120 downloads, 45 installs, 20 still active on day 7".

## Non-goals

- No per-person tracking, no fingerprinting, no IP matching between download and
  first start.
- No third-party analytics SDK.
- No reading of these statistics through the friends server. It only writes them.
- Microsoft Store downloads (item 114) are counted by the Store's own campaign
  links; nothing here for them.

## 1. Downloads with a source (`server/site-proxy`)

- New path **`/get/<ref>`** on `clippiboy.com`. `ref` matches
  `^[a-z0-9-]{1,32}$`; anything else (or `/get` alone) counts as `site`.
- The worker looks up the latest release's setup asset (GitHub API, cached
  300 s like the page), fetches it and streams it back with
  `Content-Disposition: attachment; filename="ClippiBoy_<version>_x64-setup.<ref>.exe"`.
  A redirect is not enough: then GitHub's file name wins.
- A `GET` adds one to `downloads(day, ref)`. Not a `HEAD`, and not a request
  whose `User-Agent` contains `bot`, `crawler` or `spider`. Nothing else about
  the request is stored.
- The proxy writes to the friends database through its own D1 binding, so there
  is no public counting route anyone could call.
- The site's download buttons point at `/get/site` instead of GitHub
  (`site/index.html`, `data.setup.url` in `site/main.js`). The size and version
  shown stay as they are.
- A download straight from GitHub carries no source; it shows up as an install
  without one.

## 2. Installer (`src-tauri/windows/hooks.nsh`)

- `NSIS_HOOK_POSTINSTALL` writes `$EXEFILE` (the installer's own file name) to
  `$LOCALAPPDATA\gg.clippiboy.app\install-source.txt`, **only if that file does
  not exist yet**. So the first install decides; updates (which run the plain
  GitHub installer) change nothing.
- The hook does no parsing. The app reads the source out of the name (see 3),
  which copes with Windows' `" (1)"` suffix and stays testable.
- Plain ASCII, like the existing hook.

## 3. App

### Settings
- `AppConfig.usage_stats: bool`, default `true` (serde default, so existing
  configs get `true`).
- `AppConfig.usage_stats_noticed: bool`, default `false`. See "Existing users".
- Switch **"Send anonymous usage stats"** in the *Behaviour* tab with one line
  under it: "Counts, never content: version, how many clips, which features.
  No account, no IP." and a link to the privacy page.

### State file (`%LOCALAPPDATA%\gg.clippiboy.app\usage.json`)
```json
{ "id": "uuid-v4", "heard": "discord", "pending": { "clips": 3, "shots": 1, "recordings": 0, "features": 5 }, "last_sent": 1791700000 }
```
- `id`: random UUID v4, made on first need. It is unrelated to the friends
  account and the machine.
- Switching the setting off stops all pings and deletes `id` and `pending`.
  Switching it on again starts with a new id.

### `src-tauri/src/telemetry.rs`
- `source_from_file_name(name) -> Option<String>`: matches
  `-setup\.([a-z0-9-]{1,32})(?: \(\d+\))?\.exe$` case-insensitively, returns the
  ref lowercased. Pure, unit-tested.
- Counters in memory behind a mutex: `clips`, `shots`, `recordings` and a
  feature bitmask. Bumped from the places that already announce these events:

  | Bit | Feature | Set when |
  |---|---|---|
  | 0 | `console` | the console opens |
  | 1 | `friends` | the friends connection comes up signed in |
  | 2 | `send_friend` | a clip is sent to a friend |
  | 3 | `share_link` | a share link is created |
  | 4 | `editor` | a clip edit is applied |
  | 5 | `stream_deck` | a Stream Deck action arrives |

- Counters are written to `usage.json` at most once a minute and on exit, so a
  crash loses at most a minute.
- A thread like `updater::watch`: at start and then hourly, if `usage_stats` is
  on and `last_sent` (unix seconds) is at least 20 hours ago, send one ping. The server files it under its own UTC day. On `200`, zero the
  sent counters (keeping anything counted meanwhile) and set `last_sent`. On any
  error, keep everything for the next round, log at info level, show nothing.
- The ping uses a plain client **without** the friends auth header.

### Ping (`POST https://api.clippiboy.com/t/ping`)
```json
{ "id": "…", "version": "0.8.3", "source": "tiktok1", "heard": "discord",
  "clips": 3, "shots": 1, "recordings": 0, "features": 5 }
```
`source` and `heard` are `null` when unknown. They are sent on every ping (a few
bytes); the server keeps the first non-null value.

### Setup wizard (`src/components/Setup.tsx`)
- If there is no source from the file name, the first step after the welcome
  asks **"Where did you hear about ClippiBoy?"** with TikTok, YouTube, Discord,
  A friend, Reddit, Other, and Skip. Stored as `heard` (`tiktok`, `youtube`,
  `discord`, `friend`, `reddit`, `other`), Skip stores nothing.
- The last step shows the same switch as the settings, already on, with the same
  line under it.

### Existing users
They never see the wizard. While `setup_done && usage_stats && !usage_stats_noticed`,
the dashboard shows a notice card at the top (like the update notice): "ClippiBoy
now sends anonymous usage stats" with **Got it** and **Settings** (opens
Behaviour). Either button, or 10 seconds in front of the user (the clock only
runs while the window has the focus), sets `usage_stats_noticed`. No ping goes
out before. A toast was tried first and was missed: a window behind another one
still counts as visible.

## 4. Friends server (`server/src/telemetry.ts`, migration `0006_usage.sql`)

```sql
CREATE TABLE downloads (day TEXT NOT NULL, ref TEXT NOT NULL, count INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (day, ref));
CREATE TABLE installs (id TEXT PRIMARY KEY, source TEXT, heard TEXT, first_day TEXT NOT NULL, last_day TEXT NOT NULL, version TEXT NOT NULL);
CREATE TABLE install_days (id TEXT NOT NULL, day TEXT NOT NULL, clips INTEGER NOT NULL DEFAULT 0, shots INTEGER NOT NULL DEFAULT 0, recordings INTEGER NOT NULL DEFAULT 0, features INTEGER NOT NULL DEFAULT 0, PRIMARY KEY (id, day));
CREATE INDEX installs_source ON installs(source);
CREATE INDEX install_days_day ON install_days(day);
```

- `day` is the UTC date (`YYYY-MM-DD`) at the server.
- `POST /t/ping` (no auth): body at most 1 KB; `id` must be a UUID v4;
  `version` at most 32 chars; `source` like the ref pattern; `heard` from the
  list; counts integers 0–10 000; `features` 0–63. Anything else is a `400`.
- One upsert into `installs` (`source`/`heard` via `COALESCE(existing, new)`,
  `last_day`, `version`) and one into `install_days` adding the counts and
  OR-ing the features. Answer `{ "ok": true }`.
- Validation and the row arithmetic live in pure functions, tested with
  `node --test` next to `shares.test.ts`.
- The hourly cron also deletes `install_days` and `downloads` older than 400
  days and `installs` whose `last_day` is older than 400 days.
- Faked pings cannot be prevented entirely; for statistics that is acceptable.

## 5. Privacy page (`site/privacy.html`)

New section "Usage statistics": what is sent (the ping fields above, plain
words), what is not (no clips, no names, no friends, no IP stored, no account
link), that downloads via `/get/…` are counted per link and day only, how long
rows are kept (400 days), and how to switch it off.

## Testing

- `server`: `node --test` for ping validation and the upsert arithmetic.
- `src-tauri`: `cargo test` for `source_from_file_name` (with ref, without,
  `" (1)"`, upper case, too long, invalid characters).
- By hand: download via `clippiboy.com/get/test`, install on a clean user, start
  once; the row with `source = 'test'` and a `downloads` row for `test` show up
  in D1. The app side is tested by hand in the game.
