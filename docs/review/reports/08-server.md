# ClippiBoy Worker server: security, privacy, correctness and performance audit (read-only)

**Bottom line:** the core is solid. Every SQL query is parameterized and the sign-in flow is sound. The share quota cannot be raced, share pages escape their content correctly, and media is served with fixed types plus `nosniff`.

The real problems are elsewhere:
1. **Suspensions are not enforced.** A banned user keeps working, and can wipe their own ban by deleting their account.
2. **Nothing rate-limits the unauthenticated routes that write to D1.** The free-tier daily limits are shared by the whole app, so cheap request floods can take it down.
3. **The Durable Object (Hub) accepts unlimited sockets and unlimited presence messages per user.**
4. **Several promises on the privacy page are not implemented in code.**

**How I checked.** I read every file in my slice. `npx tsc --noEmit` passes and `npm test` passes 25/25. I also ran the Worker locally (`wrangler dev --local`) from a copy in the scratchpad, at `<review-scratch>/srv`, and wrote probe scripts `probe1.mjs`, `probe2.mjs`, `probe3.mjs` and `mp4bench.ts` in the scratchpad folder. "Repro" below means the local run confirmed it. The repo is unchanged (`git status` is clean).

**Plan assumption.** Comments in the code ("R2's free tier", "free plan may carry") suggest the Workers/D1/Durable Objects Free plan. Its daily limits are about 100k Worker requests, 100k D1 rows written, 5M D1 rows read and 100k Durable Object requests. On a paid plan, the "outage" findings below become cost findings instead.

---

## 1. Route table

**Friends Worker** (`api.clippiboy.com`, and the `*.workers.dev` address, because `workers_dev: true`):

| Method | Path | Auth | What it does |
|---|---|---|---|
| GET | `/` | none | Banner text |
| GET | `/auth/start?challenge=` | none | Deletes logins older than 10 min, inserts a `logins` row, sets cookie `cb_login` (HttpOnly, Secure, SameSite=Lax, Path=/auth), 302 to Discord |
| GET | `/auth/callback?code&state` | `cb_login` must equal `state` | Discord token exchange → upsert user → ban check → one-time code → `clippiboy://auth?code=` |
| POST | `/auth/exchange` `{code, verifier}` | possession of the verifier | Code is spent on the first try; S256 check with `sameText`; returns a 256-bit bearer token (only its hash is stored) |
| GET/HEAD | `/c/:id` | none | Share page HTML (also reachable through the site proxy) |
| GET/HEAD | `/c/:id.mp4` and `/c/:id.jpg` | none | R2 object, with Range support |
| POST | `/c/:id/report` | none; 5/min per IP (`REPORT_LIMIT`) | Inserts a report, at most 20 open per share |
| POST | `/t/ping` | none | Telemetry upsert into `installs` and `install_days` |
| GET | `/shares/quota` | bearer | Weekly quota and bucket state |
| POST | `/shares?title&game&width&height&tag&name` | bearer + ban check | Atomic pending INSERT…SELECT → MP4-checked stream into R2 → finalize |
| PUT | `/shares/:id/poster` | bearer + owner | JPEG of at most 1 MB into R2 |
| DELETE | `/shares/:id` | bearer + owner | Deletes from R2 and blanks the row (a "tombstone") |
| GET | `/ws` | bearer | Worker sets `X-User`, then hands off to `Hub.getByName(me)` (WebSocket for hello, presence and relay) |
| POST | `/auth/logout` | bearer | Deletes this session |
| GET/PATCH/DELETE | `/me` | bearer | Profile, `allowRequests`, account deletion |
| GET | `/friends` | bearer | Friends, incoming, outgoing, blocked |
| POST | `/friends/request` `{query}` | bearer | Send a request by friend code or Discord username |
| POST / DELETE | `/friends/:uuid/accept`, `/friends/:uuid` | bearer | Accept (recipient only) / remove the pair |
| POST / DELETE | `/blocks/:uuid` | bearer | Block / unblock |
| cron | `17 * * * *` | – | `sweep` and `sweepUsage` |

- **Durable Object RPC methods** (`card`, `friendPresence`, `relay`, `friendsChanged`, `closeAll`) can only be called from Worker code.
- **There are no admin or moderation HTTP endpoints.** Moderation is done by hand with `wrangler d1 execute`. So there is no operator-auth surface to attack, but there is also no way to run `Hub.closeAll` when suspending someone.

