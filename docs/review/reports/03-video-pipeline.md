ClippiBoy review, Rust capture → encode → ring → mux slice (read-only)

Files read in full: pipeline.rs, wgc.rs, gpu.rs, convert.rs, mft.rs, encode.rs, buffer.rs, muxer.rs, recorder.rs, capture.rs, disk.rs, and shot.rs grab()/read_back(). For call paths I also read state.rs (start/stop/save/record), lib.rs (status tick, guards, leftovers), audio/engine.rs mix_window, config.rs budget, and the README sections. Nothing in the repo was modified.

## Architecture as I understood it
- **Startup:** `AppState::launch_pipeline` (state.rs) calls `capture::fit_to_target`, then `Pipeline::start`, then `win::start` (pipeline.rs:616).
- **gpu.rs:** creates one D3D11 device (BGRA+VIDEO support, `SetMultithreadProtected(true)`) on the adapter of the chosen encoder's vendor, plus a WinRT `IDirect3DDevice` for WGC.
- **mft.rs:** `VideoEncoder::start` spawns `clippiboy-encoder`, which builds the H.264 MFT. It is async (hardware), gets the DXGI device manager, rate control falls back Quality → PeakConstrainedVBR → UnconstrainedVBR, GOP is set, low latency is on for AMD/Intel. The thread then polls `GetEvent(NO_WAIT)`. Frames arrive through a crossbeam `bounded(4)` job channel. Packets (`Arc<[u8]>`) go to `on_packet`, which locks `Shared.packets`, tees to the Recorder, then pushes into the ReplayBuffer.
- **wgc.rs:** free-threaded WGC frame pool (2 BGRA8 buffers). `FrameArrived` runs on a thread-pool thread and synchronously calls `Latest::submit`, which converts the frame.
- **convert.rs conversion:** `Latest::submit` does an optional D2D watermark copy, then VideoProcessorBlt BGRA→NV12 into one of `SLOTS = 22` NV12 textures, chosen round-robin.
- **convert.rs pacer:** `pace()` runs on its own thread and ticks at `start + index*period`. Each tick takes the newest slot (or repeats the last one), sets `pts = index/fps`, and calls `EncoderSink`. The sink anchors `base_100ns = frame_qpc − pts` on the first frame and calls `VideoEncoder::submit` (`try_send`, then `send_timeout` 500 ms, else the frame is dropped).
- **buffer.rs:** `VecDeque` of packets, trimmed whole GOPs at a time by pts length and by a byte budget. `snapshot(n)` starts at the last keyframe ≤ newest−n.
- **Mixer (pipeline.rs):** a thread on a 10 ms tick, 80 ms behind QPC "now", calls `AudioEngine::mix_window`. Each source has an i16 PCM `TrackRing` (silence is counted, not stored). The mixer also feeds the recorder.
- **Clip save:** `Pipeline::snapshot` collects packet Arcs, rings, header, `start_100ns = base + first_pts` and `audio_frames = packets × SR / fps`. `muxer::build` then:
  1. writes a temp `.h264` and WAVs into `data_dir/temp`;
  2. runs ffmpeg `-f h264 -r fps -i …` plus the WAVs and amix → `<stem>_<id>.part.mp4` beside the target, plus per-track m4a stems;
  3. checks the result with ffprobe, renames it, makes a thumbnail.
- **Recording (recorder.rs):** the recorder taps packets and audio blocks over an unbounded mpsc to a writer thread, which writes `.recording/<id>/video.h264`, `track_n.pcm` and `meta.json`. On stop, `finalize` aligns the PCM to `frames × SR / fps`, sums the main mix into `mix.pcm`, then calls `muxer::encode`.
- **Timing model:** everything hangs off QPC. The video timeline is purely frame-count CFR: the muxer throws packet PTS away and uses `-r fps`.

## Findings, most severe first

