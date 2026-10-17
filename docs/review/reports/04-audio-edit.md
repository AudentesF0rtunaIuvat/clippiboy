# ClippiBoy review: Rust audio system, clip editing and export (read-only)

I read all of: `audio/{mod,engine,capture,ring,devices}.rs`, `stems.rs`, `edit.rs`, `export.rs`, `preview.rs`, `sounds.rs`, `src/lib/useClipMix.ts`, and the related commands in `commands.rs` (lines 670-1345). I also read the parts of `pipeline.rs`, `muxer.rs`, `state.rs`, `lib.rs`, `ClipPlayer.tsx`, `ClipEditor.tsx` and `AudioMixer.tsx` needed to follow each call path. I modified nothing.

## Audio architecture as I understood it
- **Sources.** `audio::resolve` (mod.rs:273) turns the configured sources into WASAPI streams:
  - Game: a process-loopback tap on the detected pid.
  - Application: a process-loopback tap on its live pid, re-found by exe name after a restart.
  - Input device: ordinary capture.
  - Output device: endpoint loopback ("device mode").
- **Leftovers mode.** As soon as any game or application source is enabled, every output device instead becomes one INCLUDE_TREE process tap per application root that renders there (`Leftovers::assign`). Things already recorded elsewhere (`taken`, plus ClippiBoy's own pid) are skipped.
- **Capture threads.** Each stream has its own thread (capture.rs `win::run`):
  - Devices are polled every 5 ms; process loopback is event-driven. Both use a 200 ms shared-mode buffer and MTA COM.
  - Each packet is converted to 48 kHz stereo f32 (first two channels, linear resampler).
  - It is written into a 1 s `SampleRing` with its QPC timestamp. A lost device is rebuilt with backoff.
- **SampleRing** (ring.rs) keeps its own timeline. Gaps of 50 ms–1 s are filled with zeros, gaps over 1 s restart it, and smaller drift is pulled back at up to 2000 ppm.
- **Mixer.** A thread runs every 10 ms (pipeline.rs:755). Through `AudioEngine::mix_window` it reads a time window lagging 80 ms behind QPC "now". It applies live gain/mute/solo, hard-clips each track, converts to i16 and pushes into one long `TrackRing` per source. Silence is counted rather than stored.
- **Save.** `muxer::build` writes a WAV per own-track source plus a summed main-mix WAV. It then writes one MP4 with a single mixed AAC track, and the individual tracks as AAC m4a under `<data>/tracks/<id>/`, only when there is more than one WAV.
- **Editor.** `edit::plan`/`apply` re-mixes from those tracks with `-ss` per input. A cut at the front re-encodes from the untouched original kept in `<data>/originals/<id>/video.mp4` (with `trim.json`). A cut at the back stream-copies. `repair`/`settle` fix up the bookkeeping after a crash.
- **Export.** `export::recipe` works a bitrate back from the target size and steps the resolution down when that bitrate is too thin.
- **Preview.** `useClipMix` plays the individual tracks as `<audio>` elements through a WebAudio GainNode graph, kept in sync with a muted `<video>` by seeking.

## Findings, most severe first

**1. [High] [bug] A launcher's leftovers tap also records the game, so game audio is in the clip twice** — mod.rs:201-207 and 289
```rust
let covered = spoken_for.contains(&root)
    || ancestors(root, tree).iter().any(|parent| spoken_for.contains(parent));
...
SourceKind::Game => taken.extend(game_pid),
```
- The check only asks whether a leftovers root sits *below* something already recorded, never whether it sits *above* it.
- Steam, Battle.net, Epic and the Riot Client launch the game as their child. The repo's own test fixture notes that steam.exe "holds sessions on every device".
- So steam.exe becomes a leftovers root and is tapped with `INCLUDE_TARGET_PROCESS_TREE`, which takes in its child, the game. The game is then on the game track *and* in the device/main mix. Muting the game track afterwards no longer removes it, which is exactly what the README promises cannot happen.
- Same class of bug: the game is excluded as `game_pid` itself, not `app_root(game_pid)`. If the game's parent runs the same exe (a launcher stub), the leftovers tap that parent and record the game again.
- **Confidence:** likely. The logic gap is confirmed; it depends on the real process parentage.
- **Fix:**
  - In `assign`, also skip any root that is an ancestor of a taken pid: `taken.iter().any(|t| ancestors(*t, tree).contains(&root))`.
  - For such a root, tap its other session-holding descendants one by one instead. Accept that the launcher's own sound is lost, or surface a warning.
  - Use `app_root` for the game in both `taken` and the tap.
  - Add a test with steam.exe as the parent of game.exe.

**2. [High] [bug/data loss] Clips with one audio track: gain compounds on every save, and mute permanently strips the audio** — edit.rs:590-646 with stems.rs:174-179 and ClipEditor.tsx:113-121
- When a clip has no individual tracks (only one WAV at save time, e.g. "desktop audio only", or a legacy clip), `from_stems == false`. The audio then comes from `format!("0:a:{}", track.index)` of input 0.
- When nothing is cut at the front, input 0 is the current clip file (`VideoSource::Clip`), which already has the previous gain baked in.
- The editor still shows a gain/mute row for that single track, and `apply_clip_edit` stores the mix. Every later save re-applies it on top: −6 dB, then −12 dB, and so on. It also adds one more AAC generation each time.
- A front cut instead reads the original, so the level jumps back.
- Muting the track gives `mix_filter` → `None` → `-an`. Without an original, the audio is gone for good. `clip_tracks` then returns 0 tracks, so `dirty` (ClipPlayer.tsx:535) can never become true and Save is disabled for that clip forever.
- With an original present, later saves fail because `0:a:0` no longer exists in the target.
- **Confidence:** confirmed.
- **Fix:**
  - For single-track clips, always take the audio from `edit::source_path(clip)`, seeked to `stems_start_ms`. That keeps it one generation from the original, and the gain is relative to it.
  - Alternatively, write a stem (a copy of the original track) on the first edit.
  - Or hide/disable gain and mute when `!separate`, and store neutral values after baking them in.

**3. [High] [bug] Endpoint loopback on 5.1/7.1 devices drops the centre and surround channels (game dialogue)** — capture.rs:280-287
```rust
if src_channels == 1 { left = sample; right = sample; }
else if channel == 0 { left = sample; } else if channel == 1 { right = sample; }
```
- Device mode is what "Quick setup" creates (default output device + mic; AudioMixer.tsx:506-511).
- Gaming headsets often expose a 7.1 endpoint (virtual surround). Games put dialogue on the centre channel, which never reaches the clip; rear and side effects are lost too.
- **Confidence:** confirmed.
- **Fix (either):**
  - Initialise the device client with `float_format()` and `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY`, so Windows does the downmix and resampling.
  - Or implement an ITU downmix driven by `dwChannelMask`.

**4. [High] [robustness/data loss] Rolling back the first cut can delete the only copy of the recording** — edit.rs:559-568
```rust
if let Err(err) = replace_file(temp, target) {
    if first_cut {
        let _ = move_across(&archive, target);
        let _ = std::fs::remove_file(note_path(&clip.id));
        let _ = std::fs::remove_dir_all(dir(&clip.id));
    }
    let _ = std::fs::remove_file(temp);
```
- At this point the target has already been renamed into the originals store.
- If `replace_file` fails 10 times (e.g. an antivirus or indexer handle) and the move back also fails, the result is ignored. `remove_dir_all` then deletes the archive and `temp` is deleted as well, so the clip no longer exists anywhere.
- This contradicts the doc comment "the clip can never disappear entirely".
- **Confidence:** confirmed code path; unlikely trigger.
- **Fix:** delete the note and folder only when `move_across` returned `Ok`. Otherwise leave note + archive in place, so `repair` arm 1 (`(false, Some(_), true)`) restores it on the next start, and keep `temp` too.

**5. [High/Medium] [robustness] First cut across drives: a crash during the copy makes `repair` mark the untouched clip as trimmed** — edit.rs:497-511 with 945-948
- `move_across` uses `std::fs::copy(from, to)` straight onto `video.mp4`. When clips sit on D: and AppData on C: (the case the README describes), copying several GB takes seconds to minutes.
- If the app is killed during the copy, on restart target, note and (partial) `video.mp4` all exist and the database row has no `original`. That matches arm `(true, Some(note), true) if clip.original.is_none()` ("swapped, but the database never got its turn"), so `set_original(note)` runs.
- `settle` returns early because the clip's size is unchanged.
- Result: the database says "trimmed at start_ms" while the file is the full recording, so the individual tracks play `start_ms` off the picture. "Undo trim" works from a truncated archive with no moov (the muxer does not use faststart) and fails.
- Related: if the copy fails partway, swap_in (550-556) removes temp and note but leaves the partial `video.mp4`. `source_path` (87-94) checks only `video.mp4`, so it hands that partial file to later `clip_tracks` and edits until the next restart.
- **Confidence:** confirmed by tracing.
- **Fix:**
  - Copy to `video.mp4.part`, fsync, then rename within AppData; delete `.part` files in `repair`.
  - In arm 3, check that the clip really is trimmed (probed length < `note.duration_ms` − tolerance) before trusting the note.
  - Better still: keep the originals store on the same volume as the clip (e.g. a hidden `.clippiboy/originals` folder) so the move is always an atomic rename. That also removes the problem of copying GBs to C: without checking free space.

**6. [Medium] [bug] An application picked by a child pid (Discord) records only that child; the other session is recorded nowhere** — mod.rs:292-298 vs 336-345
- `taken` excludes `app_root(live_pid)`, i.e. the whole Discord tree, from the leftovers.
- But the stream taps `pid: now`, the live pid itself.
- `list_processes` returns *session* pids, and for Discord those are its utility children (the repo's own `discord()` fixture). The UI stores one of them (AudioMixer.tsx:787-795).
- The sibling session (voice or notifications/stream, depending which "Discord" entry was clicked) ends up neither on the Discord track nor in the leftovers.
- **Confidence:** confirmed.
- **Fix:** tap `app_root(live_pid)` in `resolve`, consistent with `taken`. Also deduplicate the picker by root.

**7. [Medium] [robustness/leak] A capture-start timeout leaves a zombie capture thread and WASAPI client running forever** — capture.rs:127-136
```rust
Err(_) => Err("timed out starting the audio source".into()),
```
- `stop` is never set and the JoinHandle is dropped.
- If the device's `Activate`/`Initialize` answers after 5 s, the thread sends into a dead channel and then pumps into an unreferenced ring forever, holding the client.
- `retry_failed` runs every 10 s and can pile these up for a flaky USB or Bluetooth device.
- **Confidence:** confirmed (needs a slow device).
- **Fix:** call `stop.store(true, Relaxed)` before returning the timeout error. The thread checks it after init and exits.

**8. [Medium] [bug/UX] Solo is destructive and persisted, a disabled soloed source silences every track, and the meters do not show it** — mod.rs:448-451, engine.rs:242
```rust
let any_solo = sources.iter().any(|s| s.solo);
source.enabled && !source.muted && (!any_solo || source.solo)
```
- `any_solo` also counts disabled sources. Solo a source, then disable it, and every track records silence.
- Solo is stored in the config, so this survives restarts.
- `levels()` checks only `muted`, not `audible()`, so the meters keep moving while the buffer records zeros.
- Separately, the UI's `audible` (useClipMix.ts:19-21: `anySolo ? state.solo : !state.muted`) treats soloed+muted as audible, unlike the core. The comment claims the two are the same.
- **Confidence:** confirmed.
- **Fix:**
  - Use `sources.iter().any(|s| s.enabled && s.solo)`.
  - Make the meters follow `audible()`.
  - Clear solo on disable and/or on startup.
  - Add a configuration warning "solo active — N tracks record silence".

**9. [Medium] [bug] In device mode (the Quick-setup default), ClippiBoy's own sounds and the editor preview are recorded into the buffer** — sounds.rs:55-58 / capture.rs:334-336
- Excluding ClippiBoy's own pid only happens in leftovers mode. Plain endpoint loopback records everything.
- That includes the "clip saved" chime (on by default, volume 0.5), "recording started" (lands in the recording itself), friend chimes, and any clip played in the editor while the buffer runs.
- **Confidence:** confirmed.
- **Fix (one of):**
  - When there is a single output source, use process loopback in `EXCLUDE_TARGET_PROCESS_TREE` mode on ClippiBoy's own pid.
  - Or at least warn about it in device mode.
  - Or play the chimes on another endpoint (`eCommunications`).

**10. [Medium] [bug] Export-to-size arithmetic misses the target** — export.rs:79, 83-93, 89
- `let seconds = (target.duration_ms / 1000).max(1);` truncates. A 15.8 s clip at 10 MiB is budgeted for 15 s and comes out about 10.71 MB (+2.2%) before any encoder error. A 1.9 s clip is budgeted for 1 s and comes out nearly 2× over.
- The resolution loop sets `height = step` *before* checking the budget. Any source taller than 1080 (1440p, 4K, 1200p) is always scaled to ≤1080p, and 900p to 720p, even with ample budget. The comment "anything taller than the first is left where it is" is contradicted, and no test covers 1440p.
- `bitrate_for(.., 30)` assumes 30 fps, so the starvation check is 2× too optimistic for 60 fps clips.
- One-pass ABR/CBR leaves only a 3% margin.
- **Confidence:** confirmed.
- **Fix:**
  - Compute in ms: `video_bits * 1000 / duration_ms / 1000`.
  - First check whether the source height itself fits; use the real fps; consider stepping to 30 fps before dropping resolution.
  - x264 two-pass, or verify the result and re-encode with `bitrate × target/written × 0.97` when it is over.

**11. [Medium] [bug] `export_for_discord` reuses stale or half-written files and never fixes an oversize one** — commands.rs:871-888
```rust
let ready = std::fs::metadata(&output).map(|meta| meta.len() > 0 && meta.len() <= target_bytes)
```
- There is no freshness check. After the user trims or remixes, the old export is copied to the clipboard.
- `shrink` writes straight to the final name. A crash or kill leaves a truncated MP4 without moov that counts as "ready" from then on.
- Two quick presses run two ffmpeg processes with `-y` on the same file.
- An over-limit result is put on the clipboard regardless. The next press re-encodes the same recipe and gets the same overshoot.
- **Confidence:** confirmed.
- **Fix:**
  - Check `stems::is_newer(&output, &source)`.
  - Write to `*.part`, then rename.
  - Allow one export per clip at a time.
  - Retry at a lower bitrate when the file is over the limit.

**12. [Medium] [robustness/perf] No cancel, timeout or kill for ffmpeg, and it runs at normal priority next to the game** — edit.rs:749-796 (and muxer `run`)
- There is no Job Object, `kill`, `try_wait` or timeout anywhere in the crate (grep-confirmed).
- A hung hardware encoder holds `edit::WORKING` forever, so every later trim/remix/"Clear all trims" blocks.
- On app exit, ffmpeg children are orphaned (Windows does not kill children).
- x264 `-preset medium` exports, started from the in-game console, compete with the game at normal priority.
- **Fix:**
  - Put children in a Job Object with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`.
  - Use `BELOW_NORMAL_PRIORITY_CLASS`.
  - Add a cancel token that kills the child, and a stall watchdog when `out_time` stops advancing.
  - Consider `-hwaccel d3d11va` for decoding.

**13. [Medium] [quality] The resampler is stateless and per packet, with no anti-aliasing** — capture.rs:298-312
- Each packet restarts the phase at 0, and `next` is clamped inside the packet, so the last samples are held instead of interpolated into the next packet.
- `target_frames = (frames as f64 * ratio) as usize` truncates, dropping up to one frame per packet; the ring's slew then has to absorb it.
- 96/192 kHz endpoints (common "studio quality" settings) are decimated by picking samples, which aliases.
- 16/8 kHz Bluetooth HFP mics produce imaging.
- **Fix:** the AUTOCONVERTPCM + SRC_DEFAULT_QUALITY flags from item 3, or a stateful resampler (`rubato`) kept per stream.

**14. [Medium] [security/robustness] `delete_clip("")` wipes every individual track and every original** — commands.rs:680-681, stems.rs:45-47, edit.rs:61-63
- `sanitize("")` is `""`, so `root().join("")` is the root itself, and `remove_dir_all` removes `<data>/tracks` and `<data>/originals` entirely.
- This happens before the clip is even looked up in the database.
- An empty id from a frontend bug, or a compromised webview, destroys every untrimmed original.
- **Confidence:** confirmed.
- **Fix:** reject ids that are empty or not UUIDs inside `stems::dir`/`edit::dir`, and resolve the clip from the database before deleting anything.

**15. [Medium-Low] [bug] A single bad timestamp inserts silence and then shifts the source late for tens of seconds** — ring.rs:151-163 with capture.rs:631-657
- Any block whose stamp is over 50 ms late is treated as a real gap and filled with zeros immediately.
- The correctly stamped blocks that follow show negative drift, which may only slew at 2000 ppm: 50 ms takes about 25 s, during which the source plays late.
- When does this happen?
  - `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR` (0x4) is ignored; only SILENT (0x2) is handled.
  - For devices on the fallback clock, the read-time `now` is used. After a thread stall, the first packet of the burst looks "late" and the rest "early", so spurious silence is inserted even though WASAPI had kept the data.
  - Negative drift between −50 ms and −1 s is never treated as a discontinuity.
- **Confidence:** likely.
- **Fix:**
  - Detect real gaps with `pu64DevicePosition` (pass it instead of `None`): it jumps by exactly the missing frames.
  - Ignore stamps flagged 0x4.
  - With the fallback clock, use `now − (GetCurrentPadding + frames)/rate`.
  - Require two agreeing blocks before filling a gap, and re-anchor on a sustained negative drift.
  - Add tests for these cases.

**16. [Medium-Low] [perf/robustness] No MMCSS, and the 80 ms mixer lag discards late packets** — pipeline.rs:42, capture.rs:470-499
- Capture and mixer threads run at normal priority with no `AvSetMmThreadCharacteristicsW`.
- If a capture thread is delayed more than about 70 ms under game load, its packet lands in the ring after the mixer has already pushed zeros for that window. WASAPI kept the data, but it is lost from the clip.
- **Fix:**
  - Register the threads with MMCSS ("Audio"/"Capture").
  - Raise `AUDIO_LAG` to about 200-250 ms. A replay buffer does not need low latency; the save can wait for the mixer to pass the last frame's QPC.
- **Related (speculative):** `snapshot()` hands over the *live* `TrackRing`s, which keep only 2 s of slack, and the WAVs are only written after the whole h264 file. A slow disk plus aligning the start to a keyframe could let the front of the clip's audio scroll out of the ring before it is read. Copy the windows inside `snapshot()`.

**17. [Low-Medium] [bug] Sources set to "the default device" do not follow default-device changes** — capture.rs:320-327, mod.rs:235
- There is no `IMMNotificationClient` (grep-confirmed). An empty `device_id` binds to whatever was default at stream start and stays there until that device becomes invalid. The fingerprint `out:` never changes, so the stream is never restarted. The configuration warning even claims the source "follows the Windows default device".
- In `Leftovers::assign`, `*device == self.default_device` compares `""` against the real id, so the catch-all rule never matches for such a source.
- **Fix:** register `OnDefaultDeviceChanged` and restart the affected stream; normalise `""` to the resolved id before assigning.

**18. [Low-Medium] [perf] Drawing the waveform holds the whole decoded audio in RAM** — preview.rs:49-53
- ffmpeg's `showwavespic` queues every input frame until EOF (s16 mono), which is about 345 MB per hour of recording.
- There is no lock, so StrictMode/double opens write the same PNG concurrently. A long waveform job also keeps the clip open, so a save during it fails with "is it open somewhere".
- Fully muted clips (no audio stream) re-run ffmpeg and error on every open.
- **Confidence:** likely (ffmpeg internals).
- **Fix:** put `aresample=8000`/`asetnsamples` before it, or compute min/max peaks in Rust from `-ac 1 -ar 4000 -f s16le pipe:1`; add a per-clip lock.

**19. [Low] [bug] Applications running elevated are stored as "PID n" and can never be found again** — devices.rs:91-96
- `OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ)` plus `GetModuleBaseNameW` fails for elevated processes. The UI then stores `exe: "PID 1234"`, and after a restart `live_pid` finds nothing and the source drops out silently.
- **Fix:** take the name from the `process_tree()` snapshot, or use `QueryFullProcessImageNameW`, which needs only limited query rights.

**20. [Low] [unsafe/robustness] The activation completion handler can signal a closed event handle after a timeout** — capture.rs:177-192, 389-393
- On a 5 s timeout `CloseHandle(event)` runs while the async operation still holds the handler, which holds the raw HANDLE. A late `ActivateCompleted` then calls `SetEvent` on a closed handle, which may since have been reused for another event.
- `Error::from_win32()` after `WAIT_TIMEOUT` reports GetLastError = 0 ("The operation completed successfully").
- **Fix:** let the handler own the event (close it in `Drop`) and return an explicit timeout HRESULT.

**21. [Low] [quality] Log spam: `live_pid` logs every 2 s, twice per pass** — mod.rs:139-141
- The stored pid is never updated, so once an application has restarted, every `resolve` logs "'{exe}' is {now} now". That is about 86k lines a day.
- **Fix:** log only when the value changes, or write the new pid back to the config.

**22. [Low] [quality] No limiter on the summed mix** — pipeline.rs:225-234 (i16 `saturating_add`), stems.rs:349-352 (`amix … normalize=0`)
- Game, Discord and mic at 0 dB clip hard twice, and the AAC encoder receives float samples above 1.0.
- **Fix:** add an `alimiter`/true-peak limiter (−1 dBTP) on `[aout]`, and optionally a soft clipper in `write_mix_wav`.

**23. [Low] [perf/quality] Small costs in the realtime path**
- `convert` allocates a `stereo` Vec on every packet (capture.rs:251).
- `mix_window` allocates `scratch` on every 10 ms tick (engine.rs:355), despite the doc comment "rather than allocating".
- `next_100ns = from + frames*1e7/48000` (pipeline.rs:818) truncates and accumulates about 3.5 ppm. Derive it from a total frame count instead.
- The meter peak uses load-then-store instead of `fetch_max` (ring.rs:130-133).
- In leftovers mode, every idle application holding a session gets its own thread and loopback client.
- `default_output_id` and `list_devices` enumerate every endpoint and its property store every 2 s.
- `list_audio_devices`/`list_audio_processes` are synchronous Tauri commands, so they do COM work on the main thread.
- At startup, `AppState::new` calls `apply` synchronously on the main thread and can block up to 5 s per slow stream (state.rs:308).

**24. [Low] [bug] Preview sync in `useClipMix.ts`**
- Tracks are only corrected when off by more than 120 ms (line 317), with a hard seek once a second (line 353). An A/V offset of up to 120 ms persists, and each correction is an audible skip.
  - **Fix:** use a threshold of about 30-40 ms and nudge `playbackRate` by ±1-3% for small corrections, seeking only above about 150 ms. Or decode the tracks into AudioBuffers and schedule them against `ctx.currentTime`.
- Elements recreated after falling back to the old route never get the current `playbackRate` until the next `ratechange`.
- The silence watchdog (lines 238-252) trips on two seconds of genuine digital silence (AAC of zeros decodes to exact zeros), or on elements still buffering, and then permanently switches to the attenuate-only route.
  - **Fix:** switch only if no sample was non-zero since the graph was built.
- `ctx.state !== "running"` returns early, so a context blocked by autoplay stays silent with the video muted, and `play().catch(() => undefined)` hides that.
- An AudioContext created in `try` leaks if `createMediaElementSource` throws.

**25. [Low] [quality] Smaller edit/export issues**
- **Temp files litter the clip folder:** `*.{pid}.neu.mp4` (edit.rs:422) and `"{stem} (N MB).mp4"` sit in the user's clip folder and are never cleaned after a crash.
- **Very short trims:** `plan` can produce a 200 ms excerpt starting at the very end of the file (end clamped past the duration).
- **Two first-cut checks are missing:**
  - No free-space check before copying the original.
  - The colour metadata (VUI) is not carried over on re-encode (`-colorspace/-color_primaries/-color_trc` are not set).
- **MB vs MiB (speculative):** the export target is computed in MiB. If Discord enforces decimal MB, "10 MB" exports land 4.9% over.
- **Leftover German strings:** some log and test messages are still in German ("blieb liegen", "Pegel", "bildgenau…").

## What I checked and found sound
- **Lock order:** `running` is always taken before a ring lock, so there is no deadlock.
- **Buffer pairing:** `GetBuffer`/`ReleaseBuffer` are paired correctly.
- **Pipes:** stderr is drained on its own thread and stdout progress is read in parallel, so ffmpeg cannot stall on a full pipe.
- **Command construction:** all ffmpeg arguments are passed as an argv Vec with no shell; filter strings contain only numeric stream specifiers. Command injection is not possible.
- **Seeking:** each input gets its own `-ss`, and trims compose correctly in original coordinates.
- **Panics:** no reachable panic in these modules. The `clamp` bounds are ordered, `expect("checked")` is guarded, and indexing uses `get`.

## Missing features / improvement opportunities
- **Recording:**
  - Proper multichannel downmix and high-quality resampling.
  - Follow the default device (`IMMNotificationClient`).
  - MMCSS thread priority.
  - A per-source sync offset to compensate Bluetooth/USB latency.
  - Monitoring a source and a solo that does not destroy the recording.
- **Microphone processing:** noise suppression (RNNoise/DeepFilterNet), noise gate, compressor and push-to-talk/ducking.
- **Loudness and limiting:** a true-peak limiter on the mix; EBU R128 normalisation (`loudnorm`, two-pass) on save and export (−14 LUFS for social, −23 for broadcast).
- **Multi-track delivery:**
  - Export individual tracks (WAV/FLAC, or multi-track MKV/MOV for Premiere or DaVinci Resolve).
  - Opus/WebM for sharing.
  - A waveform per track in the editor.
- **Export:** a cancel button, two-pass or verify-and-retry, a frame-rate step (60→30) before resolution, HEVC/AV1 at the same size, and background priority.
- **Trimming:**
  - "Smart render" for cuts at the front: re-encode only up to the next keyframe and copy the rest, which is fast and nearly lossless.
  - A "snap to keyframe, lossless" option.
- **Storage:** keep the originals store on the same volume as the clips.