**Site proxy** (`clippiboy.com`, `www`):
- Anything other than GET/HEAD gets 405; `www` redirects 301 to the bare domain.
- `/c/[a-z0-9]+` goes through the service binding to the friends Worker.
- `/get[/ref]` reads `release.json` from Pages (cached 300 s), streams `release.setup.url` back as an attachment, and adds +1 to `downloads(day, ref)` when the request is a GET from a non-bot User-Agent.
- Everything else is fetched from `https://einfabo.github.io/clippiboy` + path (`cacheEverything`, 300 s). The host is fixed, so this is not an open proxy and I found no SSRF. Request headers are not forwarded upstream, so there is no header-based cache poisoning.

**Data model**
- **D1:**
  - `users` (with `banned_at`, `unlimited_links`, `links_reset_at`)
  - `sessions` (`token_hash` PK, `last_used`)
  - `logins` (`state` PK, `challenge`, `code` UNIQUE, `user_id`, `created_at`; created_at has **no index**)
  - `friendships` (ordered pair PK, pending/accepted)
  - `blocks`
  - `shares` (`pending` flag, tombstones kept 2 weeks)
  - `reports` (no FK, kept 30 days)
  - `downloads(day, ref)`, `installs` (`last_day` has **no index**), `install_days`
  - Foreign keys cascade from `users`.
- **R2:** `<id>.mp4`, `<id>.jpg`.
- **Hub Durable Object storage:** `published` (presence) and `lastSeen`. Each WebSocket carries an attachment `{user, ready, invisible, presence, at}`.

**Done well:**
- All SQL uses bound parameters; the only interpolations are constant fragments.
- The quota INSERT…SELECT is atomic. Repro: 6 parallel uploads from one account gave 3×200 and 3×429.
- The state cookie, single-use code and S256 verifier together stop both login CSRF and deep-link hijack.
- Owner checks hold. Repro: account A deleting B's share gets 403.
- Friendship is checked before every relay; `X-User` is overwritten by the Worker.
- Escaping is correct. Repro: `<script>`, `<img onerror>` and `</title>` payloads in title, game and tags render as entities.
- Media is served with a fixed `video/mp4` or `image/jpeg` type and `nosniff`.
- Errors never leak stack traces.

---

## 2. Findings, most severe first

### [High] [security / trust & safety] Suspension is not enforced on existing sessions, and a banned user can erase their own ban
- **Where:**
  - `server/src/db.ts:129-147`: `authenticate()` selects only `user_id, last_used` from `sessions` and never looks at `users.banned_at`.
  - The only ban checks are at `index.ts:198` (new sign-ins) and `links.ts:182` (`createShare`).
  - `links.ts:139-144`: `live()` does not consider whether the owner is banned.
  - `index.ts:383-391`: `deleteAccount` runs `DELETE FROM users WHERE id = ?`, which removes `banned_at` along with the row.
  - `hub.ts:178`: `closeAll(reason)` is only ever called from `deleteAccount`.
- **Repro** (seeded user C with `banned_at` set and a live session):
  - `GET /me` → 200.
  - WebSocket hello → C's presence ("Banned Game") reached friend A.
  - A relay from C was delivered to A.
  - On banned user B: the existing share page and media still return 200, and `PUT /shares/:id/poster` → 200. Only `POST /shares` → 403.
  - `DELETE /me` by C → 200, and the users row (including `banned_at`) is gone. The next Discord login would create a fresh account through `upsertDiscordUser`.
- **Scenario:** someone shares abusive clips and the operator sets `banned_at`. Their 180-day session keeps working. They can still use presence, send clips to friends, and send friend requests. Their live links stay up, and they can swap a link's poster (the `og:image` that Discord embeds show) to any JPEG. Then they run `DELETE /me`, sign in again with the same Discord account, and get a clean account with fresh quota.
- **Capability needed:** a suspended user who still holds a token. That is the normal case, because nothing deletes sessions on suspension.
- **Fix:**
  ```ts
  // db.ts authenticate
  const s = await db.prepare(`SELECT s.user_id, s.last_used, u.banned_at FROM sessions s
     JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?`).bind(hash).first<...>();
  if (!s || now - s.last_used > SESSION_IDLE_MS) throw new HttpError(401, ...);
  if (s.banned_at !== null) throw new HttpError(403, "This account is suspended.");
  // links.ts live(): ... FROM shares s JOIN users u ON u.id = s.owner WHERE s.id = ? AND u.banned_at IS NULL
  ```
  - Keep bans outside `users`: a `banned_discord_ids` table, checked in `authCallback` before the upsert. Alternatively, refuse `DELETE /me` while banned, or keep a minimal row.
  - Add a "suspension apply" step. For example, the cron finds users with `banned_at` set and `suspended_applied_at IS NULL`, deletes their sessions, takes down their shares (R2 + tombstone), and calls `HUB.getByName(id).closeAll("suspended")`. This avoids adding a new admin HTTP surface.
  - Also check the ban in `putPoster`.