**1. [High][bug] The pacer never skips ticks, and the muxer discards PTS. A sustained encoder deficit or dropped frames therefore become ever-growing A/V desync, and eventually silent clips.**
- **Where:** convert.rs:705-733, mft.rs:1184, pipeline.rs:535-537, muxer.rs:263, pipeline.rs:1002-1003.
- **Code:** convert.rs:705-733 is `let due = start + period * index as u32; … if due > now { sleep } else if index > 0 { latest.overruns.fetch_add(1, …) } … index += 1; … let pts = (index as i64 - 1) * 10_000_000 / fps as i64;`. Submit can block up to 500 ms (mft.rs:1184 `send_timeout(job, SUBMIT_WAIT)`). The mux is `-f h264 -r fps` (muxer.rs:263) and `qpc_of = base + pts_us*10`.
- **Scenario:**
  - The encoder sustains 50 fps for a 60 fps config. Causes include OBS holding NVENC (the README's own example), 1440p165, or an iGPU under game load.
  - The loop then runs at 50 Hz, but pts advance 1/60 s per frame. After 10 minutes the video timeline is about 100 s behind the wall clock.
  - `qpc_of()` maps packets about 100 s too early, so the audio cut at `start_100ns` is from long before the picture.
  - Once the lag exceeds the TrackRing (buffer + 2 s), `window()` returns zeros and the clip is silent. The "last 30 s" also spans more real time than 30 s.
  - Every 500 ms submit timeout (a lost frame) shifts everything after it by one frame, because the raw ES is muxed at `-r fps`. The snapshot's `audio_frames = packets.len()*SR/fps` only fixes the total length.
  - Recordings accumulate both effects over hours. Nothing appears in the UI; overruns are only logged.
- **Fix:**
  - When `now − due > period`, set `index = elapsed/period` (skip ticks) so pts stay true to the wall clock.
  - Mux with real timestamps: a VFR container fed with packet PTS (a minimal NUT/MKV/TS writer piped to ffmpeg), or cut audio per packet at `qpc_of(pts)`.
  - Surface the overrun and drop rate in the UI.
- **Confidence:** confirmed.

**2. [High][bug] NV12 slot reuse is guarded by job count, but slots rotate once per conversion. Frames still queued for, or held by, the encoder get overwritten.**
- **Where:** convert.rs:337-338, convert.rs:643-650, convert.rs:660-681, wgc.rs:205-208.
- **Code:** convert.rs:337-338 is `let slot = self.next_slot; self.next_slot = (self.next_slot + 1) % self.slots.len();`. convert.rs:646 is `*depth as u64 + stats.in_flight.load(..) >= SLOTS as u64`. wgc.rs:208 is `let _ = session.SetMinUpdateInterval(interval);`.
- **Scenario (a), encoder stall:** the pacer blocks for up to 500 ms in `send_timeout`, but WGC keeps converting on its own thread (about 30 conversions at 60 Hz). The 22-slot ring laps. The 4 queued jobs and the in-flight frames now point at textures holding newer pictures, or textures being written while NVENC/AMF reads them. The result is out-of-order or torn frames with every counter clean. The guard only trips at in_flight ≥ 18.
- **Scenario (b), high-refresh monitor:** the code itself notes MinUpdateInterval is missing before Windows 11, and the README supports Windows 10. WGC then delivers 144-240 Hz while the pacer runs at 60. A job's slot is overwritten about 22/(refresh/fps) ticks later. Any encoder holding about 5 or more frames is hit (NVENC lookahead, or AMD with `CLIPPIBOY_LOW_LATENCY=0`).
- **Off-by-one:** the job the pacer holds inside `send_timeout` is never counted.
- **Fix:** carry the slot index in `Job`; keep a per-slot refcount; release a slot when the MFT drops the sample (`MFCreateTrackedSample` + `IMFTrackedSample::SetAllocator` callback); the converter takes a free slot or skips the frame. A simpler alternative: in the WGC callback only `CopyResource` into one "latest BGRA" texture, and convert to NV12 in the pacer at tick time. That gives one conversion per job and also cuts VideoProcessor and watermark GPU work 2-4× at high refresh.
- **Confidence:** the logic is confirmed; how visible it is depends on hardware.

**3. [High][robustness] Sleep/resume, or any long thread stall, makes the mixer allocate the whole gap in one window, which is an OOM abort. The pacer also replays every missed tick.**
- **Where:** pipeline.rs:780-785, audio/engine.rs:346-350.
- **Code:** pipeline.rs:780-785 is `let frames = ((target - from) * SAMPLE_RATE as i64 / 10_000_000) as usize; … shared.audio.mix_window(&sources, from, frames, &mut buffers);`. engine.rs does `track.resize(sample_count, 0.0)` per source, plus a scratch buffer of the same size.
- **Scenario:**
  - The PC sleeps for an hour with the buffer on. QPC/`Instant` keep counting across suspend.
  - The first mixer tick after resume (before the 10 s lock-screen stop) asks for about 173M frames. That is about 1.4 GB of f32 per source, plus scratch, plus an i16 copy.
  - Allocation fails, the process aborts, and the buffer is lost. An in-progress recording loses its BufWriter tail.
  - The pacer replays about 216k missed ticks back-to-back: encoder at 100%, and the ring fills with the pre-sleep still frame under wrong timestamps.
  - There is no `WM_POWERBROADCAST` handling anywhere.
- **Fix:** if `target − from` exceeds about 1 s, account the gap as ring silence and jump `from`. In the pacer, resync `index` when more than about 250 ms behind. Stop and restart the pipeline on PBT_APMSUSPEND / resume.
- **Confidence:** likely for sleep (it depends on QPC counting across suspend); confirmed for long stalls.

**4. [High][perf/robustness] Saving copies the whole clip into one Vec and writes it to the system drive. With a large buffer this means double RAM and an OOM abort.**
- **Where:** muxer.rs:131-141, state.rs:856, pipeline.rs:238-256, config.rs:100-108.
- **Code:** muxer.rs:131-141 is `let mut stream: Vec<u8> = Vec::with_capacity(snapshot.packets.iter().map(|p| p.data.len()).sum::<usize>() + …); … std::fs::write(&video_path, &stream)`. state.rs:856 is `temp_dir: config::data_dir().join("temp")`. `write_wav` (pipeline.rs:238-256) builds the whole WAV in a Vec after `window()` has already allocated it.
- **Scenario:**
  - The default budget is 1.5 × bitrate × seconds (config.rs:100-108). The buffer slider allows up to 600 s, so 1440p60 ≈ 3.7 GB and 4K60 ≈ 8.4 GB of ring.
  - A full-length save allocates that much again, plus about 350 MB transient per audio track at 10 min, while a game is running. The result is an abort, or paging that stalls the game.
  - The temp copy goes to %APPDATA% (C:) even when clips go to D:. A full C: fails saves that would fit on D:, and GBs are written and read back twice.
- **Fix:** stream the header and packets through a `BufWriter`, or pipe straight into ffmpeg (`-f h264 -i pipe:0`; stdout/stderr draining already exists). Write WAV samples incrementally (`VecDeque::as_slices`). Put temp files on the output volume, and size the budget against `GlobalMemoryStatusEx`.
- **Confidence:** confirmed.

**5. [High][bug/leak] A COM reference to the video-processor input view leaks on every frame.**
- **Where:** convert.rs:336-347.
- **Code:** `let input = self.input_view(...)?;` returns a `view.clone()`, which is then passed as `pInputSurface: std::mem::ManuallyDrop::new(Some(input)),` and never `ManuallyDrop::into_inner`/dropped after `VideoProcessorBlt`.
- **Scenario:**
  - Each frame adds one reference that is never released.
  - Once a cache entry goes away, the view never dies and keeps its texture and, through it, the whole `ID3D11Device` alive. Entries go away on pool Recreate (window or resolution change), watermark toggle, more than 4 entries, or the Converter drop at every pipeline restart.
  - Restarts are frequent: the buffer automation per game, the lock screen, screen drift, config changes. Each restart leaks 1-2 frame-sized BGRA textures (8 MB at 1080p, 33 MB at 4K) plus a D3D device.
  - Dragging a window edge during window capture leaks two textures per size step. A tray app left running grows VRAM steadily.
- **Fix:** keep `stream` in a local and call `ManuallyDrop::into_inner(stream.pInputSurface)` after the Blt, or don't clone (`transmute_copy` a borrowed pointer).
- **Confidence:** the pattern is confirmed; the impact is likely.

**6. [High][robustness] There is no device-removed / encoder-death detection or recovery. Errors are only logged, and a stale ring is saved as if it were current.**
- **Where:** mft.rs:1312, mft.rs:1344, mft.rs:1174-1176, mft.rs:1284-1290, pipeline.rs:605-609.
- **Code:** mft.rs:1312 and 1344 are both `Err(err) => log::warn!("{err}")`. mft.rs:1174-1176 is `Disconnected => log::warn!("encoder has stopped — one frame is lost"); return false;`. The encoder thread exits quietly on event-queue errors (1284-1290). Nothing calls `GetDeviceRemovedReason`, and only `dropped` is counted (pipeline.rs:605-609).
- **Scenario:**
  - A TDR or driver reset happens mid-game (also a driver update, eGPU unplug, hybrid GPU switch).
  - WGC stops delivering, the pacer repeats the last slot, and the MFT fails or shuts down.
  - The log gets about 60 warn lines per second and `dropped` climbs, but no banner appears: `report()` is only called on the capture path, and only the first error per run is surfaced.
  - The ring keeps its pre-failure packets, so "Save clip" writes minutes-old footage as "the last 30 s". Alternatively the ring silently fills with a frozen frame.
- **Fix:** send encoder/D3D errors through `Shared::report`. Add a status-tick watchdog: no new packet for more than 2 s, or `GetDeviceRemovedReason() != S_OK`, should report and auto-restart with a new device. Treat D2DERR_RECREATE_TARGET the same way, and flag saves whose newest packet is stale.
- **Confidence:** likely.

**7. [Medium][robustness/leak] The hardware MFT is never shut down.**
- **Where:** mft.rs:716, mft.rs:1440-1444.
- **Code:** the MFT is created by `candidates[pick].activate.ActivateObject()` (mft.rs:716). The run ends with only `ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0)` (1440-1444). The `IMFActivate` is dropped at the end of `build`. There is no `IMFShutdown::Shutdown`, no `ShutdownObject`, and no `SET_D3D_MANAGER(0)`.
- **Scenario:** Media Foundation makes whoever calls `ActivateObject` responsible for shutting the object down, and async MFTs own event queues and worker threads. Without that, some vendor MFTs keep the encoder session, the device manager and the D3D device alive. Each buffer restart can leak an NVENC/AMF session until the consumer session cap is hit and "Buffer will not start" appears, along with VRAM growth.
- **Fix:** store the activate object. After the drain: `SET_D3D_MANAGER(0)`, then `cast::<IMFShutdown>()?.Shutdown()`, then `activate.ShutdownObject()`. Send `MFT_MESSAGE_NOTIFY_END_OF_STREAM` before `COMMAND_DRAIN`.
- **Confidence:** likely, but vendor-dependent.

**8. [Medium][bug] The ring's length and byte limits can't be enforced when only one keyframe exists.**
- **Where:** buffer.rs:93, buffer.rs:128.
- **Code:** buffer.rs:93 is `while self.keyframes.len() > 1 {`. buffer.rs:128 is `if !self.keyframes.is_empty() || self.packets.is_empty() { return; }`.
- **Scenario:** an encoder ignores the GOP setting (infinite GOP or intra-refresh; the GOP read-back is only logged) and sends only its initial IDR. Neither limit applies and the "no keyframe" safety net is skipped, so the ring grows until OOM.
- **Fix:** if no keyframe arrives within 2× GOP, force one (`CODECAPI_AVEncVideoForceKeyFrame`). When over budget with a single GOP, drop the ring and wait for the next IDR — memory should win, as the module's own doc says. Add a unit test; the current tests only cover zero or many keyframes.
- **Confidence:** the logic is confirmed; the trigger is speculative.

**9. [Medium][perf/robustness] The recorder queue is unbounded, and recorder file I/O happens under the ring and tracks locks.**
- **Where:** recorder.rs:146, pipeline.rs:954-964, recorder.rs:306-322.
- **Code:** recorder.rs:146 is `let (tx, rx) = channel::<Job>();` (std mpsc, unbounded). pipeline.rs:954 takes `let ring = self.shared.packets.lock();`, and `Recorder::start` (create_dir_all, File::create, meta write + rename, thread spawn) runs at line 964 while that lock is held. `open_track` (recorder.rs:306-322) writes meta.json from the mixer thread while it holds `shared.tracks`.
- **Scenario:** a slow or sleeping HDD, NAS, USB drive or AV scan stalls the writer. Video (about 5 MB/s at 40 Mbit) plus PCM per source then piles up in RAM indefinitely. Starting a recording also blocks the encoder's `on_packet`, so in_flight grows, submits wait or drop, and finding 2 gets worse.
- **Fix:** use a byte-bounded channel with an explicit fail-and-stop policy. Do the file creation before taking the lock and install the recorder under it. Move the meta writes into the writer thread.
- **Confidence:** confirmed.

**10. [Medium][robustness] Recording stop and finalize are fragile on a full disk.**
- **Where:** state.rs:1072-1078, lib.rs:894-895, recorder.rs:580-583.
- **Code:** state.rs:1072-1078 is `let meta = match recorder.finish(..) { Ok(meta) => meta, Err(err) => { log::error!(..); return None; } };`. That return skips both `halt_pipeline()` and `status.recording = false`. The room guard (lib.rs:894-895) is `if free < needed + RECORDING_SPARE_BYTES` with `needed = estimated mp4 size`.
- **Scenario:**
  - The writer hits a full disk; stop runs; `write_meta` fails. The UI then shows "recording" forever, and a pipeline started only for the recording keeps encoding.
  - Separately: a 2-hour recording with two or more main-mix sources. `sum_tracks` writes `mix.pcm` (recorder.rs:580-583) at about 691 MB/h, which the 512 MB spare can't cover. Finalize fails, the folder is kept, and it fails the same way on every start.
- **Fix:** always reset the status and halt the pipeline, even on error. Count mix PCM and stems in the guard, or skip pre-summing and let ffmpeg amix all the PCMs.
- **Confidence:** the paths are confirmed (a tiny NTFS-resident meta write may still succeed).

**11. [Medium][quality] The VideoProcessor setup has gaps.**
- **Where:** convert.rs:141-168, wgc.rs:229-242, state.rs:774.
- **Code:** the content description uses `InputWidth: width, InputHeight: height` — the output size, not the source size. Only the colour spaces are set. There is no `VideoProcessorSetStreamAutoProcessingMode(.., FALSE)`, no source/destination rectangle, and no background colour.
- **Scenario:**
  - Auto-processing is on by default, so driver "enhancement" features (sharpening, denoise, RTX video enhancement) may alter the footage.
  - A window resized after start (pool recreated in wgc.rs:229-242; no restart for windows, state.rs:774) is stretched to the fixed encoder size.
- **Fix:** disable auto-processing. Set stream source and destination rects that preserve aspect (letterbox) with a black background. Use the real source size in the content description.
- **Confidence:** likely for auto-processing; confirmed for stretching.

**12. [Medium][robustness] ffmpeg/ffprobe runs have no timeout and no kill-on-close job object; stdin is inherited; `.part` files can be orphaned.**
- **Where:** muxer.rs:65-68, muxer.rs:432-458, muxer.rs:476-488, muxer.rs:323, lib.rs:677-688.
- **Code:** `run` uses `command.output()` (muxer.rs:65-68). `run_reporting` (432-458) does `.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()` then `child.wait()`, with no stdin setting and no `-nostdin`. ffprobe (476-488) uses `.output()`. The `.part.mp4` (323) sits beside the clip, while lib.rs:677-688 only cleans data_dir/temp.
- **Scenario:**
  - A network share vanishes mid-write. ffmpeg blocks forever, `SavingGuard` stays set, and every later save says "already being saved" until restart.
  - A crash during a save leaves an orphaned ffmpeg and a `<stem>_<id>.part.mp4` in the user's clip folder forever.
  - Started from a console, ffmpeg reads that console's stdin.
- **Fix:** `Stdio::null()` plus `-nostdin`; a job object with KILL_ON_JOB_CLOSE; a wait with timeout and kill; sweep stale `*.part.mp4` at startup.
- **Not a problem:** command injection — arguments go through `Command::arg`, there is no shell, and file names are sanitized.
- **Confidence:** confirmed.

**13. [Medium][robustness] A closed or recreated window is never recovered during window capture.**
- **Where:** pipeline.rs:689-695, state.rs:772-776.
- **Code:** the Closed handler only sets a flag and reports once (pipeline.rs:689-695). Drift detection returns None for windows: `|| config.target_kind != TargetKind::Monitor` (state.rs:772-776).
- **Scenario:** the game window closes or restarts with a new HWND. After one banner, the buffer encodes the last frame forever.
- **Fix:** stop the buffer, or re-resolve the window by process and restart.
- **Confidence:** confirmed.

**14. [Low][bug] Refresh-rate snapping produces 59 fps on 59.94 Hz panels.**
- **Where:** capture.rs:155-158, capture.rs:598-606.
- **Code:** the rate comes from integer `dmDisplayFrequency`, and `choices.push(refresh)` adds it as a step.
- **Scenario:** a 59.94 Hz panel reports 59, so a 60 fps config snaps to 59 fps CFR. The pacer then misses about one WGC frame per second (judder). The same happens at 119/143/164 Hz.
- **Fix:** treat ±1 Hz around a standard step as that step, or read the rational refresh rate from QueryDisplayConfig.
- **Confidence:** likely.

**15. [Low][bug] Async `MF_E_TRANSFORM_STREAM_CHANGE` handling consumes the event without retrying.**
- **Where:** mft.rs:1015-1023, mft.rs:896-911.
- **Code:** on stream change it sets the new output type, then `return Ok(None)`, which consumes the METransformHaveOutput event without calling ProcessOutput again. The SPS/PPS is never re-read afterwards, because `catch_up_on_header` stops at the first non-empty header.
- **Effect:** one packet is delayed or stuck, in-flight pairing is off by one, and a stale header may be prepended to clips.
- **Fix:** call ProcessOutput again immediately; re-read the header; as a fallback, extract SPS/PPS (NAL 7/8) from the first IDR packet.
- **Confidence:** likely.

**16. [Low][perf] The encoder thread polls instead of blocking.**
- **Where:** mft.rs:1330, mft.rs:1354, mft.rs:1406-1411.
- **Code:** `jobs.recv_timeout(2ms)` (1330), `sleep(1ms)` (1354), and a busy-polling drain loop (1406-1411).
- **Effect:** 500-1000 wakeups per second. crossbeam's timed park may round up to the 15.6 ms system timer, so HaveOutput events can wait.
- **Fix:** a helper thread doing a blocking `GetEvent(0)` and forwarding into a crossbeam channel, then `select!` over events and jobs (or BeginGetEvent).

**17. [Low][robustness] The encoder thread never initialises COM.**
- **Where:** mft.rs:1115-1131.
- **Detail:** MFStartup, MFTEnumEx and ActivateObject run without CoInitializeEx. This only works because other threads (audio `ensure_com`, windows-rs CoIncrementMTAUsage) keep the implicit MTA alive.
- **Fix:** call CoInitializeEx(MTA) at thread start.

**18. [Low][bug] The last ~80 ms of every clip is silent.**
- **Where:** pipeline.rs:42, pipeline.rs:983-1026.
- **Detail:** `AUDIO_LAG_100NS = 80ms`, and the snapshot doesn't wait for the mixer to catch up, so `window()` fills the tail with zeros (80 ms minus encoder latency).
- **Fix:** wait up to ~100 ms for the mixer horizon to pass the clip end, or trim the trailing packets.
- **Confidence:** likely.

**19. [Low][bug] The mixer timeline truncates, so long recordings drift.**
- **Where:** pipeline.rs:818.
- **Code:** `next_100ns = Some(from + frames as i64 * 10_000_000 / SAMPLE_RATE as i64);` floors away 1/3 to 2/3 of a 100 ns tick per window.
- **Effect:** the recorder's contiguous PCM drifts behind the video by about 12 ms per hour. The replay ring's trimming cancels the error, so clips are unaffected.
- **Fix:** keep an origin plus a u64 sample counter.
- **Confidence:** likely (the maths checks out).

**20. [Low][quality] The stop order contradicts its own comment.**
- **Where:** pipeline.rs:577-578, pipeline.rs:746-750.
- **Detail:** the field comment says "Has to drop before the encoder", but `stop()` drops the encoder explicitly first, and `_capture` only goes when the function returns. WGC keeps converting and watermarking into slots throughout the ≤2 s drain.
- **Fix:** take and drop the capture first.

**21. [Low][perf] MFTs are re-enumerated on every config write.**
- **Where:** encode.rs:164-171, state.rs:367.
- **Detail:** `encode::resolve()` → `list_encoders()` → MFTEnumEx runs on every `replace_config`, which includes every gain-slider step; `preferred_encoder` enumerates again.
- **Fix:** cache the result.

**22. [Low][perf] Screenshot path does extra work.**
- **Where:** shot.rs:516, shot.rs:531-533.
- **Detail:** a new D3D device is created per screenshot (516). `let _ = sender.try_send(read_back(&gpu, frame.texture));` (531-533) runs the staging copy, a blocking Map and a 24 MB (4K) conversion for every frame that arrives before the drop, not just the first.
- **Fix:** check whether the channel is already full before calling `read_back`.
- **Verified OK:** RowPitch handling is correct.

**23. [Low][quality] `halt_pipeline` holds the pipeline mutex during the stop.**
- **Where:** state.rs:559-561.
- **Detail:** `if let Some(mut pipeline) = self.pipeline.lock().take() { pipeline.stop(); }` — under Rust 2021 the temporary guard lives across `stop()`, so `is_buffering`, save and start callers block for up to the 2 s drain.
- **Fix:** bind the `take()` result to a local first.

**24. [Low][quality] Code health.**
- German and English user-facing errors are mixed; these reach banners via `?` ("Encoder starten", "Paket abholen", "Taktgeber", "NV12-Textur fehlt").
- Stale doc at state.rs:26-27: it says `save_clip` holds `pipeline` across the ffmpeg run, which is no longer true.
- `mft::run` is about 225 lines and duplicates the event handling in the drain loop.
- `muxer::run` and `run_reporting` duplicate error formatting.
- `sum_tracks` duplicates `write_mix_wav`.
- The `frames*SR/fps` formula appears twice (pipeline.rs:1002, recorder.rs:546).
- `EncodedPacket.track` / `duration_us` and the audio cases in the buffer tests are vestigial.
- `convert.rs` is `#![cfg(windows)]`, so the pure pacer and slot logic can't be tested.
- release.yml never runs `cargo test`.

**Speculative items**
- **S1:** MinUpdateInterval set exactly equal to the frame period may skip alternate frames because of jitter. Since the pacer does CFR anyway, use about 0.5-0.9 × the period.
- **S2:** MFTEnumEx is called without an adapter LUID. On same-vendor multi-GPU machines (Ryzen APU + dGPU) the MFT may belong to a different adapter than the device. Use MFTEnum2 with the device's adapter LUID.
- **S3:** The fps choices go up to the panel's refresh (165/240). At 1440p/4K that exceeds H.264 Level 5.2 (1440p ≤ ~144 fps, 4K ≤ ~64 fps), so the MFT may refuse the type or emit out-of-level streams.
- **S4:** 22 NV12 slots are allocated up front: about 68 MB at 1080p, 122 MB at 1440p and 273 MB at 4K of VRAM, even with a near-zero queue.
- **S5:** B-frames (`CLIPPIBOY_BFRAMES` or a driver default) combined with raw Annex-B remux at `-r`, a pts-filtered snapshot and discarded PTS is fragile.

**Verified correct**
- `MFT_OUTPUT_DATA_BUFFER` ManuallyDrop handling (`take` after ProcessOutput).
- MFTEnumEx array release.
- Media buffer Lock/Unlock pairing.
- SetMultithreadProtected, plus ID3D11Multithread Enter/Leave around the D2D draw.
- ffmpeg stdout/stderr drained concurrently, so no pipe deadlock.
- Atomic rename after the ffprobe check.
- CREATE_NO_WINDOW is set.
- Colour metadata (BT.709 / limited range) matches between the VideoProcessor and the media type.
- GOP trimming is correct in the multi-keyframe tests.
- Lock order across packets → recorder, tracks → recorder and tracks → ring has no cycle.

## Improvement opportunities / missing features
- **Real PTS in the mux (VFR).** This fixes finding 1 and lets duplicate frames be dropped instead of encoded.
- **Device-loss watchdog and auto-restart.** Also handle suspend/resume.
- **HEVC/AV1 MFTs** (RTX 40, RDNA3, Arc). Better quality per bit, and level headroom for 1440p165 and 4K120.
- **HDR.** Capture R16G16B16A16Float, then tone-map to SDR, or encode HEVC Main10 PQ. BGRA8 capture on HDR displays looks washed out (the screenshot path too).
- **Scheduling priority.** MMCSS (`AvSetMmThreadCharacteristics` "Capture"/"Games") for the pacer, encoder and WGC threads. GPU priority via `IDXGIDevice::SetGPUThreadPriority` or `D3DKMTSetProcessSchedulingPriorityClass`, so the game can't starve conversion and encoding.
- **Cheaper watermark.** Composite it as a second VideoProcessor input stream with alpha, instead of a full-frame CopyResource plus a D2D pass on every frame.
- **Resize in place.** Handle resolution changes with letterboxing instead of a restart that wipes the buffer (Drift::Resized).
- **Capture options.** Follow the game's monitor, multi-monitor capture, and a cursor on/off setting.
- **Adaptive slot pool.** Grow NV12 slots on demand using tracked samples.
- **RAM-aware budget.** Size the default ring budget against available RAM, and stream saves without temp files.
