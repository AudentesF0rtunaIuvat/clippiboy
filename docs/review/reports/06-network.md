ClippiBoy network and peer slice: read-only security and robustness review

Scope: I read all of share.rs, friends.rs, links.rs, telemetry.rs, tools.rs and updater.rs, plus the server files hub.ts, index.ts, links.ts and telemetry.ts, the privacy page, the usage-stats design doc and tauri.conf.json. I also briefly checked control.rs, the local Stream Deck port, and found nothing there. Where a finding depends on library behaviour I checked the crate sources in ~/.cargo: iroh 1.2.0, tauri-plugin-updater 2.10.1, tungstenite 0.30 and reqwest 0.13.4. Nothing in the repo was modified.

There are no Critical findings. The checks that matter most are in place and hold up:
- **Peer file names and game names:** they cannot escape the clip folder (`safe_file_name` plus `filing::folder_name` strip separators, `:`, `..` and force a known extension).
- **Transfer secret:** compared in constant time.
- **Sign-in:** the PKCE-style verifier plus the state cookie on the server are correct.
- **Updater:** the minisign signature is checked before install.
- **ffmpeg download:** each program's SHA-256 is pinned in the binary and checked before the files are moved into place. Entries are read by exact name, so there is no zip-slip.
- **Share-link upload:** streamed from disk, never whole in memory.
- **Server data in the WebView:** no `dangerouslySetInnerHTML` anywhere; the CSP limits images to `cdn.discordapp.com`; the app never opens a URL the server sends.