### [High on Free plan, Medium on paid] [security / perf, DoS] Unauthenticated, unthrottled routes write to D1, and one of them amplifies reads quadratically; this exhausts the shared daily quota or bloats the database
Each of these routes writes to D1 with no rate limit:

- **`POST /t/ping`** (`telemetry.ts:83-111`). There is no rate limit; the only check is the body shape. Each ping with a fresh random UUIDv4 inserts into `installs` and `install_days`. Counting their PK and secondary indexes, that is roughly 6 D1 "rows written" per request, and the rows are **kept 400 days**.
- **`GET /auth/start`** (`index.ts:139-143`). Each request runs `DELETE FROM logins WHERE created_at < ?`, and `logins` has **no index on `created_at`**, so every request scans the whole table. It then runs an INSERT that touches the table plus 2 unique indexes.
  - R requests inside 10 minutes cost about R²/2 rows read. 3,000 requests ≈ 4.5M rows read, which is close to the whole free daily read allowance.
- **Site proxy `GET /get/<ref>`** (`site-proxy/index.ts:67-75`). Any client with a browser User-Agent adds one D1 write to the **same database**. `ref` is any `[a-z0-9-]{1,32}`, so every new ref creates a row kept 400 days. The count happens even if the client aborts right after the headers.
- **Hourly `sweepUsage`** (`telemetry.ts:119`) runs `DELETE FROM installs WHERE last_day < ?`. `installs.last_day` has no index, so this is a full scan every hour (rows read = install count × 24 per day). Fake pings inflate it further.

**Scenario:** a script sends ~20k pings per day (about one every 4 s) and exhausts the free write allowance. After that, every D1 write fails until 00:00 UTC: sign-in, `sessions.last_used` touches, uploads and friend actions. On a paid plan, unbounded growth and billing take the place of the outage. **No capability is needed.** Confirmed in code; the platform limits are a "likely" assumption.

**Fix:**
- Add rate-limit bindings keyed by `CF-Connecting-IP`, as `REPORT_LIMIT` already does without storing the address: `PING_LIMIT` (for example 5/60 s) and `AUTH_LIMIT`. Add a binding or a WAF rule for `/get` in the site proxy.
- Turn off `workers_dev`, or WAF rules can be bypassed through it.
- `CREATE INDEX logins_created ON logins(created_at)`, and move the logins cleanup into the cron.
- `CREATE INDEX installs_last_day ON installs(last_day)`, and run `sweepUsage` once a day.
- Optionally drop pings whose `version` is not a known release, and cap distinct refs per day.

### [Medium] [security / perf] Hub: unlimited sockets per user, and no rate limit on hello/presence; the relay budget is per connection
- **Where:**
  - `hub.ts:66`: `acceptWebSocket` with no cap.
  - `hub.ts:248-252`: the relay budget lives in `this.relayed`, keyed **per WebSocket**.
  - `hub.ts:89-98`: every `hello` or `presence` message runs `sendSnapshot`, then `publish`. That means a Durable Object storage write, a D1 `friendIds` query, and one RPC to each friend's Hub, which in turn sends to every one of that friend's sockets.
  - `hub.ts:256`: every relay does a D1 `areFriends` lookup, even when the result is "not a friend".
- **Repro:**
  - 4 sockets of one user delivered **119 relays in about 2 s** (the intended limit is 30 per 10 s).
  - 200 presence flips produced **200 fan-out messages** at a friend.
