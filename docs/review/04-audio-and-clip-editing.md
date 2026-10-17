# Audio capture, mixing, clip editing and export

**Scope:** audio/{mod,engine,capture,ring,devices}.rs, stems.rs, edit.rs, export.rs, preview.rs, sounds.rs, src/lib/useClipMix.ts.

**Findings:** 43 — 24 verified (9 medium or higher), 15 reported but not independently re-verified, 2 unverifiable/disputed, 2 rejected by verification.

Severity shown is the verified severity; where verification changed it, the reported one is in parentheses. See [README](README.md#how-findings-were-verified) for how verification worked.

## Verified issues (medium and above)

### AU-1 · High · A launcher's leftovers tap also records the game, so game audio is in the clip twice

- **Where:** `src-tauri/src/audio/mod.rs:201-207`, `src-tauri/src/audio/mod.rs:289`
- **Verification:** confirmed (code: confirmed; impact: confirmed; reachability: confirmed)

**Problem:** In `Leftovers::assign` (audio/mod.rs:201-207) the coverage check `let covered = spoken_for.contains(&root) || ancestors(root, tree).iter().any(|parent| spoken_for.contains(parent));` only asks whether a leftovers root sits *below* something already recorded, never whether it sits *above* it. In addition, `SourceKind::Game => taken.extend(game_pid),` (mod.rs:289) excludes only the game pid itself, not `app_root(game_pid)`. Launchers (Steam, Battle.net, Epic, Riot Client) start the game as their child, and the repo's own test fixture notes steam.exe "holds sessions on every device". So steam.exe becomes a leftovers root and is tapped with `INCLUDE_TARGET_PROCESS_TREE`, which includes its child, the game. Same class: if the game's parent runs the same exe (a launcher stub), the leftovers tap that parent and record the game again.

**Scenario:** User records a Steam game with a Game source plus an output device in leftovers mode. steam.exe is chosen as a leftovers root and its INCLUDE_TREE tap captures game.exe, so the game is on the game track and also in the device/main mix. Muting the game track afterwards no longer removes the game audio, which is exactly what the README promises cannot happen.

**Fix:** In `assign`, also skip any root that is an ancestor of a taken pid: `taken.iter().any(|t| ancestors(*t, tree).contains(&root))`. For such a root, tap its other session-holding descendants one by one instead (accept that the launcher's own sound is lost, or surface a warning). Use `app_root` for the game in both `taken` and the tap. Add a test with steam.exe as the parent of game.exe.

**Verifier on the fix:** The direction is right. The extra step 'tap its other session-holding descendants one by one' is already free: steamwebhelper.exe and other differently-named children are separate roots, and they get picked up by themselves once steam.exe is skipped and not pushed into spoken_for. Minimal fix: in assign, before the loop, `let above_taken: HashSet<u32> = taken.iter().flat_map(|t| ancestors(*t, tree)).collect();` and add `|| above_taken.contains(&root)` to `covered` (only the original `taken` needs this, because leftover roots are processed shallowest first). In resolve, use `taken.extend(game_…

### AU-2 · High · Clips with one audio track: gain compounds on every save, and mute permanently strips the audio

- **Where:** `src-tauri/src/edit.rs:590-646`, `src-tauri/src/stems.rs:174-179`, `src/components/ClipEditor.tsx:113-121`, `src/components/ClipPlayer.tsx:535`
- **Verification:** confirmed (reachability: confirmed; impact: confirmed; code: confirmed)

**Problem:** When a clip has no individual tracks (only one WAV at save time, e.g. "desktop audio only", or a legacy clip), `from_stems == false` and the audio comes from `format!("0:a:{}", track.index)` of input 0. When nothing is cut at the front, input 0 is the current clip file (`VideoSource::Clip`), which already has the previous gain baked in. The editor (ClipEditor.tsx:113-121) still shows a gain/mute row for that single track and `apply_clip_edit` stores the mix, so every later save re-applies it on top and adds one more AAC generation. A front cut instead reads the original, so the level jumps back. Muting the track makes `mix_filter` return `None`, which yields `-an`; without an original the audio is gone for good, `clip_tracks` then returns 0 tracks, so `dirty` (ClipPlayer.tsx:535) can never become true and Save is disabled for that clip forever. With an original present, later saves fail because `0:a:0` no longer exists in the target.

**Scenario:** A desktop-audio-only clip: set the track to -6 dB and save; open again and save another edit -> audio is now -12 dB, and so on with an extra AAC generation each time. Or mute the track and save -> audio stripped permanently (no original) and Save is disabled for that clip forever; with an original, later saves fail on missing `0:a:0`.

**Fix:** For single-track clips, always take the audio from `edit::source_path(clip)`, seeked to `stems_start_ms`, so it stays one generation from the original and the gain is relative to it. Alternatively, write a stem (a copy of the original track) on the first edit. Or hide/disable gain and mute when `!separate`, and store neutral values after baking them in.

**Verifier on the fix:** Option 1 (always read audio from source_path) is not enough on its own. Mix-only saves on an untrimmed clip never archive an original, so source_path is the overwritten file itself. When the original has been discarded, stems_start_ms is also the wrong offset into the clip file. Option 2 (write the single track as a stem, e.g. `-c:a copy` to stems/<id>/0.m4a plus write_index, from source_path before the first swap) is the correct fix and keeps the feature, at about 1.4 MB/min of storage. Clips that were already edited must have their stored mix reset to neutral when the stem is made from an a…

### AU-3 · High · Endpoint loopback on 5.1/7.1 devices drops the centre and surround channels (game dialogue)

- **Where:** `src-tauri/src/audio/capture.rs:280-287`
- **Verification:** confirmed (reachability: confirmed; impact: confirmed; code: confirmed)

**Problem:** The packet conversion in capture.rs:280-287 keeps only the first two channels: `if src_channels == 1 { left = sample; right = sample; } else if channel == 0 { left = sample; } else if channel == 1 { right = sample; }`. There is no downmix of centre/LFE/surround channels. Device mode (endpoint loopback) is what "Quick setup" creates (default output device + mic; AudioMixer.tsx:506-511).

**Scenario:** A gaming headset exposes a 7.1 endpoint (virtual surround). The game puts dialogue on the centre channel, which never reaches the clip; rear and side effects are lost too.

**Fix:** Either initialise the device client with `float_format()` and `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY` so Windows does the downmix and resampling, or implement an ITU downmix driven by `dwChannelMask`.

**Verifier on the fix:** Both proposed fixes are valid. Smallest change: in device_client, initialise with float_format() and AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY (plus LOOPBACK when needed), return the fixed 48k/2ch/f32 Format, and fall back to the current mix-format path if Initialize rejects it, because driver/loopback support for AUTOCONVERTPCM should not be assumed everywhere. This also covers the mic path and replaces the linear resampler. If the code must keep the mix format instead, read dwChannelMask from WAVEFORMATEXTENSIBLE (defaults 0x3F/0x60F for 6 ch, 0x63F for 8…

### AU-11 · Medium · In device mode (Quick-setup default), ClippiBoy's own sounds and the editor preview are recorded into the buffer

- **Where:** `src-tauri/src/sounds.rs:55-58`, `src-tauri/src/audio/capture.rs:334-336`
- **Verification:** confirmed (reachability: confirmed; code: confirmed)

**Problem:** Excluding ClippiBoy's own pid only happens in leftovers mode; plain endpoint loopback (capture.rs:334-336) records everything rendered on the device, including sounds played by sounds.rs:55-58.

**Scenario:** With Quick setup (default output device + mic), the "clip saved" chime (on by default, volume 0.5), "recording started" (lands in the recording itself), friend chimes, and any clip played in the editor while the buffer runs all end up in the replay buffer/clips.

**Fix:** One of: when there is a single output source, use process loopback in `EXCLUDE_TARGET_PROCESS_TREE` mode on ClippiBoy's own pid; or at least warn about it in device mode; or play the chimes on another endpoint (`eCommunications`).

**Verifier on the fix:** Process loopback in EXCLUDE_TARGET_PROCESS_TREE mode on ClippiBoy's pid fixes it but needs Windows build 20348+ (same requirement already documented for game sources) and is not tied to a specific endpoint, so it fits only when the single output source is the default device; a fallback (warning, or skipping chimes / muting preview while buffering) is still needed on older builds. Playing chimes on eCommunications would only help if that endpoint differs from the captured one.

### AU-12 · Medium · Export-to-size truncates the duration to whole seconds and overshoots the target

- **Where:** `src-tauri/src/export.rs:79`
- **Verification:** confirmed (code: confirmed)

**Problem:** `export::recipe` computes `let seconds = (target.duration_ms / 1000).max(1);` (export.rs:79), which truncates the duration, so the bitrate budget is computed for a shorter clip than the real one.

**Scenario:** A 15.8 s clip with a 10 MiB target is budgeted for 15 s and comes out at about 10.71 MB (+2.2%) before any encoder error; a 1.9 s clip is budgeted for 1 s and comes out nearly 2x over.

**Fix:** Compute in ms: `video_bits * 1000 / duration_ms / 1000`.

**Verifier on the fix:** Right direction but incomplete as written: both the audio cost (128 * duration_ms bits) and the division must use milliseconds (video_bits * 1000 / duration_ms / 1000, guarding duration_ms == 0), and ExportDialog.tsx preview() must be changed in step; capping video_kbps at the source bitrate would also stop short clips from being inflated.

### AU-13 · Medium · Export resolution loop always downscales sources taller than 1080p (and 900p to 720p) even with ample budget

- **Where:** `src-tauri/src/export.rs:83-93`
- **Verification:** confirmed (code: confirmed)

**Problem:** The resolution step-down loop in export.rs:83-93 sets `height = step` *before* checking the budget, so any source taller than 1080 (1440p, 4K, 1200p) is always scaled to <=1080p, and 900p to 720p. This contradicts the comment "anything taller than the first is left where it is", and no test covers 1440p.

**Scenario:** Exporting a 1440p clip with a large size budget still produces a 1080p file.

**Fix:** First check whether the source height itself fits the budget before stepping down; add a test for 1440p.

**Verifier on the fix:** Correct: check bitrate_for(target.width, target.height, 30) / STARVED against the budget before stepping down (or put the source height first in the candidate list), add 1440p and 900p tests, and change ExportDialog.tsx preview() in step as its comment requires.

### AU-16 · Medium (reported High) · `export_for_discord` reuses stale or half-written files and never fixes an oversize one

- **Where:** `src-tauri/src/commands.rs:871-888`, `commands.rs:876`, `Console.tsx:498`
- **Verification:** partial (code: confirmed; impact: partly confirmed; reachability: partly confirmed)
- **Also reported as:** FP-4 (Console "Shrink to 10/25 MB and copy" can copy a stale or half-written file)

**What verification found:** Real, medium severity, not high. (1) Stale reuse is confirmed and is the main problem: after a trim, remix or restore in the editor (which swaps a new file in at the same path), the console's 10/25 MB button copies the earlier, pre-edit export without telling anyone. (2) The half-written case does exist, but it rarely comes from the app being killed, because the orphaned ffmpeg usually finishes the file. The realistic route is a second press of the same size while the first export is still running: the single busy slot lets the button come back on, and the in-progress file is copied as if it were finished. Two ffmpeg processes on the same file happen only in a narrow 0-byte window, not in general. A truncated file that stays for good needs ffmpeg itself to die (power loss, crash, ffmpeg killed). (3) The over-limit copy is deliberate and the user is told. It is still a defect for the Discord flow, and when the 200 kbps floor kicks in (long clips) it can never succeed, so retrying always reproduces it.

**Original claim:** `export_for_discord` (commands.rs:871-888) decides reuse with `let ready = std::fs::metadata(&output).map(|meta| meta.len() > 0 && meta.len() <= target_bytes)`: there is no freshness check against the source clip. `shrink` writes straight to the final file name, so a crash/kill leaves a truncated MP4 without moov that counts as "ready" from then on. Two quick presses run two ffmpeg processes with `-y` on the same file. An over-limit result is put on the clipboard regardless, and the next press re-encodes the same recipe and gets the same overshoot.

**Scenario:** User exports for Discord, then trims or remixes the clip and exports again: the old export is copied to the clipboard. Or the app is killed mid-export: the truncated file is reused forever. Or the result is over the limit: it is put on the clipboard anyway and retrying reproduces the same size.

**Fix:** Check `stems::is_newer(&output, &source)`; write to `*.part`, then rename; allow one export per clip at a time; retry at a lower bitrate when the file is over the limit.

**Verifier on the fix:** Gating reuse on `stems::is_newer(&output, &source)` is correct and enough for the stale case, because swap_in renames a freshly written temp file, so the edited clip gets a new mtime. Writing to a temp file and renaming is right, but the temp name must keep an .mp4 extension (e.g. `{stem} ({mb} MB).part.mp4`) or pass `-f mp4`, or ffmpeg cannot choose a muxer. Also delete any leftover part file first, and on Windows use a rename that replaces the destination. Guarding concurrency in the backend, with a static Mutex<HashSet<PathBuf>> on `output` (reject or wait), is better than the frontend Set…

### AU-4 · Medium (reported High) · Rolling back the first cut can delete the only copy of the recording

- **Where:** `src-tauri/src/edit.rs:559-568`
- **Verification:** confirmed (reachability: confirmed; impact: confirmed; code: confirmed)

**Problem:** In the swap-in rollback (edit.rs:559-568): `if let Err(err) = replace_file(temp, target) { if first_cut { let _ = move_across(&archive, target); let _ = std::fs::remove_file(note_path(&clip.id)); let _ = std::fs::remove_dir_all(dir(&clip.id)); } let _ = std::fs::remove_file(temp);`. At this point the target has already been renamed into the originals store. The result of `move_across` is ignored, so if the move back fails, `remove_dir_all` deletes the archive and `temp` is deleted as well. This contradicts the doc comment "the clip can never disappear entirely".

**Scenario:** On a first cut, `replace_file` fails 10 times (e.g. an antivirus or indexer handle) and the move of the archive back to the target also fails; the code then deletes the originals folder (the archive) and the temp file, so the clip no longer exists anywhere.

**Fix:** Delete the note and folder only when `move_across` returned `Ok`. Otherwise leave note + archive in place so `repair` arm 1 (`(false, Some(_), true)`) restores it on the next start, and keep `temp` too.

**Verifier on the fix:** The proposed fix is correct and sufficient: on Err from move_across, keep the note, the archive and temp, and log at error level. On the next start, repair arm 1 (edit.rs:918, target missing + note + video) moves the original back and clears the record. clip.original is already None on a first cut, so set_original(None) is correct. Minimal form: `if first_cut { match move_across(&archive, target) { Ok(()) => { let _ = remove_file(note_path(&clip.id)); let _ = remove_dir_all(dir(&clip.id)); let _ = remove_file(temp); } Err(back) => log::error!("rollback failed ({back}); original kept in store…

### AU-5 · Medium (reported High) · First cut across drives: a crash during the copy makes `repair` mark the untouched clip as trimmed

- **Where:** `src-tauri/src/edit.rs:497-511`, `src-tauri/src/edit.rs:945-948`, `src-tauri/src/edit.rs:550-556`, `src-tauri/src/edit.rs:87-94`
- **Verification:** confirmed (impact: partly confirmed; reachability: confirmed; code: confirmed)

**What verification found:** The core mechanism is right. The interrupted copy sits in the cross-volume first-cut path, repair arm 3 runs, and settle's size check skips the self-heal. The DB then says trimmed while the file is still the full recording. A second, smaller window gives the same outcome: after the copy finishes but before `remove_file(from)`, the archive is complete, so undo still works. The severity is overstated. It needs the clip folder on a different volume than %APPDATA% (the default Videos\ClippiBoy is usually on C:), the clip's first trim, and the process dying during the copy. For typical replay clips the copy takes about 1-6 s (hundreds of MB); only long manual recordings take longer. The 'related' sub-claim (a failed copy leaves a partial video.mp4) depends on the platform. std::fs::copy on Windows uses CopyFileExW, which normally deletes the partial destination when the copy fails, so that path is likely not reachable on Windows. On Linux it would be. The finding also misses the worst consequence: the next back-only cut overwrites the full clip file without archiving, so the cut-away footage is lost for good.

**Original claim:** `move_across` (edit.rs:497-511) uses `std::fs::copy(from, to)` straight onto `video.mp4` in the originals store. When clips are on D: and AppData on C: (the case the README describes), copying several GB takes seconds to minutes. If the app is killed during the copy, on restart target, note and a partial `video.mp4` all exist and the DB row has no `original`, which matches repair arm `(true, Some(note), true) if clip.original.is_none()` ("swapped, but the database never got its turn", edit.rs:945-948), so `set_original(note)` runs; `settle` returns early because the clip's size is unchanged. Related: if the copy fails partway, swap_in (edit.rs:550-556) removes temp and note but leaves the partial `video.mp4`, and `source_path` (edit.rs:87-94) checks only that `video.mp4` exists, so it hands the partial file to later `clip_tracks` and edits until the next restart.

**Scenario:** User makes a first front cut on a clip stored on D:, app is killed during the multi-GB copy to C:. On restart the DB says "trimmed at start_ms" while the file is the full recording, so the individual tracks play `start_ms` off the picture, and "Undo trim" works from a truncated archive with no moov (muxer does not use faststart) and fails.

**Fix:** Copy to `video.mp4.part`, fsync, then rename within AppData; delete `.part` files in `repair`. In arm 3, check that the clip really is trimmed (probed length < `note.duration_ms` - tolerance) before trusting the note. Better: keep the originals store on the same volume as the clip (e.g. a hidden `.clippiboy/originals` folder) so the move is always an atomic rename, which also removes copying GBs to C: without checking free space.

**Verifier on the fix:** Copying to `video.mp4.part` and then renaming is correct and makes a kill mid-copy land in arm 2, which already clears up correctly (clip.duration_ms == note.duration_ms). It does not cover a kill between the rename and `remove_file(from)`, so the arm-3 guard is the part that matters. That guard alone is the smallest fix that covers every window: in arm 3, probe the target, and if `is_the_whole_recording(Some(&note), actual_ms)` call `remove(&clip.id)` and leave the DB at None instead of `set_original`. That is one ffprobe, only in this rare arm. Moving the originals store onto the clip's own…

## Reported, not independently re-verified

The original reviewer checked these against the code, but the independent verification pass was stopped before reaching them. Treat as likely, confirm before fixing.

- **AU-14 · Medium · Export starvation check assumes 30 fps** — `src-tauri/src/export.rs:89`  
  The starvation check uses `bitrate_for(.., 30)` (export.rs:89), assuming 30 fps regardless of the clip's real frame rate, so the check is 2x too optimistic for 60 fps clips.  
  *Fix:* Use the real fps; consider stepping to 30 fps before dropping resolution.
- **AU-15 · Medium · One-pass ABR/CBR export leaves only a 3% size margin** — `src-tauri/src/export.rs`  
  Export-to-size uses one-pass ABR/CBR with only a 3% margin, and the result is not verified against the target size.  
  *Fix:* Use x264 two-pass, or verify the result and re-encode with `bitrate × target/written × 0.97` when it is over.
- **AU-18 · Medium · The resampler is stateless and per packet, with no anti-aliasing** — `src-tauri/src/audio/capture.rs:298-312`  
  The linear resampler in capture.rs:298-312 restarts the phase at 0 on each packet, and `next` is clamped inside the packet, so the last samples are held instead of interpolated into the next packet. `target_frames = (frames as f64 * ratio) as usize` truncates, dropping up to one frame per packet, which the ring's slew then has to absorb. There is no anti-aliasing filter.  
  *Fix:* Use the AUTOCONVERTPCM + SRC_DEFAULT_QUALITY flags (as for the multichannel downmix issue), or a stateful resampler (`rubato`) kept per stream.
- **AU-20 · Medium · A single bad timestamp inserts silence and then shifts the source late for tens of seconds** — `src-tauri/src/audio/ring.rs:151-163`, `src-tauri/src/audio/capture.rs:631-657`  
  In `SampleRing` (ring.rs:151-163) any block whose stamp is over 50 ms late is treated as a real gap and immediately filled with zeros; the correctly stamped blocks that follow show negative drift, which may only slew at 2000 ppm (50 ms takes about 25 s). Negative drift between -50 ms and -1 s is never treated as a discontinuity. In capture.rs:631-657, `AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR` (0x4) is ignored (only SILENT 0x2 is handled), and for devices on the fallback clock the read-time `now` is used as the stamp.  
  *Fix:* Detect real gaps with `pu64DevicePosition` (pass it instead of `None`), which jumps by exactly the missing frames; ignore stamps flagged 0x4; with the fallback clock use `now − (GetCurrentPadding + frames)/rate`; require two agreeing blocks before filling a gap and re-anchor on a sustained negative drift; add tests for these cases.
- **AU-21 · Medium · No MMCSS, and the 80 ms mixer lag discards late packets** — `src-tauri/src/pipeline.rs:42`, `src-tauri/src/audio/capture.rs:470-499`  
  Capture threads (capture.rs:470-499) and the mixer thread run at normal priority with no `AvSetMmThreadCharacteristicsW` (MMCSS). The mixer reads with an 80 ms lag (`AUDIO_LAG`, pipeline.rs:42); a packet that lands in the ring after the mixer has already pushed zeros for that window is lost from the clip.  
  *Fix:* Register the threads with MMCSS ("Audio"/"Capture"); raise `AUDIO_LAG` to about 200-250 ms (a replay buffer does not need low latency; the save can wait for the mixer to pass the last frame's QPC).
- **AU-22 · Medium · `snapshot()` hands over live TrackRings with only 2 s slack; front of clip audio may scroll out before being read** — `src-tauri/src/pipeline.rs`  
  `snapshot()` hands over the *live* `TrackRing`s, which keep only 2 s of slack, and the WAVs are only written after the whole h264 file has been written. The audio windows are not copied inside `snapshot()`.  
  *Fix:* Copy the windows inside `snapshot()`.
- **AU-23 · Medium · Sources set to "the default device" do not follow default-device changes** — `src-tauri/src/audio/capture.rs:320-327`  
  There is no `IMMNotificationClient` (grep-confirmed). An empty `device_id` (capture.rs:320-327) binds to whatever was the default at stream start and stays there until that device becomes invalid. The fingerprint `out:` never changes, so the stream is never restarted. The configuration warning even claims the source "follows the Windows default device".  
  *Fix:* Register `OnDefaultDeviceChanged` and restart the affected stream.
- **AU-24 · Medium · Leftovers catch-all rule never matches a source configured as the default device ("")** — `src-tauri/src/audio/mod.rs:235`  
  In `Leftovers::assign` (mod.rs:235) the catch-all rule compares `*device == self.default_device`; for a source configured as "the default device" `device` is `""` while `default_device` is the real endpoint id, so the catch-all never matches such a source.  
  *Fix:* Normalise `""` to the resolved default device id before assigning.
- **AU-25 · Medium · Drawing the waveform holds the whole decoded audio in RAM; no per-clip lock; muted clips re-run ffmpeg** — `src-tauri/src/preview.rs:49-53`  
  The waveform uses ffmpeg's `showwavespic` (preview.rs:49-53), which queues every input frame until EOF (s16 mono), about 345 MB per hour of recording. There is no lock, so StrictMode/double opens write the same PNG concurrently; a long waveform job also keeps the clip open. Fully muted clips (no audio stream) re-run ffmpeg and error on every open.  
  *Fix:* Put `aresample=8000`/`asetnsamples` before `showwavespic`, or compute min/max peaks in Rust from `-ac 1 -ar 4000 -f s16le pipe:1`; add a per-clip lock.
- **AU-38 · Medium · Preview sync tolerates up to 120 ms offset and corrects with audible hard seeks** — `src/lib/useClipMix.ts:317`, `src/lib/useClipMix.ts:353`, `useClipMix.ts:317` (+1 more)  
  In `useClipMix.ts`, tracks are only corrected when off by more than 120 ms (line 317), with a hard seek once a second (line 353). An A/V offset of up to 120 ms persists, and each correction is an audible skip.  
  *Fix:* Use a threshold of about 30-40 ms and nudge `playbackRate` by ±1-3% for small corrections, seeking only above about 150 ms. Or decode the tracks into AudioBuffers and schedule them against `ctx.currentTime`.
- **AU-40 · Medium · Preview silence watchdog trips on genuine silence or buffering and permanently switches route** — `src/lib/useClipMix.ts:238-252`, `useClipMix.ts:246-251`  
  The silence watchdog in useClipMix.ts:238-252 trips on two seconds of genuine digital silence (AAC of zeros decodes to exact zeros) or on elements still buffering, and then permanently switches to the attenuate-only route.  
  *Fix:* Switch only if no sample was non-zero since the graph was built.
- **AU-6 · Medium · An application picked by a child pid (Discord) records only that child; the other session is recorded nowhere** — `src-tauri/src/audio/mod.rs:292-298`, `src-tauri/src/audio/mod.rs:336-345`, `src/routes/AudioMixer.tsx:787-795`  
  In `audio::resolve`, `taken` excludes `app_root(live_pid)` (the whole Discord process tree) from the leftovers (mod.rs:292-298), but the application stream taps `pid: now`, the live pid itself (mod.rs:336-345). `list_processes` returns *session* pids, and for Discord those are its utility children (the repo's own `discord()` fixture); the UI stores one of them (AudioMixer.tsx:787-795).  
  *Fix:* Tap `app_root(live_pid)` in `resolve`, consistent with `taken`. Also deduplicate the process picker by root.
- **AU-7 · Medium · A capture-start timeout leaves a zombie capture thread and WASAPI client running forever** — `src-tauri/src/audio/capture.rs:127-136`  
  On the start timeout (capture.rs:127-136) the code returns `Err(_) => Err("timed out starting the audio source".into()),` without setting `stop`, and the JoinHandle is dropped. If the device's `Activate`/`Initialize` answers after 5 s, the thread sends into a dead channel and then pumps into an unreferenced ring forever, holding the WASAPI client. `retry_failed` runs every 10 s and can pile these threads up.  
  *Fix:* Call `stop.store(true, Relaxed)` before returning the timeout error; the thread checks it after init and exits.
- **AU-8 · Medium · Solo counts disabled sources and is persisted: a disabled soloed source silences every track** — `src-tauri/src/audio/mod.rs:448-451`  
  The audibility rule in audio/mod.rs:448-451 is `let any_solo = sources.iter().any(\|s\| s.solo); source.enabled && !source.muted && (!any_solo \|\| source.solo)`. `any_solo` also counts disabled sources, and solo is stored in the config, so the state survives restarts. Solo is destructive (affects what is recorded, not just monitoring).  
  *Fix:* Use `sources.iter().any(\|s\| s.enabled && s.solo)`. Clear solo on disable and/or on startup. Add a configuration warning "solo active — N tracks record silence".
- **AU-9 · Medium · Level meters ignore solo and keep moving while the buffer records zeros** — `src-tauri/src/audio/engine.rs:242`  
  `levels()` (audio/engine.rs:242) checks only `muted`, not `audible()`, so a source silenced by another source's solo still shows live meter levels.  
  *Fix:* Make the meters follow `audible()`.

## Verified lower-severity issues

| ID | Severity | Title | Where | Fix |
|---|---|---|---|---|
| AU-10 | Low (reported Medium) | UI `audible` treats soloed+muted as audible, unlike the core | src/lib/useClipMix.ts:19-21 | No separate fix given in the report; implied: make the UI's `audible` match the core rule (muted always wins). |
| AU-26 | Low | Applications running elevated are stored as "PID n" and can never be found again | src-tauri/src/audio/devices.rs:91-96, audio/devices.rs:91-98 | Take the name from the `process_tree()` snapshot, or use `QueryFullProcessImageNameW`, which needs only limited query rights. |
| AU-27 | Low | The activation completion handler can signal a closed event handle after a timeout | src-tauri/src/audio/capture.rs:177-192, src-tauri/src/audio/capture.rs:389-393 | Let the handler own the event (close it in `Drop`) and return an explicit timeout HRESULT. |
| AU-28 | Low | Log spam: `live_pid` logs every 2 s, twice per pass | src-tauri/src/audio/mod.rs:139-141 | Log only when the value changes, or write the new pid back to the config. |
| AU-29 | Low | No limiter on the summed mix | src-tauri/src/pipeline.rs:225-234, src-tauri/src/stems.rs:349-352 | Add an `alimiter`/true-peak limiter (-1 dBTP) on `[aout]`, and optionally a soft clipper in `write_mix_wav`. |
| AU-34 | Low | Leftovers mode spawns a thread and loopback client for every idle application holding a session | src-tauri/src/audio/mod.rs | No explicit fix given in the report. |
| AU-35 | Low | `default_output_id` and `list_devices` enumerate every endpoint and its property store every 2 s | src-tauri/src/audio/devices.rs | No explicit fix given; related: follow device changes via `IMMNotificationClient` instead of polling. |
| AU-37 | Low | `AppState::new` applies audio config synchronously on the main thread at startup | src-tauri/src/state.rs:308 | No explicit fix given; implied: apply the audio configuration off the main thread. |
| AU-45 | Low | No free-space check before copying the original on a first cut | src-tauri/src/edit.rs | No explicit fix given; related: keep the originals store on the same volume as the clips. |
| AU-30 | Info (reported Low) | `convert` allocates a stereo Vec on every capture packet | src-tauri/src/audio/capture.rs:251 | No explicit fix given; implied: reuse a per-stream buffer. |
| AU-31 | Info (reported Low) | `mix_window` allocates `scratch` on every 10 ms tick despite its doc comment | src-tauri/src/audio/engine.rs:355 | No explicit fix given; implied: keep and reuse the scratch buffer. |
| AU-33 | Info (reported Low) | Meter peak uses load-then-store instead of `fetch_max` | src-tauri/src/audio/ring.rs:130-133 | Use `fetch_max`. |
| AU-42 | Info (reported Low) | AudioContext leaks if `createMediaElementSource` throws | src/lib/useClipMix.ts, useClipMix.ts:207-228 | No explicit fix given; implied: close the context in the error path. |
| AU-44 | Info (reported Low) | `plan` can produce a 200 ms excerpt starting at the very end of the file | src-tauri/src/edit.rs | No explicit fix given in the report. |
| AU-48 | Info (reported Low) | Leftover German strings in log and test messages | src-tauri/src/commands.rs, src-tauri/src/edit.rs | No explicit fix given; implied: translate the strings to English. |

## Unverifiable from source or disputed

| ID | Reported | Title | What is uncertain |
|---|---|---|---|
| AU-41 | Low | Autoplay-blocked AudioContext leaves preview silent and the error is swallowed | The code facts hold: the watchdog bails out at useClipMix.ts:242 `if (!element \|\| element.paused \|\| built.ctx.state !== "running") return;`, so a context that never runs is never declared dead; resume and play errors are swallowed (useClipMix.ts:325 `ctx.resu… |
| AU-47 | Low | Export target computed in MiB while Discord may enforce decimal MB | The MiB arithmetic is not in export.rs. export.rs:39-41 takes a byte count (`pub bytes: u64`), and its MiB constant only appears in the tests (export.rs:206 `const MB: u64 = 1024 * 1024;`). Production targets are binary. commands.rs:872 has `let target_bytes… |

## Claims rejected by verification

| ID | Claim | Why it was rejected |
|---|---|---|
| AU-39 | Preview elements recreated after the fallback route never get the current `playbackRate` | src/lib/useClipMix.ts:190-198 build() creates `new Audio()` without copying playbackRate, and the only place a rate is applied is the ratechange handler (useClipMix.ts:332-334 `audio.playbackRate = element.playbackRate`, :350). But nothing in the app ever changes the video's rate: `grep playbackRat… |
| AU-46 | Colour metadata (VUI) is not carried over on re-encode | edit.rs:661-662 maps the video straight through (`-map 0:v:0`), with no video filter; the only filter_complex is the audio mix (edit.rs:654-657). video_args (edit.rs:714-741) adds only `-pix_fmt yuv420p` plus rate-control flags, which is accurate, but no explicit colour flags are needed: ffmpeg's C… |

## What is solid

- Lock order: `running` is always taken before a ring lock, so there is no deadlock.
- Buffer pairing: WASAPI `GetBuffer`/`ReleaseBuffer` are paired correctly.
- Pipes: ffmpeg stderr is drained on its own thread and stdout progress is read in parallel, so ffmpeg cannot stall on a full pipe.
- Command construction: all ffmpeg arguments are passed as an argv Vec with no shell, and filter strings contain only numeric stream specifiers; command injection is not possible.
- Seeking: each input gets its own `-ss`, and trims compose correctly in original coordinates.
- Panics: no reachable panic in the audio/edit/export modules; the `clamp` bounds are ordered, `expect("checked")` is guarded, and indexing uses `get`.

## Improvement opportunities and missing features

| ID | Idea | Why | Effort | Sketch |
|---|---|---|---|---|
| AU-P-1 | Proper multichannel downmix and high-quality resampling | Keeps centre/surround content (dialogue) in clips from 5.1/7.1 endpoints and avoids aliasing/imaging from the naive linear resampler. | ? | Initialise capture clients with `AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM \| AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY`, or an ITU downmix driven by `dwChannelMask` plus a stateful per-stream resampler (`rubato`) in audio/capture.rs. |
| AU-P-2 | Follow the Windows default device | Sources set to "default device" keep recording the right endpoint when the user switches outputs. | ? | Register an `IMMNotificationClient` and restart the affected stream on `OnDefaultDeviceChanged`. |
| AU-P-3 | MMCSS thread priority for capture and mixer threads | Prevents packet loss under game load. | ? | Call `AvSetMmThreadCharacteristicsW` ("Audio"/"Capture") on capture and mixer threads. |
| AU-P-4 | Per-source sync offset | Compensates Bluetooth/USB latency so sources line up with the picture. | ? | A per-source offset setting applied when sources are mixed (no further detail in the report). |
| AU-P-5 | Source monitoring and a non-destructive solo | Lets the user listen to a single source without silencing what gets recorded. | ? | Monitoring a source and a solo that does not destroy the recording (no further detail in the report). |
| AU-P-6 | Microphone processing | Cleaner voice in clips. | ? | Noise suppression (RNNoise/DeepFilterNet), noise gate, compressor and push-to-talk/ducking. |
| AU-P-7 | Loudness normalisation and true-peak limiting | Avoids clipping of the summed mix and delivers consistent loudness for social/broadcast targets. | ? | A true-peak limiter on the mix; EBU R128 normalisation (`loudnorm`, two-pass) on save and export (-14 LUFS for social, -23 for broadcast). |
| AU-P-8 | Export individual tracks | Enables editing in Premiere or DaVinci Resolve with separate tracks. | ? | Export individual tracks as WAV/FLAC, or as multi-track MKV/MOV. |
| AU-P-9 | Opus/WebM for sharing | Smaller, widely supported files for sharing. | ? | Add an Opus/WebM export option (no further detail in the report). |
| AU-P-10 | A waveform per track in the editor | Users can see each source's activity while editing. | ? | Render a waveform per individual track in the clip editor (see the cheaper peak computation suggested for preview.rs). |
| AU-P-11 | Export improvements: cancel, two-pass/verify-and-retry, frame-rate step, HEVC/AV1, background prior… | Exports hit the size target reliably, can be aborted, keep higher quality at a given size and do not disturb the game. | ? | A cancel button; two-pass or verify-and-retry; a frame-rate step (60->30) before resolution; HEVC/AV1 at the same size; background priority for ffmpeg. |
| AU-P-12 | Smart render for front cuts | Front cuts become fast and nearly lossless. | ? | Re-encode only up to the next keyframe and stream-copy the rest. |
| AU-P-13 | "Snap to keyframe, lossless" trim option | Instant lossless trims when frame accuracy is not needed. | ? | Offer an option that snaps cut points to keyframes and stream-copies. |
| AU-P-14 | Keep the originals store on the same volume as the clips | The first-cut move becomes an atomic rename, avoiding crash windows and copying GBs to C: without a free-space check. | ? | Store originals in e.g. a hidden `.clippiboy/originals` folder next to the clips instead of `<data>/originals/<id>/`. |