## Trust-boundary map
1. **Any Discord user → server → app.** Friend requests, display names and avatar URLs (built by the server from Discord's hash) end up in React views, overlay banners, in-app toasts and Windows notifications.
2. **Accepted friend → hub relay → `share::handle_relay`.** Limits: 4 KB, 30 messages per 10 s per socket, friendship checked, `from` set by the server. The body is entirely the friend's: file name, title, game, sizes, sha256, secret, `EndpointAddr`, tracks, mix.
   - Without the user doing anything: a map entry, a sound, an in-game banner and a notification.
   - After the user accepts: the app dials whatever `EndpointAddr` the friend sent (NodeId, IP:ports, relay URL — iroh opens any relay URL, not just ones it knows). It writes the bytes to the clip folder and to `%APPDATA%\ClippiBoy\tracks`, checks the SHA-256 against the friend's own claim, runs ffmpeg for the thumbnail, adds the clip to the library tagged "from <name>", and later plays it in WebView2 or opens it via ShellExecute.
3. **Anyone who knows our NodeId (per-run, ephemeral)** — friends we offered to, the server, n0's relays — can reach the iroh router (ALPN `clippiboy/share/1`). `serve()` reads a ≤4 KB JSON ask, checks id and secret, then streams the file.
4. **The friends server (`api.clippiboy.com`).** HTTPS via reqwest with the platform verifier, WSS via tungstenite with native roots and its default 64 MiB message cap. The app trusts it for:
   - identity (`from`), presence and friend lists;
   - share-link ids, URLs and quota;
   - signing the user out (401, close code 4001).

   It also sees every relay body, including secrets and addresses.
5. **Websites or local processes → `clippiboy://auth?code=`** (deep link or single-instance argv) → `handle_url` → `/auth/exchange` with the verifier held in memory.
6. **GitHub Releases.**
   - The ffmpeg zip (TLS plus per-file hashes) lands in `%LOCALAPPDATA%\gg.clippiboy.app\ffmpeg\9.0.1`; after that, a file being present is enough to trust it.
   - For updates, `latest.json` is unsigned and the installer is minisign-verified; it runs as a passive, per-user NSIS install.
7. **App → `/t/ping`.** No auth, a random UUID and counts. Outbound only.
8. **n0 (pkarr DNS and relays).** Sees NodeIds and IP addresses.
9. **Same-user local processes** (control port token, `friends.json`, the ffmpeg folder, `links.json`). This is not a privilege boundary.

## Findings, most severe first

### Medium

**1. [Medium] [security/privacy] The P2P handshake fully trusts the server: an active server can fetch or swap any clip — share.rs:338, 886, 935–939**
- The offer carries everything needed to download: `Signal::ShareOffer { id, size, sha256, secret, addr, meta }`.
- The sender lets in anyone with the secret, before any answer: `let open = matches!(entry.transfer.stage, Stage::Asking | Stage::Moving); if !same(entry.secret…) || !open` ("Dialling in is their yes"). It never checks `connection.remote_id()`.
- The keys are new every run (`Endpoint::bind(presets::N0)`), so nothing can be pinned.
- Scenario (needs the server operator, or whoever compromises the Worker or Cloudflare account):
  - While the offer is in its 60 s Asking window, the server dials the sender and downloads the clip without the friend ever accepting.
  - Or it replaces `addr` and `sha256` with its own endpoint and hash. The friend then receives the server's content, filed as "from <friend>", and the hash check passes.
- This breaks the privacy page's "never reaches our server" / end-to-end claim against anything other than a passive server. Confirmed.
- Fix:
  - Keep a persistent iroh secret key per install, protected with DPAPI.
  - The receiver puts its NodeId in `ShareAnswer`; the sender serves only after an accepting answer and only to that `remote_id()`.
  - Pin each friend's NodeId on first use and warn when it changes; optionally show a short safety code.
  - Reword the privacy page.

**2. [Medium] [security/bug] Size overflow in the offer lets a friend show "1.0 MB" and fill the disk — share.rs:565, 755, 816**
- `let total = size + meta.tracks.iter().map(|track| track.size).sum::<u64>();` — the release profile has no `overflow-checks`, so this wraps.
- Example: `size` = 1 MiB and `tracks[0].size` = 2^64−1 add up to "1.0 MB" in the banner. After the user accepts, `fetch_tracks` asks for track 0, the sender answers `rest` = 2^64−1, which passes `offset + rest != size`, and the receiver writes `…\tracks\<id>\0.m4a.part` until C: is full.
- The recorder and the OS suffer before the 6 retries give up and the folder is deleted.
- Even without the trick there is no upper limit on size and no free-space check anywhere in `receive`, unlike the 2 GB guard recordings have.
- Attacker: an accepted friend, plus one click on accept. Confirmed.
- Fix:
  - Use `checked_add` throughout.
  - Reject offers above a sane cap (e.g. clip ≤ 64 GiB, each track ≤ 2 GiB, `track.size` ≤ `size`).
  - Check free space on both volumes with `disk.rs` before accepting and while writing.
  - Never write past `size`.

**3. [Medium] [security/perf] An accepted friend can spam offers: banner, sound and toast each time, plus a slow UI — share.rs:582–603, model.rs:355, hub.ts:39–40, 248–253**
- Every offer inserts a map entry (an existing id is silently overwritten), plays `ClipReceived`, shows an overlay banner over the game and raises `Alert::Clip`, which is always wanted.
- The default is `accept_clips: AcceptClips::All`.
- The server's budget is per WebSocket and is lost when the Durable Object hibernates, and a user may open any number of sockets. So the rate is effectively unbounded.
- Each `emit()` sends the whole transfer list to three webviews, so cost grows roughly with the square of the number of live offers (entries stay about 60 s or more).
- Scenario: a friend scripts N sockets and floods offers during a match. The victim's only remedies are blocking them or changing settings. Confirmed in code; the size of the slowdown is likely, not measured.
- Fix:
  - Client side: at most 1–2 pending offers per friend and about 10 in total; reject ids that are already known (incoming or outgoing); one banner and one sound per friend per 30 s.
  - Server side: a persisted per-user budget and a cap on sockets per user.

**4. [Medium] [bug] An incomplete ffmpeg folder blocks setup for good and re-downloads every 30 minutes — tools.rs:166–168, 290, 47–52; muxer.rs:45–48**
- `if PROGRAMS.iter().all(|(name, _)| target.join(name).is_file()) { return Ok(()) }`, otherwise it downloads, then `std::fs::rename(&staging, target)`.
- If `9.0.1\` exists but `ffmpeg.exe` is gone (antivirus quarantining ffmpeg is a common false positive, or the user deleted it), the rename onto the existing non-empty folder fails on Windows: "could not put ffmpeg in place". It retries every 30 minutes, forever, pulling the full archive each time.
- Meanwhile `tool()` quietly falls back to `PathBuf::from("ffmpeg")`, i.e. the app folder or PATH. That is an untested and possibly old ffmpeg, which then also parses friends' files (see 5).
- Confidence: likely (based on Windows rename semantics).
- Fix: when the folder is incomplete, `remove_dir_all(target)` before the rename (or unpack under a fresh name); don't fall back to PATH silently; keep the 30-minute retry from becoming a download loop.

### Low–Medium

**5. [Low–Medium] [security] Files from friends go to ffmpeg unchecked, with format auto-detection, no whitelists and no timeout, on an async worker — share.rs:837–841, thumbs.rs:31–37, muxer.rs:65–68**
- The thumbnail is made with `ffmpeg -ss 0.5 -i <received file> …`, and `file_clip` runs it synchronously inside the tokio task.
- Unlike the share-link server (`Mp4Boxes`), the P2P path never checks that the file is an MP4. ffmpeg picks the demuxer from the content.
- The pinned build's HLS demuxer refuses an `.mp4` name, and concat runs with `safe=1`, which limits the obvious tricks. What remains: ffmpeg parser bugs, playlist-style demuxers (DASH, concat reading sibling files), and any old PATH ffmpeg from finding 4.
- Confidence: speculative on exploitability; the attack surface itself is confirmed.
- Fix:
  - For received files, add input options `-format_whitelist mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,image2,png_pipe,jpeg_pipe -protocol_whitelist file` (and to every later ffmpeg/ffprobe input of such files), or check the box structure as the server does.
  - Put a timeout and kill on `muxer::run`.
  - Run `file_clip` in `spawn_blocking`.

### Low

**6. [Low] [bug] Two concurrent receives with the same file name share one `.part` file — share.rs:643–647, 687, 782, 1163–1167**
- `free_name` only checks the final name, so both transfers get `X.mp4.part`. The second takes its resume offset from the first one's partly written file and truncates it (`set_len`).
- Rust opens files with `FILE_SHARE_DELETE`, so after the first one renames the file into place, the second keeps writing into the clip that was already verified and filed.
- `rename` also replaces anything that appeared at the target since, and `free_name` hands back the existing target after 999 name collisions.
- An offer re-sent with the same id overwrites an entry that is already Moving and puts it back to Asking, so it can be accepted twice.
- Trigger: the same clip offered twice and both accepted, or a malicious friend. Likely.
- Fix: a unique part name (`.<uuid>.part`); claim the final name atomically (`create_new`, or a move without replace, retrying on collision); reject duplicate offer ids.

**7. [Low] [bug] Stalls on several network paths never time out — share.rs:787, 794, 808, 918–926, 971; friends.rs:428; updater.rs:73–91**
- iroh sends a keep-alive every 5 s, so the QUIC idle timeout never fires while the peer process lives. The following can therefore hang forever if the other side goes quiet:
  - in `fetch`: `open_bi`, `read_exact(header)` and `recv.read`;
  - in `serve`: `accept_bi`, `read_exact(length)` and `send.write_all`.
- Cancel is only checked between chunks, so it cannot break a blocked call.
- The router's `JoinSet` is unbounded, so anyone who knows the NodeId can pile up idle connections.
- An outgoing transfer stays "Moving" with a working secret for good if the receiver disappears.
- `tokio_tungstenite::connect_async` has no timeout; a middlebox that stalls the TLS handshake freezes the friends reconnect loop until restart.
- The updater builder has no `.timeout()`, so one hung check stops all hourly checks.
- Confirmed.
- Fix: per-operation `tokio::time::timeout` (e.g. 30 s without progress); an expiry on outgoing secrets; a 15 s timeout on the WebSocket connect; a timeout on the updater builder.

**8. [Low] [privacy] Share links publish the friend's name on received clips — share.rs:866 (`tags: vec![format!("from {from}")]`), clips.rs:347–351, links.rs:546**
- `is_friend_tag` only matches a "with " prefix. So when the user shares a received clip as a public link, the tag "from <friend's Discord display name>" goes to the public page. The server-side `cleanTags` keeps it.
- This contradicts the stated intent ("who played along stays among friends, a link is public"; commit f041a3d). Confirmed.
- Fix: treat "from …" as a friend tag, or better, store friend and sender tags in a structured field.

**9. [Low] [security] The session token sits in plaintext in Roaming AppData — friends.rs:831–860**
- `std::fs::write(&path, json!({ "token": token }))` writes to `%APPDATA%\ClippiBoy\friends.json`.
- It is a bearer token that stays valid for 180 days without use (renewed with use), so it follows roaming profiles and backups and any same-user malware can take it.
- No token appears in the logs. Confirmed.
- Fix: protect it with DPAPI (`CryptProtectData`) or the Windows Credential Manager (keyring), and keep it in LocalAppData.

**10. [Low] [security] Any deep link uses up the pending sign-in — friends.rs:147–155, 289**
- `friends.verifier.lock().take()` runs before the exchange, for any `clippiboy://auth?code=<junk>`. A web page (or a local process) can therefore make a pending sign-in fail, and can pop error toasts when no sign-in is open.
- Login CSRF (the victim ending up signed into the attacker's account) is blocked by PKCE, unless the attacker also learns the victim's challenge (it travels in the URL query, so browser history or server logs) within 10 minutes. That part is speculative.
- Confirmed for the denial of service.
- Fix: keep the verifier until an exchange succeeds or a timeout passes; ignore links silently when no sign-in is in progress.

**11. [Low] [privacy] P2P exposes IP addresses, and friend-chosen endpoints get dialled — share.rs:313, 783**
- The offer contains `router.endpoint().addr()`: the sender's public and LAN IPs, sent to the friend before they accept.
- On accept, the receiver contacts IP:ports and a relay URL chosen by the friend (confirmed in iroh's `start_active_relay`, which ignores whether the URL is in the relay map). That reveals the receiver's IP to an attacker-run relay and sends QUIC packets to arbitrary hosts, LAN included.
- Fix: say this in the privacy page; offer only global addresses plus the n0 relay; ignore relay URLs outside the default map; consider a relay-only option.

**12. [Low] [bug] Temp files leak — share.rs:1013–1015, 1113; links.rs:461**
- The cut tracks in `%TEMP%\clippiboy-share\<id>\` are deleted only after a successful send (`o.tracks.drain(..)`). After Declined, Expired, Cancelled or Failed they stay forever, and nothing cleans them at startup.
- `.part` files from a crash stay in the user's clip folder or under `tracks\`.
- A failed shrink returns early via `??` and leaves `temp\link-<id>.mp4` behind.
- Confirmed.
- Fix: `impl Drop` for `Outgoing` (or clean up on every end state), plus a startup sweep of those folders.

**13. [Low] [quality] `safe_file_name` misses device names and bidi controls — share.rs:1176–1194**
- It doesn't apply the reserved-name rules `filing::folder_name` has, so `CON.mp4` / `NUL.mp4.part` are possible: on Windows ≤10 the transfer fails or writes into a device. `COM¹`-style names aren't handled by either function.
- It doesn't strip Unicode format characters (`c.is_control()` misses U+202E), so a name can be shown as "…4pm.png" in Explorer, though it stays a `.mp4`.
- Likely.
- Fix: reuse the reserved-name logic and strip the Cf bidi characters.

**14. [Low] [security] Updater downgrade through an unsigned version field — plugin updater.rs:530–532, 712**
- `release.version > self.current_version` comes from `latest.json`, which is unsigned, and the minisign signature covers only the installer bytes.
- Someone with write access to the GitHub release (but without the signing key) can label an old, signed, vulnerable installer as 99.0 and it installs silently in passive mode.
- Nothing in the app's own code skips verification. Likely; this is a known limitation of the Tauri updater.
- Fix: after download, check that the signature's trusted comment (`file:ClippiBoy_<ver>…`) matches `update.version`, or keep a minimum-version floor in the app.

**15. [Low] [security] The ffmpeg archive isn't hashed as a whole and nothing caps its size — tools.rs:221–257, 262–291**
- The download has no size cap, and `ZipArchive` parses the unverified file; entries are copied to disk before their hash is known.
- A compromised release could fill `%LOCALAPPDATA%` (a zip bomb or an endless stream). Nothing gets executed, though.
- Fix: pin the zip's own SHA-256 and check it before opening; cap each entry at its expected size.

**16. [Low] [quality] Several hardening gaps**
- `handle_relay` doesn't check that `from` is in the current friend list; it relies entirely on the server (share.rs:471).
- `link.id` from the server goes into URL paths unchecked (links.rs:489, 660). A malicious server returning `../me` turns the take-down into `DELETE /me`. That only matters for the server operator, who controls the account anyway.
- The outbox is an unbounded channel (friends.rs:438).
- The reconnect backoff has no jitter, so clients all come back together after a deploy (friends.rs:404–409).
- Unknown server messages are logged whole, up to 64 MiB (friends.rs:521).
- `.expect("HTTP client")` in three client builders would abort the whole app, ending any recording, if TLS setup ever failed.
- Any Discord user who knows a username can re-request right after being declined, which causes an in-game banner each time (index.ts `sendRequest`, friends.rs:360–366).
- Whether Windows toasts escape their XML (tauri-winrt-notification 0.7.3 is not available locally) is unverified. The text comes from friends (display name, game, offer title) but only reaches a toast when banners are off.

**Telemetry:** no security findings. Exactly the documented fields are sent (UUID v4, version, source, heard, three counts capped at 10,000, six feature bits). It uses its own client with no auth header and no shared connections, runs on its own thread with a 15 s timeout, and nothing goes out before the notice has been seen or while the switch is off. Turning it off deletes `usage.json`, turning it on again starts a new id, and a ping that was already in flight at the switch is not recorded.

## Privacy consistency check
These match:
- The ping contents, opt-out with the notice gate, delete-on-off and new-id-on-on, and the `usage.json` path, all as in design doc §3.
- "Without signing in, only GitHub": the only hosts in the code are github.com and api.clippiboy.com (the ping); iroh starts only on the first share.
- Avatars come only from `cdn.discordapp.com` (enforced by the CSP).
- "Show what I'm playing" and "Appear offline" behave as described.

Discrepancies:
1. **Friend names on public links:** received clips put "from <friend>" on share-link pages (finding 8), against the code's own promise.
2. **"Never reaches our server":** true only while the server behaves. The relay carries the download secret, the hash and the sender's full address, so the server can fetch or MITM (finding 1). The page also says "connection details of both devices" pass the server; in fact only the sender's do. The receiver's go directly to the friend.
3. **IP addresses:** the page doesn't say that your friend learns your public and LAN IPs (the sender's as soon as it offers), or that a friend-chosen relay host may be contacted (finding 11).
4. **"Our server does not store your IP address with it":** true for D1. But `wrangler.jsonc` has `"observability": { "enabled": true }`, so Cloudflare request logs (IP, location, time) also cover `/t/ping` for a few days. The friends section mentions Cloudflare logs; the usage-statistics section does not. Likely; worth confirming in the dashboard.
5. **"Only a hash is kept":** true on the server, but the app itself keeps the raw session key in plaintext in Roaming AppData (finding 9). Not a contradiction, but worth a line or DPAPI.
6. **"Appear offline":** the app still sends game and status to the server (kept in memory and on the socket, not passed to friends). Consistent with "friends see you as offline", but not stated.

Files most relevant: `src-tauri/src/share.rs`, `src-tauri/src/friends.rs`, `src-tauri/src/tools.rs`, `src-tauri/src/muxer.rs`, `src-tauri/src/thumbs.rs`, `src-tauri/src/clips.rs`, `src-tauri/src/links.rs`, `src-tauri/src/updater.rs`, `server/src/hub.ts`, `site/privacy.html`.