- **Scenario:** one free Discord account opens N sockets and spams relays to random UUIDs, or flips its presence. Every message costs a D1 read and Durable Object requests or storage writes. This drains the account-wide D1 and Durable Object daily quotas for everyone, and floods friends' apps.
- **Fix:**
  - Cap sockets per user, for example closing the oldest beyond 5.
  - Apply one per-connection budget to all message types, and close the socket with code 4008 on abuse.
  - Keep the per-user relay budget in `ctx.storage` or in memory per user, not per socket.
  - Cache the friend set in the Hub, invalidated by `friendsChanged`, so relays do not hit D1.
  - Coalesce `publish` calls (debounce about 1 s).

### [Medium] [privacy] "Appear offline" can be detected through a relay probe
- **Where:** `hub.ts:158-162`. `relay()` delivers to every `ready` socket and returns true without checking `attachment.invisible`, so the sender only gets `relayFailed: "offline"` when there are truly no sockets.
- **Repro:** B said hello with `invisible: true`. A relayed `{id: "probe"}`: there was no `relayFailed`, and B received the probe.
- **Scenario:** any accepted friend can poll a body that the target's app ignores, and learn exactly when an invisible friend is online. The privacy page promises: "With 'Appear offline', friends see you as offline."
- **Fix:** if `this.current() === null` (invisible or offline), do not deliver and answer `"offline"`. Alternatively, have the app opt in per relay. Either way the answer must look the same.

### [Medium] [security, DoS] Unbounded request bodies are read into memory, including on unauthenticated routes
- **Where:**
  - `util.ts:16-22`: `body()` calls `request.json()`; used by `/auth/exchange` (unauthenticated), `/me` PATCH and `/friends/request`.
  - `telemetry.ts:85-87`: the size check relies on Content-Length, which is absent in chunked uploads, then calls `request.text()`.
  - `links.ts:324`: `JSON.parse((await request.text()).slice(0, 2048))` reads everything, then slices.
- **Repro:** a 5 MB chunked body to `/t/ping` was read completely before the 400 came back.
- **Scenario:** bodies of up to 100 MB (the edge limit) are buffered and decoded, which can exceed the isolate's 128 MB memory and kill other requests running on the same isolate. `putPoster` already guards against exactly this (see its comment at `links.ts:276`), but these routes do not. Severity is Medium-Low in practice because the attacker pays the bandwidth.
- **Fix:** a shared `readLimited(request, max)` that streams with `getReader()`, cancels and returns 413 once `max` is passed, and rejects when Content-Length exceeds `max`. Validate that the parsed JSON is an object; this also fixes the next item.

### [Medium] [abuse / availability] A few throwaway accounts can keep the global share bucket "full"
- **Where:** `shares.ts:18` sets `TOTAL_BYTES = 9 GB` with `WEEKLY_LIMIT = 3` per account. `links.ts:57`: a pending upload holds its declared `bytes` for up to 30 minutes, and an aborted upload is deleted (`links.ts:252`), so it **does not use up a weekly slot**.
- **Scenario:** about 60 Discord accounts each keep 3 × 50 MB uploads open with a stalled body. Every 30 minutes the reservations lapse and the attacker simply re-opens them. Everyone else gets 507 "Sharing is full" indefinitely, and almost nothing is stored. Real 50 MB uploads from the same accounts do it for 5 days at a time. **Likely.**
- **Fix:**
  - Count a pending reservation against the week even after it is abandoned (keep the row with `aborted = 1` instead of deleting it).
  - Cap how many bytes a single account can have live.
  - Alert on a full bucket.
  - Consider a per-IP rate limit on `POST /shares`.

### [Medium] [privacy] Share pages reveal the uploader's Discord user ID
- **Where:** `db.ts:88` builds the avatar URL as `https://cdn.discordapp.com/avatars/${user.id}/${hash}.png`, and `page.ts:116` puts it into the public page whenever `show_name` is set (the default, which users opt out of).
- **Scenario:** anyone who has the link can read the snowflake ID from the HTML and look up the exact Discord account. The privacy page only says "display name and avatar".
- **Fix:** proxy the avatar through a route such as `/c/:id/avatar`, or use the default avatar, or disclose it. Also, `show_name` is frozen at upload time, so turning the setting off later does not hide the name on links that are already live.

