# Cloudflare Worker server and site proxy

**Scope:** server/src/*, migrations, site-proxy, wrangler configs, e2e script. The reviewer also ran the Worker locally (wrangler dev) to reproduce several findings.

**Findings:** 57 — 50 verified (6 medium or higher), 6 reported but not independently re-verified, 1 unverifiable/disputed, 0 rejected by verification.

Severity shown is the verified severity; where verification changed it, the reported one is in parentheses. See [README](README.md#how-findings-were-verified) for how verification worked.

## Verified issues (medium and above)

### SV-1 · Medium (reported High) · Suspension is not enforced on existing sessions, and a banned user can erase their own ban via DELETE /me

- **Where:** `server/src/db.ts:129-147`, `index.ts:198`, `links.ts:182`, `links.ts:139-144`, `index.ts:383-391`, `hub.ts:178`
- **Verification:** confirmed (reachability: confirmed; impact: confirmed; code: confirmed)

**Problem:** authenticate() in server/src/db.ts:129-147 selects only `user_id, last_used` from `sessions` and never looks at `users.banned_at`. The only ban checks are at index.ts:198 (new sign-ins) and links.ts:182 (`createShare`). `live()` at links.ts:139-144 does not consider whether the share owner is banned, and `putPoster` has no ban check. `deleteAccount` at index.ts:383-391 runs `DELETE FROM users WHERE id = ?`, which removes `banned_at` along with the row, so the next Discord login creates a fresh account through `upsertDiscordUser`. `closeAll(reason)` at hub.ts:178 is only ever called from `deleteAccount`; there are no admin/moderation HTTP endpoints (moderation is done by hand with `wrangler d1 execute`), so nothing runs `Hub.closeAll` or deletes sessions when someone is suspended. A suspended user keeps a working session, presence, relays, friend requests, live share links/media and poster swaps; only `POST /shares` is refused.

**Scenario:** Repro with seeded user C (`banned_at` set, live session): `GET /me` -> 200; WebSocket hello -> C's presence ('Banned Game') reached friend A; a relay from C was delivered to A. On banned user B: the existing share page and media still return 200 and `PUT /shares/:id/poster` -> 200; only `POST /shares` -> 403. `DELETE /me` by C -> 200 and the users row (including `banned_at`) is gone. Scenario: someone shares abusive clips and the operator sets `banned_at`; their 180-day session keeps working, they keep using presence, sending clips to friends and sending friend requests, their live links stay up and they can swap a link's poster (the `og:image` Discord embeds show) to any JPEG; then they run `DELETE /me`, sign in again with the same Discord account and get a clean account with fresh quota. Capability needed: a suspended user who still holds a token (the normal case, because nothing deletes sessions on suspension).

**Fix:** In db.ts authenticate: `SELECT s.user_id, s.last_used, u.banned_at FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.token_hash = ?`; `if (!s || now - s.last_used > SESSION_IDLE_MS) throw new HttpError(401, ...)`; `if (s.banned_at !== null) throw new HttpError(403, "This account is suspended.")`. In links.ts live(): `... FROM shares s JOIN users u ON u.id = s.owner WHERE s.id = ? AND u.banned_at IS NULL`. Keep bans outside `users` (a `banned_discord_ids` table checked in `authCallback` before the upsert), or refuse `DELETE /me` while banned, or keep a minimal row. Add a cron 'suspension apply' step: find users with `banned_at` set and `suspended_applied_at IS NULL`, delete their sessions, take down their shares (R2 + tombstone), and call `HUB.getByName(id).closeAll("suspended")` (avoids a new admin HTTP surface). Also check the ban in `putPoster`.

**Verifier on the fix:** The JOIN in authenticate is right and cheap, and it closes the DELETE /me path too, since auth runs before routing. Return 401, not 403: friends.rs:430 and links.rs:248 only sign out on 401. With a 403 the app would retry the WebSocket every 60 s forever (friends.rs:382-409), and links.rs:259 would show 'This link was made with another Discord account'. A 401 makes the app sign out, and the next login then shows the existing 'Account suspended' page. Open hibernated WebSockets keep presence and relays going no matter what authenticate does, so the hourly cron step is still needed: delete the…

### SV-12 · Medium · A 60-second Hub alarm per online user burns the Free-plan Durable Object request quota

- **Where:** `hub.ts:33,76,135`
- **Verification:** confirmed (code: confirmed)

**Problem:** The Hub arms a 60-second alarm per online user (hub.ts:33,76,135). Alarm invocations count as Durable Object requests: 1,440 per day for each always-online user.

**Scenario:** Against the Free-plan 100k Durable Object requests per day, that caps the service at roughly 70 always-online users, before counting presence RPCs. Severity: Medium on Free plan.

**Fix:** Sweep every 5 minutes, or skip re-arming when every socket was auto-pinged recently.

**Verifier on the fix:** A longer sweep interval (for example 5 minutes, and STALE_MS still decides what is stale) cuts alarm requests about 5x for a slower cleanup of half-open sockets. The fix is sound. Skipping the re-arm altogether would leave half-open sockets showing as online forever, so keep at least a slow sweep.

### SV-13 · Medium · Report handling: reports never surfaced, owner can destroy evidence, clips expire unreviewed, reports forgeable cross-site

- **Where:** 
- **Verification:** partial (code: partly confirmed)

**What verification found:** Points (1) and (3) are accurate. Reports go into D1 and nothing reads, surfaces or resolves them. Expired clips are swept after 5 days whatever their report status. Point (2) overstates the single-delete path. DELETE /shares/:id destroys the video, poster, title, game and tags, but the shares row keeps `owner`, so the uploader can still be identified for about 14 days after the clip was created. The worse case, which the reviewer missed, is DELETE /me: it wipes the R2 objects and cascades away the shares rows, so neither the content nor the uploader identity survives, while the report row stays for 30 days pointing at nothing. The same orphaning happens to any report reviewed more than 14 days after upload. Point (4) is real but low-impact. Cross-site report POSTs are possible and the limit is per full IP address (rotatable inside an IPv6 /64), but an attacker can just as well send reports directly. Forged reports trigger nothing and are capped at 20 open reports per clip. The real harm is that a flood can fill that cap, after which genuine reports are dropped silently while still answered `{ok:true}`.

**Original claim:** (1) Reports filed via `POST /c/:id/report` are never surfaced to anyone; the operator has to poll D1. (2) The owner can delete a reported clip (`deleteShare` wipes it from R2 and blanks the title), which destroys the evidence. (3) Clips expire after 5 days whether or not anyone reviewed them. (4) Any website can make its visitors' browsers file reports (no-cors `text/plain` POST); the rate limit (`REPORT_LIMIT`, 5/min) is per IP and per Cloudflare location, and IPv6 rotation gets around it.

**Scenario:** An abusive clip is reported but nobody is notified; the owner deletes it (destroying the evidence) or it expires after 5 days unreviewed. A malicious website silently makes its visitors' browsers file reports, and IPv6 rotation bypasses the per-IP limit. Severity: Low-Medium.

**Fix:** Hide a clip automatically after N distinct-IP reports, notify the operator (webhook or email), and move reported objects to a quarantine prefix instead of deleting them.

**Verifier on the fix:** Operator notification (a webhook or email on the first open report), quarantining reported objects instead of deleting them, and blocking owner and account deletion and sweep from destroying objects with open reports all address real gaps. Auto-hiding after N distinct-IP reports is the risky part: it is exactly what point (4) lets a malicious site trigger, so it would need an Origin check, a /64-keyed limit and a high threshold before it is safe.

### SV-2 · Medium (reported High) · POST /t/ping is unauthenticated and unthrottled; each fresh-UUID ping writes ~6 D1 rows kept 400 days, exhausting the shared daily write quota

- **Where:** `telemetry.ts:83-111`
- **Verification:** confirmed (impact: partly confirmed; reachability: confirmed; code: confirmed)

**What verification found:** The code claim holds. POST /t/ping is unauthenticated and has no rate limit. Each accepted ping writes D1 rows: about 6 with indexes for a new id, fewer for a repeated id. A new id's rows stay up to 400 days. On the Free D1 plan, roughly 17k scripted pings a day could use up the daily write allowance. The online features (sign-in, share links, friends) would then fail until 00:00 UTC, and the analytics tables can be filled with fake installs. Severity is overstated, though. Local recording, the core product, is not affected. The outage resets every day and loses no data. Ping is also not the only open write path: GET /auth/start (index.ts:136-143) writes to `logins` on every call with no throttle. And on the Workers Free plan, 100k requests a day to any endpoint already take the API down, so ping only makes a DoS that is already cheap about 6x cheaper. That makes this Medium (a DoS of secondary online features plus statistics pollution), not High.

**Original claim:** `POST /t/ping` (telemetry.ts:83-111) has no rate limit; the only check is the body shape. Each ping with a fresh random UUIDv4 inserts into `installs` and `install_days`; counting their PK and secondary indexes, that is roughly 6 D1 'rows written' per request, and the rows are kept 400 days. The D1 database and its Free-plan daily limits (about 100k rows written, 5M rows read) are shared by the whole app.

**Scenario:** A script sends ~20k pings per day (about one every 4 s) and exhausts the free D1 write allowance. After that every D1 write fails until 00:00 UTC: sign-in, `sessions.last_used` touches, uploads and friend actions. On a paid plan, unbounded growth and billing take the place of the outage. No capability is needed. Confirmed in code; the platform limits (Free plan inferred from code comments) are a 'likely' assumption. Severity: High on Free plan, Medium on paid.

**Fix:** Add a rate-limit binding `PING_LIMIT` keyed by `CF-Connecting-IP` (for example 5/60 s), as `REPORT_LIMIT` already does without storing the address. Turn off `workers_dev`, or WAF rules can be bypassed through it. Optionally drop pings whose `version` is not a known release.

**Verifier on the fix:** A per-IP `PING_LIMIT` rate-limit binding is correct and cheap, and it caps D1 writes from one address. On its own it is not enough. Blocked requests still count toward the Workers Free daily request quota, distributed sources get around a per-IP key, and /auth/start stays an open write path, so it needs the same limiter (or a shared `ANON_WRITE_LIMIT`). Turning off workers_dev would break the friends feature for old builds that still use the workers.dev host (wrangler comment). A smaller fix: in the Worker, refuse /t/ping unless the host is api.clippiboy.com, which is safe because every pingi…

### SV-3 · Medium (reported High) · GET /auth/start is unthrottled and runs a full-table DELETE on logins (no created_at index), so D1 reads grow quadratically

- **Where:** `index.ts:139-143`
- **Verification:** confirmed (impact: confirmed; code: confirmed; reachability: confirmed)

**Problem:** Each unauthenticated `GET /auth/start` (index.ts:139-143) runs `DELETE FROM logins WHERE created_at < ?`, and `logins` has no index on `created_at`, so every request scans the whole table. It then runs an INSERT that touches the table plus 2 unique indexes. There is no rate limit. R requests inside 10 minutes cost about R^2/2 rows read.

**Scenario:** 3,000 requests inside 10 minutes cost about 4.5M rows read, which is close to the whole Free-plan daily D1 read allowance (5M) shared by the entire app. No capability is needed. Severity: High on Free plan, Medium on paid.

**Fix:** Add an `AUTH_LIMIT` rate-limit binding keyed by `CF-Connecting-IP`. `CREATE INDEX logins_created ON logins(created_at)`, and move the logins cleanup into the cron. Turn off `workers_dev` so WAF rules cannot be bypassed.

**Verifier on the fix:** Adding an index on logins(created_at) in a new migration, or moving the DELETE to the hourly cron, removes the quadratic term on its own; either one is the minimal sufficient fix. authCallback (index.ts:173) and authExchange (index.ts:219) already re-check LOGIN_TTL_MS, so rows that expire but linger until the cron are harmless. A per-IP AUTH_LIMIT binding is useful defence in depth against linear flooding, but it is not sufficient alone, because attackers can rotate IPs and the binding is counted per location. Turning off workers_dev would break pre-domain builds that README.md:7 says still…

### SV-38 · Medium (reported High) · Privacy page: suspension promise ('signed out ... links are taken down') is not implemented

- **Where:** 
- **Verification:** confirmed (reachability: confirmed; code: confirmed; impact: confirmed)

**Problem:** The privacy page says: "A suspended account is signed out, cannot sign in or share links, and its links are taken down." Only 'cannot sign in again' and 'cannot create links' are implemented. Existing sessions, sockets, relays, live links and poster swaps keep working, and the ban disappears on `DELETE /me` (same root cause as SV-1).

**Scenario:** Repro: a banned user's session, presence, relays, live links and poster PUT all kept working, and `DELETE /me` removed the ban.

**Fix:** Implement the promise (see SV-1 fix) or change the wording.

**Verifier on the fix:** Either fix works. This is the same root cause as SV-1 and should be merged with it, so the code fix lands once: check banned_at in authenticate, filter live() by owner ban, check the ban in putPoster, add an hourly cron step that, for newly banned users, deletes their sessions, takes their shares down (R2 delete plus TOMBSTONE) and calls HUB.getByName(id).closeAll('suspended'), and keep bans outside the deletable users row (or refuse DELETE /me while banned). If the code fix is not ready, a minimal stopgap is to reword privacy.html:101 to match what is enforced ('cannot sign in again or creat…

## Reported, not independently re-verified

The original reviewer checked these against the code, but the independent verification pass was stopped before reaching them. Treat as likely, confirm before fixing.

- **SV-14 · Medium · The download endpoint trusts release.setup.url without pinning it** — `site-proxy/index.ts:61`  
  site-proxy/index.ts:61 streams whatever URL `release.json` (read from Pages) names in `release.setup.url`, served as `ClippiBoy_<v>_x64-setup.<ref>.exe` from clippiboy.com, without validating the URL origin or the version format.  
  *Fix:* Require `setup.url` to start with `https://github.com/EinFabo/clippiboy/releases/download/` and `version` to match `^\d+\.\d+\.\d+$`.
- **SV-43 · Medium · Privacy page: share links say 'display name and avatar' but also expose the Discord user ID** —   
  The privacy page says share links show 'display name and avatar'. The avatar URL also exposes the Discord user ID (same issue as SV-10).  
  *Fix:* Proxy or replace the avatar, or disclose it (see SV-10).
- **SV-6 · Medium · Hub Durable Object: unlimited sockets per user, no rate limit on hello/presence, relay budget is per connection** — `hub.ts:66`, `hub.ts:248-252`, `hub.ts:89-98` (+1 more)  
  `acceptWebSocket` at hub.ts:66 has no cap on sockets per user. The relay budget (intended 30 per 10 s) lives in `this.relayed`, keyed per WebSocket (hub.ts:248-252), so more sockets multiply it. Every `hello` or `presence` message (hub.ts:89-98) runs `sendSnapshot`, then `publish`: a Durable Object storage write, a D1 `friendIds` query, and one RPC to each friend's Hub, which in turn sends to every one of that friend's sockets; there is no rate limit on these messages. Every relay does a D1 `areFriends` lookup (hub.ts:256), even when the result is 'not a friend'.  
  *Fix:* Cap sockets per user (e.g. close the oldest beyond 5). Apply one per-connection budget to all message types and close the socket with code 4008 on abuse. Keep the per-user relay budget in `ctx.storage` or in memory per user, not per socket. Cache the friend set in the Hub, invalidated by `friendsChanged`, so relays do not hit D1. Coalesce `publish…
- **SV-7 · Medium · 'Appear offline' can be detected through a relay probe** — `hub.ts:158-162`  
  `relay()` at hub.ts:158-162 delivers to every `ready` socket and returns true without checking `attachment.invisible`, so the sender only gets `relayFailed: "offline"` when the target has truly no sockets. An invisible user is therefore distinguishable from an offline one, contradicting the privacy page promise: "With 'Appear offline', friends see you as offline."  
  *Fix:* If `this.current() === null` (invisible or offline), do not deliver and answer "offline". Alternatively, have the app opt in per relay. Either way the answer must look the same.
- **SV-8 · Medium · Unbounded request bodies are read into memory, including on unauthenticated routes** — `util.ts:16-22`, `telemetry.ts:85-87`, `links.ts:324`  
  `body()` at util.ts:16-22 calls `request.json()` with no size limit; it is used by `/auth/exchange` (unauthenticated), `/me` PATCH and `/friends/request`. At telemetry.ts:85-87 the size check relies on Content-Length, which is absent in chunked uploads, then calls `request.text()`. links.ts:324 does `JSON.parse((await request.text()).slice(0, 2048))`, reading everything and then slicing. `putPoster` already guards against this (see its comment at links.ts:276), but these routes do not.  
  *Fix:* A shared `readLimited(request, max)` that streams with `getReader()`, cancels and returns 413 once `max` is passed, and rejects when Content-Length exceeds `max`. Validate that the parsed JSON is an object (this also fixes the null-body 500).
- **SV-9 · Medium · A few throwaway accounts can keep the global share bucket 'full' with abandoned pending uploads** — `shares.ts:18`, `links.ts:57`, `links.ts:252`  
  shares.ts:18 sets `TOTAL_BYTES = 9 GB` with `WEEKLY_LIMIT = 3` per account. At links.ts:57 a pending upload holds its declared `bytes` for up to 30 minutes, and an aborted upload is deleted (links.ts:252), so it does not use up a weekly slot. Reservations can be re-opened repeatedly at no quota cost.  
  *Fix:* Count a pending reservation against the week even after it is abandoned (keep the row with `aborted = 1` instead of deleting it). Cap how many bytes a single account can have live. Alert on a full bucket. Consider a per-IP rate limit on `POST /shares`.

## Verified lower-severity issues

| ID | Severity | Title | Where | Fix |
|---|---|---|---|---|
| SV-10 | Low (reported Medium) | Share pages reveal the uploader's Discord user ID via the avatar URL | db.ts:88, page.ts:116 | Proxy the avatar through a route such as `/c/:id/avatar`, or use the default avatar, or disclose it. |
| SV-11 | Low (reported Medium) | The e2e script seeds known bearer tokens into the production database | scripts/e2e.mjs:11-21, server/scripts/e2e.mjs:4-6 | Generate random tokens at seed time and print them; default to a local `wrangler dev`; give the seeded rows a short `last_used` expiry or a cleanup cron. |
| SV-15 | Low | A null or non-object JSON body gives a 500 | util.ts:16-22, index.ts:212 | Have `body()` reject anything that is not an object; check `typeof code === "string"`. |
| SV-16 | Low | Two users sending each other friend requests at the same moment get a 500 | index.ts:326-342 | `INSERT ... ON CONFLICT(user_a, user_b) DO UPDATE SET status='accepted' WHERE requester <> excluded.requester`, then re-read the row. |
| SV-17 | Low | Presence can arrive at friends out of order | hub.ts:219-237 | Add a sequence number or `at` to `friendPresence` and have receivers drop older ones; or serialize `publish` through a promise chain. |
| SV-18 | Low | Media Range and conditional request handling is wrong (bad ranges return 206 full body, If-None-Match ignored, HEAD reads body) | links.ts:383-406 | Parse `Range` yourself; return 416 if `start >= size` and 200 when the range is unparseable; pass `onlyIf: request.headers` and return 304; use `CLIPS.head()` for HEAD. |
| SV-19 | Low | Media is publicly cacheable for up to an hour after deletion, and not marked noindex | links.ts:395 | `private, max-age=300` or `no-cache` with an ETag, and `X-Robots-Tag: noindex` on media and on the gone page. |
| SV-23 | Low | No 'sign out everywhere' |  | Add `DELETE /sessions`. |
| SV-24 | Low | Logout and suspension do not close an already-open WebSocket | index.ts:82-91 | Call `closeAll` on logout of the last session or on suspension. |
| SV-26 | Low | sweep runs its cleanup steps in sequence, so one R2 error skips the rest including the 30-day report deletion | links.ts:419-448 | Wrap each step on its own. |
| SV-28 | Low | The MP4 checker has no cap on box count (CPU exhaustion) | shares.ts:173-216 | Refuse files with more than about 100k top-level boxes. |
| SV-30 | Low | Friend requests: no rate limit or pending cap, and 404 vs 403 responses enable user enumeration | index.ts:313-345 | Rate-limit; cap outgoing pending requests at around 20; return the same answer for 403 and 404. |
| SV-31 | Low | Usernames are only synced at login, so two users rows can share a username after a Discord rename | db.ts:93-119 | Make `username` unique and sync it on use. |
| SV-32 | Low | Posters can be swapped any number of times | links.ts:273-289 | Allow one poster PUT per share, or a few at most. |
| SV-39 | Low | Privacy page: usage stats called 'anonymous' are actually pseudonymous |  | Either commit to never issuing per-person refs, or say 'pseudonymous'. |
| SV-4 | Low (reported High) | Site proxy GET /get/<ref> lets any browser-UA client add D1 writes to the shared database, with unbounded new refs kept 400 days | site-proxy/index.ts:67-75 | Add a rate-limit binding or a WAF rule for `/get` in the site proxy; optionally cap distinct refs per day. |
| SV-40 | Low | Privacy page: 'does not store your IP address' may be undermined by invocation logs | wrangler.jsonc:8, server/wrangler.jsonc | Consider turning invocation logs off or excluding `/t/ping`. |
| SV-41 | Low | Privacy page: 'database is located in Western Europe' unverifiable; Hub DO storage not pinned to EU |  | Not stated explicitly; implied: pin the Hub namespace with `jurisdiction("eu")` and verify D1/R2 locations, or adjust the wording. |
| SV-46 | Low | Spec: 'no public counting route anyone could call' contradicted by /get/<ref> |  | Rate-limit or otherwise protect `/get` counting (see SV-4), or update the spec. |
| SV-47 | Low | Missing tests: auth flow (state mismatch, code reuse, expiry, verifier, bans) |  | Add these tests with @cloudflare/vitest-pool-workers or Miniflare. |
| SV-48 | Low | Missing tests: authorization matrix |  | Add integration tests for these cases. |
| SV-49 | Low | Missing tests: upload quota race and upload edge cases |  | Add integration tests for these cases. |
| SV-5 | Low (reported High) | Hourly sweepUsage does a full scan of installs because installs.last_day has no index | telemetry.ts:119 | `CREATE INDEX installs_last_day ON installs(last_day)`, and run `sweepUsage` once a day. |
| SV-51 | Low | Missing tests: share page rendering and response headers |  | Add a snapshot test with hostile inputs and header assertions. |
| SV-52 | Low | Missing tests: Hub behaviour |  | Add Durable Object integration tests for these cases. |
| SV-53 | Low | Missing tests: body limits |  | Add tests for these cases. |
| SV-54 | Low | Missing tests: sweep |  | Add integration tests for the sweep steps. |
| SV-55 | Low | Missing tests: Range and conditional requests |  | Add tests for these cases. |
| SV-56 | Low | Missing tests: site proxy |  | Add tests for these cases. |
| SV-20 | Info (reported Low) | Missing CSP, frame-ancestors, Referrer-Policy and no-store headers on share and sign-in pages | links.ts:372, index.ts:242 | `Content-Security-Policy: default-src 'none'; script-src 'sha256-…'; style-src 'unsafe-inline'; img-src https://api.clippiboy.com https://cdn.discordapp.com; media-src https://api.clippiboy.com; font-src https://clippib… |
| SV-21 | Info (reported Low) | Idle sessions are rejected after 180 days but never deleted |  | Have the cron delete stale sessions. |
| SV-22 | Info (reported Low) | logins rows (user_id + one-time code) are only cleaned up on the next /auth/start | index.ts:139-143 | Have the cron delete stale logins. |
| SV-25 | Info (reported Low) | Cron uses Promise.all inside waitUntil, so a sweep rejection can cut sweepUsage short | index.ts:45 | Use `Promise.allSettled`. |
| SV-27 | Info (reported Low) | Account deletion fails for accounts with more than 500 live links | links.ts:305 | Delete in chunks of 1,000 keys. |
| SV-29 | Info | Note: MP4/HTML polyglots are accepted; safety relies on fixed video/mp4 type plus nosniff | shares.ts:173-216 | Keep the fixed `video/mp4` type and `nosniff` (and document the polyglot case in tests). |
| SV-33 | Info (reported Low) | Expiry rounding overstates the time left | shares.ts:98 | Use `Math.floor` for days, and show hours below 2 days. |
| SV-34 | Info (reported Low) | workers_dev: true lets anyone bypass zone-level WAF and rate-limit rules |  | Turn off `workers_dev`. |
| SV-35 | Info (reported Low) | QUICK_LINKS_FOR is read from env through a cast and missing from secrets.d.ts | links.ts:67 | Not stated explicitly; implied: declare `QUICK_LINKS_FOR` in `secrets.d.ts` and drop the cast. |
| SV-36 | Info (reported Low) | The site proxy is not covered by npm run check |  | Not stated explicitly; implied: include the site proxy in the type check. |
| SV-42 | Info (reported Low) | Privacy page: 'No cookies' while api.clippiboy.com sets cb_login |  | Reword the privacy page to mention the strictly necessary sign-in cookie. |
| SV-44 | Info (reported Low) | Privacy page: 'Delete account removes everything immediately' is not fully true |  | Shorten/privatise media caching and sweep logins in the cron (see SV-19, SV-22), or adjust the wording. |
| SV-45 | Info (reported Low) | README: sessions 'expire after 180 days' but rows are kept |  | Delete stale sessions in the cron (see SV-21). |
| SV-50 | Info (reported Low) | Missing tests: Mp4Boxes edge cases |  | Add unit tests for these cases. |
| SV-57 | Info (reported Low) | Missing tests: expiresIn boundaries |  | Add unit tests at these boundaries. |

## Unverifiable from source or disputed

| ID | Reported | Title | What is uncertain |
|---|---|---|---|
| SV-37 | Low | Site proxy Location rewrite can produce a protocol-relative URL | server/site-proxy/index.ts:37-41: `const location = response.headers.get("Location"); if (location?.startsWith(ORIGIN)) {...} else if (location?.startsWith("/clippiboy")) { response.headers.set("Location", location.slice("/clippiboy".length) \|\| "/"); }`. A Lo… |

## What is solid

- `npx tsc --noEmit` passes and `npm test` passes 25/25.
- All SQL uses bound parameters; the only interpolations are constant fragments.
- The share quota INSERT…SELECT is atomic and cannot be raced (repro: 6 parallel uploads from one account gave 3x200 and 3x429).
- The sign-in flow is sound: the `cb_login` state cookie (HttpOnly, Secure, SameSite=Lax, Path=/auth), single-use code (spent on the first try) and S256 verifier check (with `sameText`) together stop both login CSRF and deep-link hijack; the 256-bit bearer token is stored only as a hash.
- Owner checks hold (repro: account A deleting B's share gets 403).
- Friendship is checked before every relay, and `X-User` is overwritten by the Worker before handing off to the Hub.
- Durable Object RPC methods (`card`, `friendPresence`, `relay`, `friendsChanged`, `closeAll`) can only be called from Worker code.
- There are no admin or moderation HTTP endpoints, so there is no operator-auth surface to attack.
- Share page escaping is correct (repro: `<script>`, `<img onerror>` and `</title>` payloads in title, game and tags render as entities).
- Media is served with a fixed `video/mp4` or `image/jpeg` type and `nosniff`.
- Errors never leak stack traces.
- Site proxy: the upstream host (`https://einfabo.github.io/clippiboy`) is fixed, so it is not an open proxy and no SSRF was found; request headers are not forwarded upstream, so there is no header-based cache poisoning; non-GET/HEAD requests get 405.
- `putPoster` already bounds its body size (JPEG of at most 1 MB).
- `REPORT_LIMIT` rate-limits reports per IP without storing the address; reports are capped at 20 open per share.
- MP4 box parsing is otherwise correct: largesize is handled, `size < header` is refused, and a first box other than `ftyp` (or one over 1 KB) is refused.
- Consistent with the privacy page: downloads store only `(day, ref, count)` and the proxy has no observability configured; the 400-day retention works (the cutoff keeps 400–401 days); report rows hold no reporter or IP; relays store nothing (`console.error` logs user IDs only on errors); only the `identify` Discord scope is used; tombstones drop title, game and tags and are purged after 2 weeks.
- The privacy-page statement that the server does not store the IP address is true for D1.

## Improvement opportunities and missing features

| ID | Idea | Why | Effort | Sketch |
|---|---|---|---|---|
| SV-P-1 | Integration test harness with real D1, R2 and Durable Objects | Today's 25 tests only cover pure functions; an integration harness would let the auth, authorization, quota, Hub, sweep, body-limit and Range behaviours be tested and keep the security fixes from reg… | M | Use `@cloudflare/vitest-pool-workers` or Miniflare with real D1, R2 and Durable Object bindings, and add the 11 test areas listed in the report's 'Most valuable missing tests' section. |
