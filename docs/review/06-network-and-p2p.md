# Networking: P2P sharing, friends, share links, ffmpeg download, updater, telemetry

**Scope:** share.rs, friends.rs, links.rs, telemetry.rs, tools.rs, updater.rs (client side).

**Findings:** 27 — 18 verified (0 medium or higher), 5 reported but not independently re-verified, 1 unverifiable/disputed, 3 rejected by verification.

Severity shown is the verified severity; where verification changed it, the reported one is in parentheses. See [README](README.md#how-findings-were-verified) for how verification worked.

## Reported, not independently re-verified

The original reviewer checked these against the code, but the independent verification pass was stopped before reaching them. Treat as likely, confirm before fixing.

- **NW-1 · Medium · P2P handshake fully trusts the server: an active server can fetch or swap any clip** — `src-tauri/src/share.rs:338`, `src-tauri/src/share.rs:886`, `src-tauri/src/share.rs:935-939` (+1 more)  
  The share offer relayed through the friends server carries everything needed to download: `Signal::ShareOffer { id, size, sha256, secret, addr, meta }`. The sender lets in anyone holding the secret, before any answer: `let open = matches!(entry.transfer.stage, Stage::Asking \| Stage::Moving); if !same(entry.secret…) \|\| !open` (comment: "Dialling in is their yes"). It never checks `connection.remote_id()`. iroh keys are new every run (`Endpoint::bind(presets::N0)`), so nothing can be pinned. The server sees every relay body (secret, sha256, sender address), so it can download or MITM the transf…  
  *Fix:* Keep a persistent iroh secret key per install, protected with DPAPI. The receiver puts its NodeId in `ShareAnswer`; the sender serves only after an accepting answer and only to that `remote_id()`. Pin each friend's NodeId on first use and warn when it changes; optionally show a short safety code. Reword the privacy page.
- **NW-2 · Medium · Size overflow in the offer lets a friend show "1.0 MB" and fill the disk; no size cap or free-space check on receive** — `src-tauri/src/share.rs:565`, `src-tauri/src/share.rs:755`, `src-tauri/src/share.rs:816`  
  `let total = size + meta.tracks.iter().map(\|track\| track.size).sum::<u64>();` — the release profile has no `overflow-checks`, so this wraps. With `size` = 1 MiB and `tracks[0].size` = 2^64-1 the banner shows "1.0 MB". After accept, `fetch_tracks` asks for track 0, the sender answers `rest` = 2^64-1, which passes the `offset + rest != size` check, and the receiver writes `…\tracks\<id>\0.m4a.part` until C: is full. Even without the overflow trick there is no upper limit on size and no free-space check anywhere in `receive`, unlike the 2 GB guard recordings have.  
  *Fix:* Use `checked_add` throughout. Reject offers above a sane cap (e.g. clip <= 64 GiB, each track <= 2 GiB, `track.size` <= `size`). Check free space on both volumes with `disk.rs` before accepting and while writing. Never write past `size`.
- **NW-3 · Medium · An accepted friend can spam offers: banner, sound and toast each time, plus a slow UI** — `src-tauri/src/share.rs:582-603`, `src-tauri/src/model.rs:355`, `server/src/hub.ts:39-40` (+1 more)  
  Every incoming offer inserts a map entry (an existing id is silently overwritten), plays `ClipReceived`, shows an overlay banner over the game and raises `Alert::Clip`, which is always wanted. The default is `accept_clips: AcceptClips::All`. The server's rate budget (30 messages per 10 s) is per WebSocket and is lost when the Durable Object hibernates, and a user may open any number of sockets, so the rate is effectively unbounded. Each `emit()` sends the whole transfer list to three webviews, so cost grows roughly with the square of the number of live offers (entries stay about 60 s or more).  
  *Fix:* Client side: at most 1-2 pending offers per friend and about 10 in total; reject ids that are already known (incoming or outgoing); one banner and one sound per friend per 30 s. Server side: a persisted per-user budget and a cap on sockets per user.
- **NW-4 · Medium · An incomplete ffmpeg folder blocks setup for good, re-downloads every 30 minutes, and silently falls back to PATH ffmpeg** — `src-tauri/src/tools.rs:166-168`, `src-tauri/src/tools.rs:290`, `src-tauri/src/tools.rs:47-52` (+1 more)  
  Setup does `if PROGRAMS.iter().all(\|(name, _)\| target.join(name).is_file()) { return Ok(()) }`, otherwise it downloads, then `std::fs::rename(&staging, target)`. If `9.0.1\` exists but `ffmpeg.exe` is gone (antivirus quarantine is a common false positive, or the user deleted it), the rename onto the existing non-empty folder fails on Windows with "could not put ffmpeg in place". It retries every 30 minutes forever, pulling the full archive each time. Meanwhile `tool()` quietly falls back to `PathBuf::from("ffmpeg")` (app folder or PATH), an untested and possibly old ffmpeg that then also pars…  
  *Fix:* When the folder is incomplete, `remove_dir_all(target)` before the rename (or unpack under a fresh name); don't fall back to PATH silently; keep the 30-minute retry from becoming a download loop.
- **NW-5 · Medium · Files from friends go to ffmpeg unchecked, with format auto-detection, no whitelists, no timeout, on an async worker** — `src-tauri/src/share.rs:837-841`, `src-tauri/src/thumbs.rs:31-37`, `src-tauri/src/muxer.rs:65-68` (+2 more)  
  The thumbnail for a received clip is made with `ffmpeg -ss 0.5 -i <received file> …`, and `file_clip` runs it synchronously inside the tokio task. Unlike the share-link server (`Mp4Boxes`), the P2P path never checks that the file is an MP4; ffmpeg picks the demuxer from the content. The pinned build's HLS demuxer refuses an `.mp4` name and concat runs with `safe=1`, which limits the obvious tricks, but ffmpeg parser bugs, playlist-style demuxers (DASH, concat reading sibling files) and any old PATH ffmpeg (NW-4) remain. `muxer::run` has no timeout/kill.  
  *Fix:* For received files add input options `-format_whitelist mov,mp4,m4a,3gp,3g2,mj2,matroska,webm,image2,png_pipe,jpeg_pipe -protocol_whitelist file` (and to every later ffmpeg/ffprobe input of such files), or check the box structure as the server does. Put a timeout and kill on `muxer::run`. Run `file_clip` in `spawn_blocking`.

## Verified lower-severity issues

| ID | Severity | Title | Where | Fix |
|---|---|---|---|---|
| NW-10 | Low | Share links publish the friend's name on received clips | src-tauri/src/share.rs:866, src-tauri/src/clips.rs:347-351 | Treat "from …" as a friend tag, or better, store friend and sender tags in a structured field. |
| NW-11 | Low | Session token stored in plaintext in Roaming AppData | src-tauri/src/friends.rs:831-860 | Protect it with DPAPI (`CryptProtectData`) or the Windows Credential Manager (keyring), and keep it in LocalAppData. |
| NW-12 | Low | Any clippiboy://auth deep link consumes the pending sign-in verifier | src-tauri/src/friends.rs:147-155, src-tauri/src/friends.rs:289 | Keep the verifier until an exchange succeeds or a timeout passes; ignore links silently when no sign-in is in progress. |
| NW-13 | Low | P2P exposes IP addresses before accept, and friend-chosen endpoints/relays get dialled | src-tauri/src/share.rs:313, src-tauri/src/share.rs:783 | Say this in the privacy page; offer only global addresses plus the n0 relay; ignore relay URLs outside the default map; consider a relay-only option. |
| NW-14 | Low | Share temp tracks and crash .part files leak | src-tauri/src/share.rs:1013-1015, src-tauri/src/share.rs:1113 | `impl Drop` for `Outgoing` (or clean up on every end state), plus a startup sweep of those folders. |
| NW-16 | Low | safe_file_name misses Windows device names and bidi control characters | src-tauri/src/share.rs:1176-1194 | Reuse the reserved-name logic and strip the Cf bidi characters. |
| NW-17 | Low | Updater downgrade through the unsigned version field in latest.json | tauri-plugin-updater-2.10.1/src/updater.rs:530-532, tauri-plugin-updater-2.10.1/src/updat… | After download, check that the signature's trusted comment (`file:ClippiBoy_<ver>…`) matches `update.version`, or keep a minimum-version floor in the app. |
| NW-18 | Low | ffmpeg archive isn't hashed as a whole and nothing caps its size | src-tauri/src/tools.rs:221-257, src-tauri/src/tools.rs:262-291 | Pin the zip's own SHA-256 and check it before opening; cap each entry at its expected size. |
| NW-22 | Low | Reconnect backoff has no jitter | src-tauri/src/friends.rs:404-409 | Add random jitter to the reconnect backoff. |
| NW-25 | Low | Any Discord user can re-request friendship right after being declined, triggering an in-game banner each time | server/src/index.ts, src-tauri/src/friends.rs:360-366 | Rate-limit or suppress re-requests after a decline. |
| NW-27 | Low | Privacy page says connection details of both devices pass the server; only the sender's do | site/privacy.html | Correct the privacy page wording. |
| NW-6 | Low | Two concurrent receives with the same file name share one .part file; duplicate offer ids can be accepted twice | src-tauri/src/share.rs:643-647, src-tauri/src/share.rs:687 | Use a unique part name (`.<uuid>.part`); claim the final name atomically (`create_new`, or a move without replace, retrying on collision); reject duplicate offer ids. |
| NW-7 | Low | P2P fetch/serve stalls never time out; router JoinSet unbounded; outgoing secret stays valid forever | src-tauri/src/share.rs:787, src-tauri/src/share.rs:794 | Per-operation `tokio::time::timeout` (e.g. 30 s without progress); an expiry on outgoing secrets. |
| NW-8 | Low | Friends WebSocket connect has no timeout, freezing the reconnect loop | src-tauri/src/friends.rs:428 | A 15 s timeout on the WebSocket connect. |
| NW-9 | Low | Updater builder has no timeout, so one hung check stops all hourly checks | src-tauri/src/updater.rs:73-91 | Set a timeout on the updater builder. |
| NW-19 | Info (reported Low) | handle_relay doesn't check that `from` is a current friend | src-tauri/src/share.rs:471 | Check `from` against the local friend list in `handle_relay`. |
| NW-20 | Info (reported Low) | Server-supplied link.id goes into URL paths unchecked | src-tauri/src/links.rs:489, src-tauri/src/links.rs:660 | Validate/escape `link.id` before building URL paths. |
| NW-29 | Info | "Appear offline" still sends game and status to the server, not stated on the privacy page | site/privacy.html | State this on the privacy page. |

## Unverifiable from source or disputed

| ID | Reported | Title | What is uncertain |
|---|---|---|---|
| NW-26 | Low | Unverified whether Windows toasts escape friend-supplied text in XML | Path is real: friends.rs:719-723 (only after `if overlay.overlay.enabled {...return}` at 697-716, and only when the window is not in front and not hushed in game) `app.notification().builder().title("ClippiBoy").body(text).show()`; tauri-plugin-notification-2… |

## Claims rejected by verification

| ID | Claim | Why it was rejected |
|---|---|---|
| NW-15 | Failed shrink leaves temp link file behind | links.rs:461 `let written = shrinking.await.map_err(\|err\| err.to_string())??;` -- but the inner error comes from commands.rs:1079 shrink(), which removes its own output on every failure after ffmpeg starts: commands.rs:1118 `let _ = std::fs::remove_file(&output);` (hw encoder failed, before x264 re… |
| NW-21 | Friends outbox is an unbounded channel | friends.rs:438 `let (sender, mut outbox) = mpsc::unbounded_channel::<String>();` is unbounded, but the 'socket down' scenario cannot happen: on every disconnect friends.rs:402 `*friends.outbox.lock() = None;` (and sign-out friends.rs:330), and both producers bail when there is no sender: friends.rs… |
| NW-24 | .expect("HTTP client") in three client builders can abort the app | The three sites exist: links.rs:235-236, telemetry.rs:232-233, friends.rs:736-737 `.build().expect("HTTP client")`, and Cargo.toml:107 `panic = "abort"` would make a panic fatal. But build() cannot fail with this configuration: each site first calls install_crypto (friends.rs:743-746 installs ring… |

## What is solid

- Peer file names and game names cannot escape the clip folder: `safe_file_name` plus `filing::folder_name` strip separators, `:`, `..` and force a known extension.
- The P2P transfer secret is compared in constant time.
- Sign-in: the PKCE-style verifier plus the state cookie on the server are correct.
- Updater: the minisign signature is checked before install; nothing in the app's own code skips verification.
- ffmpeg download: each program's SHA-256 is pinned in the binary and checked before the files are moved into place; entries are read by exact name, so there is no zip-slip.
- Share-link upload is streamed from disk, never whole in memory.
- Server data in the WebView: no `dangerouslySetInnerHTML` anywhere; the CSP limits images to `cdn.discordapp.com`; the app never opens a URL the server sends.
- control.rs (local Stream Deck port) was briefly checked and nothing was found.
- No session token appears in the logs.
- Telemetry: no security findings. Exactly the documented fields are sent (UUID v4, version, source, heard, three counts capped at 10,000, six feature bits); own client with no auth header and no shared connections, own thread with a 15 s timeout; nothing goes out before the notice has been seen or while the switch is off; turning it off deletes `usage.json`, turning it on again starts a new id, an…
- Privacy consistency: ping contents, opt-out with the notice gate, delete-on-off and new-id-on-on, and the `usage.json` path match design doc section 3.
- Privacy consistency: "Without signing in, only GitHub" holds — the only hosts in the code are github.com and api.clippiboy.com (the ping); iroh starts only on the first share.
- Privacy consistency: avatars come only from `cdn.discordapp.com` (enforced by the CSP).
- Privacy consistency: "Show what I'm playing" and "Appear offline" behave as described.
- Server relay limits are in place: 4 KB messages, 30 messages per 10 s per socket, friendship checked, `from` set by the server.