### [Medium, if left seeded] [security] The e2e script seeds known bearer tokens into the production database
- **Where:** `scripts/e2e.mjs:11-21`. The default `BASE` is the production workers.dev address. The tokens are `tok-a-test-only` / `tok-b-test-only` and the friend codes are `AAAAAAAA` / `BBBBBBBB`, all in a public repo. Cleanup relies on a comment ("Remove the accounts afterwards").
- **Scenario:** if the seed is ever applied and not removed, anyone can act as those accounts, including uploading 3 public links per week each.
- **Fix:** generate random tokens at seed time and print them; default to a local `wrangler dev`; give the seeded rows a short `last_used` expiry or a cleanup cron. **Speculative** as to the current state; check with `SELECT * FROM users WHERE discord_id LIKE 'test-%'`.

### [Medium on Free plan] [perf / cost] A 60-second alarm per online user
- **Where:** `hub.ts:33,76,135`. Alarm invocations count as Durable Object requests: 1,440 per day for each always-online user. Against the free 100k per day, that caps the service at roughly 70 always-online users, before counting presence RPCs.
- **Fix:** sweep every 5 minutes, or skip re-arming when every socket was auto-pinged recently.

### [Low-Medium] [trust & safety] Report handling
- Reports are never surfaced to anyone; the operator has to poll D1.
- The owner can delete a reported clip (`deleteShare` wipes it from R2 and blanks the title), which destroys the evidence.
- Clips expire after 5 days whether or not anyone reviewed them.
- Any website can make its visitors' browsers file reports (no-cors `text/plain` POST); the rate limit is per IP and per Cloudflare location, and IPv6 rotation gets around it.
- **Fix:** hide a clip automatically after N distinct-IP reports, notify the operator (webhook or email), and move reported objects to a quarantine prefix instead of deleting them.

### [Low-Medium] [security, supply chain] The download endpoint trusts `release.setup.url` without pinning it
- **Where:** `site-proxy/index.ts:61` streams whatever URL `release.json` (from Pages) names, served as `ClippiBoy_<v>_x64-setup.<ref>.exe` from clippiboy.com.
- **Scenario:** any compromise of the Pages content, the bake workflow, or the `einfabo` GitHub namespace makes the official domain serve an arbitrary binary.
- **Fix:** require `setup.url` to start with `https://github.com/EinFabo/clippiboy/releases/download/` and `version` to match `^\d+\.\d+\.\d+$`.

### [Low] [bug] A `null` or non-object JSON body gives a 500
- **Where:** `util.ts:16-22` casts without validating, and callers destructure the result (for example `index.ts:212`).
- **Repro:** `/auth/exchange` and `/me` PATCH with body `null` → 500.
- **Fix:** have `body()` reject anything that is not an object; check `typeof code === "string"`.

### [Low] [bug] Two users requesting each other at the same moment get a 500
- **Where:** `index.ts:326-342`. Both requests pass the `existing` check, and the second INSERT hits the primary key.
- **Fix:** `INSERT ... ON CONFLICT(user_a, user_b) DO UPDATE SET status='accepted' WHERE requester <> excluded.requester`, then re-read the row.

### [Low] [bug] Presence can arrive at friends out of order
- **Where:** `hub.ts:219-237`. `publish` awaits D1 and RPCs, which opens the Durable Object's input gate, so two quick messages can interleave. Friends may receive the older presence last and keep showing it. **Likely.**
- **Fix:** add a sequence number or `at` to `friendPresence` and have receivers drop older ones; or serialize `publish` through a promise chain.

### [Low] [bug / perf] Range and conditional handling
- **Where:** `links.ts:383-406`.
- **Repro (local R2 emulator):** `bytes=999999-`, `bytes=garbage`, `bytes=10-5` and multi-range requests all returned **206 with the whole object**, instead of 416 or 200. Production R2 may throw instead, which would surface as the generic 500.
- `If-None-Match` is ignored (always 200), and HEAD downloads the object body (an R2 Class B operation).
- **Fix:** parse `Range` yourself; return 416 if `start >= size` and 200 when the range is unparseable; pass `onlyIf: request.headers` and return 304; use `CLIPS.head()` for HEAD.

### [Low] [privacy / correctness] Caching and indexing of media
- **Where:** `links.ts:395` sends `public, max-age=min(left, 3600)`. A deleted, moderated or account-deleted clip stays in browser and intermediary caches for up to an hour, although the page promises it is "removed immediately".
- Media responses have no `X-Robots-Tag: noindex`, and `api.clippiboy.com/robots.txt` returns 401.
- **Fix:** `private, max-age=300` or `no-cache` with an ETag, and `X-Robots-Tag: noindex` on media and on the gone page.

### [Low] [security hardening] Missing headers on the share and sign-in pages
- No CSP, `frame-ancestors` or `Referrer-Policy` on the share page (`links.ts:372`) or the auth pages (`index.ts:242`).
- The callback page carries the one-time code but has no `Cache-Control: no-store`.
- Escaping is correct, so this is defence in depth only.
- **Fix:** `Content-Security-Policy: default-src 'none'; script-src 'sha256-…'; style-src 'unsafe-inline'; img-src https://api.clippiboy.com https://cdn.discordapp.com; media-src https://api.clippiboy.com; font-src https://clippiboy.com; connect-src https://api.clippiboy.com; frame-ancestors 'none'`.

### [Low] [security / privacy] Session lifecycle gaps
- Idle sessions are rejected after 180 days but never deleted (no sweep).
- `logins` rows, which hold `user_id` and the one-time code, are only cleaned up when someone next calls `/auth/start`.
- There is no "sign out everywhere".
- Logout and suspension do not close a socket that is already open: the token is only checked when the socket connects (`index.ts:82-91`).
- **Fix:** have the cron delete stale sessions and logins; add `DELETE /sessions`; call `closeAll` on logout of the last session or on suspension.

### [Low] [bug] Cron robustness
- **Where:** `index.ts:45`. `Promise.all` inside `waitUntil`: if `sweep` rejects, the invocation may end while `sweepUsage` is still running.
- Within `sweep` (`links.ts:419-448`) the steps run in sequence, so an R2 error skips the stale-upload cleanup, the tombstone purge, and the 30-day report deletion (a privacy-page promise).
- **Fix:** `Promise.allSettled`, and wrap each step on its own.

### [Low] [bug] Account deletion fails for accounts with many live links
- **Where:** `links.ts:305`. Deleting more than 500 live shares means more than 1,000 keys in one R2 `delete`, which throws, so `DELETE /me` returns 500. Only reachable for accounts with `unlimited_links` or `QUICK_LINKS_FOR`.
- **Fix:** delete in chunks of 1,000 keys.

### [Low] [perf] The MP4 checker has no cap on box count
- **Where:** `shares.ts:173-216`.
- **Bench:** a 50 MB file made of 6.5M 8-byte boxes takes about **1.1 s of CPU** in V8 (a normal file takes 0.2 ms). It passes the check, and on the Free plan it runs past the CPU limit and stalls the isolate.
- **Fix:** refuse files with more than about 100k top-level boxes.
- Box parsing is otherwise correct: largesize is handled, `size < header` is refused, a first box other than `ftyp` (or one over 1 KB) is refused.
- Note: an MP4/HTML polyglot (ftyp + moov + size-0 mdat + HTML) is **accepted** (repro). That is by design, and it is safe only because of `video/mp4` plus `nosniff`. Keep it that way.

### [Low] [abuse] Friend requests: spam and enumeration
- **Where:** `index.ts:313-345`. There is no rate limit and no cap on outgoing pending requests, and Discord usernames can be guessed.
- The response distinguishes 404 (no such user, or they blocked you) from 403 (allow_requests is off), which reveals which Discord accounts use ClippiBoy.
- Usernames are only synced at login (`db.ts:93-119`), so after a Discord rename two rows can share a username.
- **Fix:** rate-limit; cap outgoing pending requests at around 20; return the same answer for 403 and 404; make `username` unique and sync it on use.

### [Low] [abuse] Posters can be swapped at any time
- **Where:** `links.ts:273-289`. The owner can replace the poster (the `og:image` in Discord and Twitter embeds) with any JPEG, any number of times, with no limit. That costs R2 Class A operations and allows bait-and-switch previews.
- **Fix:** allow one poster PUT per share, or a few at most.

### [Low] [bug] Expiry rounding overstates the time left
- **Where:** `shares.ts:98` uses `Math.round(left / DAY_MS)`. With 12 hours left the page says "in 1 day"; with 4.5 days left it says "in 5 days".
- **Fix:** use `Math.floor` for days, and show hours below 2 days.

### [Low] [ops] Other ops notes
- `workers_dev: true` lets anyone bypass zone-level WAF and rate-limit rules.
- `QUICK_LINKS_FOR` reaches `env` through a cast (`links.ts:67`) and is missing from `secrets.d.ts`.
- The site proxy is not covered by `npm run check` (tsconfig only includes `src`).
- **Speculative:** the proxy's Location rewrite `location.slice("/clippiboy".length)` (`site-proxy/index.ts:41`) would turn `/clippiboy//x` into the protocol-relative `//x`. Only rewrite results that start with a single `/`.

---

## 3. Privacy page vs implementation

1. **"A suspended account is signed out, cannot sign in or share links, and its links are taken down."** Only "cannot sign in again" and "cannot create links" are implemented. Existing sessions, sockets, relays, live links and poster swaps keep working, and the ban disappears on `DELETE /me` (repro).
2. **Usage stats are "anonymous", with an installation number "not linked to you".** In practice it is pseudonymous: a persistent ID with a per-day profile kept up to 400 days. A unique `/get/<ref>` handed to a single person would identify them. Either commit to never issuing per-person refs, or say "pseudonymous".
3. **"Our server does not store your IP address with it."** True for D1. However, `observability.enabled` (`wrangler.jsonc:8`) turns on invocation logs, which may record request metadata (time, path `/t/ping`, possibly IP and headers) for some days. **Speculative:** check in the dashboard. Consider turning invocation logs off or excluding `/t/ping`.
4. **"The database is located in Western Europe."** The D1 and R2 locations cannot be verified from the repo. The Hub Durable Object storage (presence, status text, game, last seen) is not pinned to a jurisdiction (no `jurisdiction("eu")`), so it is created wherever the user first connects.
5. **"No cookies."** `api.clippiboy.com` sets `cb_login` during sign-in. It is strictly necessary, so this is a wording issue.
6. **Share links show "display name and avatar".** The avatar URL also exposes the Discord user ID.
7. **Delete account removes everything "immediately".** Media can survive in caches for up to an hour (`public, max-age`); `logins` rows stay until the next sign-in start; reports referencing the share IDs stay 30 days (they hold no personal data).
8. **Sessions "expire after 180 days" (README).** They are rejected after that, but the rows are kept until the account is deleted.
9. **Spec: "no public counting route anyone could call".** `/get/<ref>` is itself that route, and its counts can be forged freely. The spec accepts fake pings but not fake downloads.
10. **Consistent with the page:** downloads store only `(day, ref, count)` (the proxy has no observability configured); the 400-day retention works (the cutoff keeps 400–401 days); report rows hold no reporter or IP; relays store nothing (`console.error` logs user IDs only on errors); the `identify` scope only; tombstones drop title, game and tags and are purged after 2 weeks.

---

## 4. Most valuable missing tests

Use `@cloudflare/vitest-pool-workers` or Miniflare with real D1, R2 and Durable Objects. Today's 25 tests only cover pure functions.

1. **Auth flow:** cookie/state mismatch, code reuse, expired login, wrong verifier, banned user at callback and **on every authenticated route**, `DELETE /me` while banned.
2. **Authorization matrix:** cross-user `DELETE` and poster `PUT` on shares, accept by the requester, block and friendship interactions.
3. **Upload quota race:** N parallel uploads; abandoned pending uploads; a body shorter or longer than its Content-Length; a non-MP4 gives 415 and leaves no R2 object behind.
4. **`Mp4Boxes` edge cases:** size-0 box to the end of the file, 64-bit largesize, size smaller than the header, `ftyp` over 1 KB, very many tiny boxes (CPU), the polyglot case documented.
5. **Rendering:** a snapshot test of `renderPage` with hostile title, game, tags and uploader name, plus the response headers (`nosniff`, cache, CSP once added).
6. **Hub:** relay to a non-friend, to an invisible friend, budget across several sockets, presence fan-out only to accepted friends, the stale-socket alarm, `closeAll`.
7. **Body limits:** `handlePing`, report and `body()` with chunked oversized bodies, and with `null` or array JSON.
8. **Sweep:** expired, stale-pending and tombstone purge; partial R2 failure; reports past 30 days; the usage cutoff boundary.
9. **Range and conditional requests:** unsatisfiable, multi-range, suffix ranges, `If-None-Match`, HEAD.
10. **Site proxy:** `refFromPath`, HEAD not counted, bots not counted, Location rewrite, pinning of `release.setup.url`.
11. **`expiresIn`** at 12 h, 36 h and 4.5 days.
