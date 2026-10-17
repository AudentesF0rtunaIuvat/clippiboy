# Video capture, encode, replay buffer and mux

**Scope:** pipeline.rs, wgc.rs, gpu.rs, convert.rs, mft.rs, encode.rs, buffer.rs, muxer.rs, recorder.rs, capture.rs, disk.rs, the GPU readback in shot.rs.

**Findings:** 41 — 23 verified (6 medium or higher), 13 reported but not independently re-verified, 4 unverifiable/disputed, 1 rejected by verification.

Severity shown is the verified severity; where verification changed it, the reported one is in parentheses. See [README](README.md#how-findings-were-verified) for how verification worked.

## Verified issues (medium and above)

### VP-1 · High · Pacer never skips ticks and muxer discards PTS: encoder deficit or dropped frames become ever-growing A/V desync and eventually silent clips

- **Where:** `src-tauri/src/convert.rs:705-733`, `src-tauri/src/mft.rs:1184`, `src-tauri/src/pipeline.rs:535-537`, `src-tauri/src/muxer.rs:263`, `src-tauri/src/pipeline.rs:1002-1003`
- **Verification:** confirmed (reachability: confirmed; impact: confirmed; code: confirmed)

**Problem:** The frame pacer pace() in convert.rs:705-733 never skips ticks: `let due = start + period * index as u32; … if due > now { sleep } else if index > 0 { latest.overruns.fetch_add(1, …) } … index += 1; … let pts = (index as i64 - 1) * 10_000_000 / fps as i64;`. pts is derived purely from the tick index, so when the loop falls behind (encoder submit can block up to 500 ms via `send_timeout(job, SUBMIT_WAIT)` at mft.rs:1184) pts keep advancing 1/fps per frame regardless of wall-clock time. The muxer then throws packet PTS away and muxes the raw ES as CFR with `-f h264 -r fps` (muxer.rs:263), while audio is mapped via `qpc_of = base + pts_us*10` (pipeline.rs:535-537) and the snapshot sets `audio_frames = packets.len()*SR/fps` (pipeline.rs:1002-1003). A sustained encoder deficit or dropped frames therefore become ever-growing A/V desync and eventually silent clips. Overruns are only logged; nothing appears in the UI.

**Scenario:** Encoder sustains 50 fps for a 60 fps config (OBS holding NVENC as in the README's own example, 1440p165, or an iGPU under game load). The loop runs at 50 Hz but pts advance 1/60 s per frame; after 10 minutes the video timeline is about 100 s behind the wall clock. qpc_of() maps packets ~100 s too early, so the audio cut at start_100ns is from long before the picture. Once the lag exceeds the TrackRing (buffer + 2 s), window() returns zeros and the clip is silent; the 'last 30 s' also spans more than 30 s of real time. Every 500 ms submit timeout (a lost frame) shifts everything after it by one frame because the raw ES is muxed at -r fps; audio_frames only fixes the total length. Recordings accumulate both effects over hours.

**Fix:** When now − due > period, set index = elapsed/period (skip ticks) so pts stay true to the wall clock. Mux with real timestamps: a VFR container fed with packet PTS (a minimal NUT/MKV/TS writer piped to ffmpeg), or cut audio per packet at qpc_of(pts). Surface the overrun and drop rate in the UI.

**Verifier on the fix:** Skipping ticks alone is not enough and makes things worse inside a clip. pts would stay on the wall clock, but the CFR `-r fps` mux and `audio_frames = packets*SR/fps` would make a 30 s window of 1500 frames play as 25 s of video against 25 s of audio, drifting up to 5 s by the end. It must ship together with timestamp-aware muxing: a VFR container fed with packet PTS, e.g. a small MKV/NUT writer, then ffmpeg with -c:v copy. A smaller interim fix: skip ticks so pts follow the wall clock, and at save time mux with `-r frames/real_span` (span taken from first and last pts) instead of the nomina…

### VP-3 · High · Sleep/resume or a long thread stall makes the mixer allocate the whole gap in one window (OOM abort); pacer replays every missed tick

- **Where:** `src-tauri/src/pipeline.rs:780-785`, `src-tauri/src/audio/engine.rs:346-350`
- **Verification:** confirmed (code: confirmed; impact: partly confirmed; reachability: confirmed)

**What verification found:** The unbounded window and the replay of missed ticks are real. But not every long stall ends in an OOM abort: a stall of a few seconds costs a few MB. What happens depends on how long the gap is and on the commit limit. Per hour of gap, each recorded source needs about 1.38 GB of f32 (out buffer), plus a 1.38 GB f32 scratch, a 0.69 GB i16 copy, and while recording a further 0.69 GB to_vec per track. All of it is zero-filled, so every page gets touched. A 1 h sleep on a typical 16-32 GB gaming PC will probably not abort. It gives a multi-GB spike and heavy paging, and `buffers` keeps that capacity for the rest of the pipeline's life. An overnight sleep (8 h: about 11 GB per source buffer, more than 30 GB for two sources) will very likely make an allocation fail, and that aborts the process. Until then the system pages heavily. The pacer burst is bounded by the 10 s lock-screen stop only when Windows shows the lock screen on wake. Otherwise it runs at encoder speed for minutes, and live frames get stale pts until it catches up.

**Original claim:** The mixer thread sizes each window from the full elapsed gap with no cap: `let frames = ((target - from) * SAMPLE_RATE as i64 / 10_000_000) as usize; … shared.audio.mix_window(&sources, from, frames, &mut buffers);` (pipeline.rs:780-785), and AudioEngine::mix_window does `track.resize(sample_count, 0.0)` per source plus a scratch buffer of the same size (audio/engine.rs:346-350). After sleep/resume or any long thread stall the whole gap is allocated at once, which is an OOM abort. The pacer likewise replays every missed tick back-to-back. There is no WM_POWERBROADCAST handling anywhere.

**Scenario:** The PC sleeps for an hour with the buffer on; QPC/Instant keep counting across suspend. The first mixer tick after resume (before the 10 s lock-screen stop) asks for ~173M frames, about 1.4 GB of f32 per source plus scratch plus an i16 copy. Allocation fails, the process aborts, the buffer is lost and an in-progress recording loses its BufWriter tail. The pacer replays ~216k missed ticks: encoder at 100% and the ring fills with the pre-sleep still frame under wrong timestamps.

**Fix:** If target − from exceeds about 1 s, account the gap as ring silence and jump from. In the pacer, resync index when more than about 250 ms behind. Stop and restart the pipeline on PBT_APMSUSPEND / resume.

**Verifier on the fix:** The direction is right, but 'jump from' alone would break sync. TrackRing timelines are sample-counted (pipeline.rs:150-156), so skipping without counting would pull later audio forward. Minimal fix: when target-from is more than about 1 s, call a new non-allocating TrackRing::skip(samples) on every ring. It does `silence += n` with the existing reset once silence >= capacity. Feed an active recorder Job::Silence (which already exists), advance its timeline, and set next_100ns = target. In the pacer, when due lags now by more than about 250 ms, set index = elapsed/period + 1 and do not replay…

### VP-2 · Medium (reported High) · NV12 slot reuse is guarded by job count but slots rotate once per conversion, so frames queued for or held by the encoder get overwritten

- **Where:** `src-tauri/src/convert.rs:337-338`, `src-tauri/src/convert.rs:643-650`, `src-tauri/src/convert.rs:660-681`, `src-tauri/src/wgc.rs:205-208`
- **Verification:** confirmed (reachability: confirmed; code: confirmed; impact: partly confirmed)

**What verification found:** The mechanism is real. Slots are used up once per WGC conversion, but the reuse guard assumes one conversion per encoder job. Whenever conversions outrun ticks, the ring can lap frames still queued or held by the encoder. The reach is narrower than claimed. (a) A stall laps the ring only after 22 conversions (about 367 ms at 60 Hz, about 150 ms at 144 Hz). That is a section that already stutters and is close to losing frames at 500 ms, and the extra damage is a few frames showing newer or out-of-order pictures. (b) In normal running, a job's slot is overwritten about 22/(refresh/fps) ticks after conversion: about 9 ticks at 144 Hz/60 fps, 5.5 at 240 Hz/60, 4.6 at 144 Hz/30. Only setups where queue plus encoder hold reaches that many frames are hit, and only on Windows builds without MinUpdateInterval (Windows 10 and older Windows 11). The defaults keep the hold shallow: AMF and QSV run low-latency, and the code says NVENC stays shallow. So the common setup is probably safe, and a 240+ Hz NVENC user or a CLIPPIBOY_LOW_LATENCY=0 AMD user is the realistic victim. 'Torn' frames are speculative. The blit and the encoder read go through the same D3D11 device, so whole-frame substitution (temporal jumps or backward steps) is the likely symptom. The off-by-one (the job the pacer holds in send_timeout and the 'latest' slot are not counted) is real, but it only matters past the measured hold maximum of 16.

**Original claim:** Latest::submit picks one of SLOTS = 22 NV12 textures round-robin on every WGC conversion: `let slot = self.next_slot; self.next_slot = (self.next_slot + 1) % self.slots.len();` (convert.rs:337-338). The reuse guard counts jobs rather than slots: `*depth as u64 + stats.in_flight.load(..) >= SLOTS as u64` (convert.rs:646, within 643-650; pacer submit path convert.rs:660-681). Because WGC converts on its own thread pool thread independently of the pacer, textures referenced by queued jobs (bounded(4) channel) or held in flight by the encoder can be overwritten with newer pictures, or written while NVENC/AMF reads them. MinUpdateInterval is set best-effort with `let _ = session.SetMinUpdateInterval(interval);` (wgc.rs:208) and the code itself notes it is missing before Windows 11. Off-by-one: the job the pacer holds inside send_timeout is never counted.

**Scenario:** (a) Encoder stall: the pacer blocks up to 500 ms in send_timeout while WGC keeps converting (~30 conversions at 60 Hz); the 22-slot ring laps and the 4 queued jobs and in-flight frames now point at textures holding newer pictures or being written, giving out-of-order or torn frames with every counter clean; the guard only trips at in_flight ≥ 18. (b) High-refresh monitor on Windows 10 (supported per README): WGC delivers 144-240 Hz while the pacer runs at 60, so a job's slot is overwritten about 22/(refresh/fps) ticks later; any encoder holding ~5 or more frames is hit (NVENC lookahead, or AMD with CLIPPIBOY_LOW_LATENCY=0).

**Fix:** Carry the slot index in Job; keep a per-slot refcount; release a slot when the MFT drops the sample (MFCreateTrackedSample + IMFTrackedSample::SetAllocator callback); the converter takes a free slot or skips the frame. Simpler alternative: in the WGC callback only CopyResource into one 'latest BGRA' texture and convert to NV12 in the pacer at tick time, giving one conversion per job and cutting VideoProcessor and watermark GPU work 2-4× at high refresh.

**Verifier on the fix:** The refcount and IMFTrackedSample fix is correct but heavy: it must also count jobs in the channel and the one the pacer holds, not just MFT samples. The 'latest BGRA, convert at tick' alternative works and saves GPU work, at the cost of one CopyResource per WGC frame. A smaller sufficient fix is to advance next_slot only after the pacer has taken the current slot. Keep a 'taken generation' atomic set in pace(), and while it lags, convert into the same slot again (no job references it yet). That caps rotation at one slot per tick, which makes the existing job-count guard valid in both scenari…

### VP-4 · Medium (reported High) · Clip save copies the whole clip (and each WAV) into one Vec, doubling RAM and risking OOM abort with large buffers

- **Where:** `src-tauri/src/muxer.rs:131-141`, `src-tauri/src/pipeline.rs:238-256`, `src-tauri/src/config.rs:100-108`
- **Verification:** confirmed (impact: partly confirmed; reachability: confirmed; code: confirmed)

**What verification found:** Confirmed: muxer::build copies the whole clip into one Vec, and write_wav copies each PCM window a second time. Three details are wrong or missing. (1) The extra video RAM equals the clip's real byte size, which is at most the ring's contents, not the 1.5x budget. A 600 s full save adds about 2.5 GB at 1440p60 and about 5.6 GB at 4K60 nominal bitrate, on top of a ring of the same size; the 3.7 GB and 8.4 GB figures are budget ceilings. (2) It is worse than claimed in one way: `stream` stays alive until build() returns, so it overlaps the WAV writes and the whole ffmpeg encode, not just the file write. (3) The audio cost is overstated. A 600 s stereo 48 kHz window is about 115 MB, so a separate track peaks at about 230 MB and the main mix at about 345 MB. Tracks are processed one after another, so these do not add up. In the default setup (1080p60, 120 s) the extra copy is only about 285 MB. The problem only matters for users who choose long buffers at high resolution, and that same choice already puts several GB in the ring. An abort needs the Windows commit limit to be exhausted, which means a fixed or small pagefile; with a system-managed pagefile the realistic result is paging and game stutter at save time. Severity is medium, not high.

**Original claim:** muxer::build concatenates the entire clip into one in-memory buffer before writing the temp .h264: `let mut stream: Vec<u8> = Vec::with_capacity(snapshot.packets.iter().map(|p| p.data.len()).sum::<usize>() + …); … std::fs::write(&video_path, &stream)` (muxer.rs:131-141). write_wav (pipeline.rs:238-256) likewise builds the whole WAV in a Vec after window() has already allocated the samples. With the default ring budget of 1.5 × bitrate × seconds (config.rs:100-108) and a buffer slider up to 600 s, a full-length save doubles RAM use.

**Scenario:** At 600 s the ring is ≈3.7 GB at 1440p60 and ≈8.4 GB at 4K60. A full-length save allocates that much again plus ~350 MB transient per audio track at 10 min while a game is running, resulting in an abort or paging that stalls the game.

**Fix:** Stream the header and packets through a BufWriter, or pipe straight into ffmpeg (-f h264 -i pipe:0; stdout/stderr draining already exists). Write WAV samples incrementally (VecDeque::as_slices). Size the budget against GlobalMemoryStatusEx.

**Verifier on the fix:** Streaming the header and packets through a BufWriter<File> is the correct, minimal and sufficient fix for the video. It removes the copy entirely without changing encode(). Piping into ffmpeg stdin also works but is more invasive, because encode() is path-based and shared with the recorder. For WAV, write the 44-byte header and then the samples through a BufWriter, which drops the `out` copy. Keep the window() copy rather than streaming from VecDeque::as_slices: as_slices would mean holding the TrackRing lock during disk I/O, which stalls the audio mixer thread. Sizing against GlobalMemorySta…

### VP-5 · Medium (reported High) · Clip-save temp files are written to the system drive (%APPDATA%) even when clips go to another volume

- **Where:** `src-tauri/src/state.rs:856`
- **Verification:** confirmed (impact: partly confirmed; reachability: confirmed; code: confirmed)

**What verification found:** Correct: a replay-buffer clip save puts its intermediate .h264 and per-track WAVs under %APPDATA%\ClippiBoy\temp, whatever drive clip_dir is on, and the free-space check only looks at the clip drive. Wrong in detail: the data is written once to temp and read once by ffmpeg, not 'written and read back twice'. At default settings it is about 300 MB per save, not GBs; GBs only with a high bitrate and a long buffer. The failure is not a crash: std::fs::write returns an OS 'disk full' error that propagates as Err and the save is reported as failed. It only happens when C: has less free space than one buffer's worth, and the UI's free-space number (clip drive) gives no warning. Stems (data_dir/tracks) and thumbnails also land on C:, but they are small AAC/JPEG files.

**Original claim:** The save temp directory is `temp_dir: config::data_dir().join("temp")` (state.rs:856), so the temp .h264 and WAVs always go to %APPDATA% on C:, regardless of the clip output folder.

**Scenario:** Clips go to D: but C: is full: saves that would fit on D: fail. GBs are written and read back twice.

**Fix:** Put temp files on the output volume (or stream saves into ffmpeg without temp files).

**Verifier on the fix:** Writing the temp files beside the output (e.g. output.parent()/.clippiboy-temp, as recordings already do with .recording) is correct and minimal; discard_leftovers (lib.rs:677-688) would then also need to clear that folder, or the files should be named so the existing .part cleanup catches them. A better fix needs no temp video at all: the H.264 stream is already one Vec<u8> in memory (muxer.rs:131-140), so it can be piped to ffmpeg's stdin (-f h264 -i pipe:0), which removes most of the bytes. The small WAVs can stay as they are or move with it.

### VP-6 · Medium (reported High) · COM reference to the video-processor input view leaks on every frame

- **Where:** `src-tauri/src/convert.rs:336-347`
- **Verification:** confirmed (reachability: confirmed; code: confirmed; impact: partly confirmed)

**What verification found:** The leak itself is real. What leaks is not memory per frame: the refcount of the same few cached views keeps rising, and those view objects can never be freed. The memory lost is one set of textures per cache lifetime. With the default watermark on, that is about 1 canvas texture per pipeline restart, per settled resize, or per watermark-off toggle. With the watermark off, it is the 2 pool buffers. Each set also likely pins the old ID3D11Device through D3D11's child-to-device internal reference (runtime behaviour, not checkable here). The cache clear at more than 4 entries almost never happens in practice. Typical growth is about 8 MB at 1080p or 33 MB at 4K per restart, plus the device. It is larger only for window capture while a window is being resized repeatedly. Refcount overflow is not a practical concern (years at 60 fps).

**Original claim:** In convert.rs:336-347, `let input = self.input_view(...)?;` returns a `view.clone()` (an extra COM reference), which is passed as `pInputSurface: std::mem::ManuallyDrop::new(Some(input)),` into the VideoProcessor stream and is never recovered with ManuallyDrop::into_inner / dropped after VideoProcessorBlt. One reference leaks per frame.

**Scenario:** Once a cache entry goes away, the view never dies and keeps its texture and through it the whole ID3D11Device alive. Entries go away on pool Recreate (window or resolution change), watermark toggle, more than 4 entries, or the Converter drop at every pipeline restart. Restarts are frequent (per-game buffer automation, lock screen, screen drift, config changes); each leaks 1-2 frame-sized BGRA textures (8 MB at 1080p, 33 MB at 4K) plus a D3D device. Dragging a window edge during window capture leaks two textures per size step; a long-running tray app grows VRAM steadily.

**Fix:** Keep stream in a local and call ManuallyDrop::into_inner(stream.pInputSurface) after the Blt, or don't clone (transmute_copy a borrowed pointer).

**Verifier on the fix:** The proposed fix is correct and sufficient. The ref must be recovered on both the success and error paths, so do it before the `?`. Because `&[stream]` moves the struct, bind it first: `let streams = [stream]; let result = unsafe { ...VideoProcessorBlt(..., &streams) }; let [stream] = streams; drop(std::mem::ManuallyDrop::into_inner(stream.pInputSurface)); result.map_err(...)?;`. An equivalent fix is to keep `input` as an owned local and pass `ManuallyDrop::new(Some(unsafe { std::mem::transmute_copy(&input) }))`, so the normal drop of `input` balances the clone. Releasing right after Blt is s…

## Reported, not independently re-verified

The original reviewer checked these against the code, but the independent verification pass was stopped before reaching them. Treat as likely, confirm before fixing.

### VP-7 · High · No device-removed / encoder-death detection or recovery; errors only logged and a stale ring is saved as current

- **Where:** `src-tauri/src/mft.rs:1312`, `src-tauri/src/mft.rs:1344`, `src-tauri/src/mft.rs:1174-1176`, `src-tauri/src/mft.rs:1284-1290`, `src-tauri/src/pipeline.rs:605-609`
- **Verification:** not independently re-verified (not independently re-verified)

**Problem:** Encoder failures are only logged: mft.rs:1312 and mft.rs:1344 are both `Err(err) => log::warn!("{err}")`; mft.rs:1174-1176 is `Disconnected => log::warn!("encoder has stopped — one frame is lost"); return false;`; the encoder thread exits quietly on event-queue errors (mft.rs:1284-1290). Nothing calls GetDeviceRemovedReason, and only `dropped` is counted (pipeline.rs:605-609). Shared::report() is only called on the capture path and only the first error per run is surfaced, so no banner appears and no restart happens.

**Scenario:** A TDR or driver reset mid-game (also driver update, eGPU unplug, hybrid GPU switch): WGC stops delivering, the pacer repeats the last slot, the MFT fails or shuts down. The log gets ~60 warn lines per second and dropped climbs, but no banner appears. The ring keeps its pre-failure packets so 'Save clip' writes minutes-old footage as 'the last 30 s', or the ring silently fills with a frozen frame.

**Fix:** Send encoder/D3D errors through Shared::report. Add a status-tick watchdog: no new packet for more than 2 s, or GetDeviceRemovedReason() != S_OK, reports and auto-restarts with a new device. Treat D2DERR_RECREATE_TARGET the same way, and flag saves whose newest packet is stale.

- **VP-10 · Medium · Recorder job queue is an unbounded std mpsc channel** — `src-tauri/src/recorder.rs:146`  
  The recorder taps packets and audio blocks to its writer thread over `let (tx, rx) = channel::<Job>();` (recorder.rs:146), an unbounded std mpsc channel with no backpressure or fail policy.  
  *Fix:* Use a byte-bounded channel with an explicit fail-and-stop policy.
- **VP-11 · Medium · Recorder file I/O runs while holding the packet-ring and tracks locks** — `src-tauri/src/pipeline.rs:954-964`, `src-tauri/src/recorder.rs:306-322`  
  pipeline.rs:954 takes `let ring = self.shared.packets.lock();` and Recorder::start (create_dir_all, File::create, meta write + rename, thread spawn) runs at pipeline.rs:964 while that lock is held. open_track (recorder.rs:306-322) writes meta.json from the mixer thread while holding shared.tracks.  
  *Fix:* Do the file creation before taking the lock and only install the recorder under it. Move the meta writes into the writer thread.
- **VP-13 · Medium · Recording free-space guard ignores mix.pcm and stems, so finalize fails on long multi-source recordings** — `src-tauri/src/lib.rs:894-895`, `src-tauri/src/recorder.rs:580-583`  
  The room guard is `if free < needed + RECORDING_SPARE_BYTES` with `needed = estimated mp4 size` (lib.rs:894-895). It does not account for finalize's sum_tracks writing mix.pcm (recorder.rs:580-583) or per-track stems.  
  *Fix:* Count mix PCM and stems in the guard, or skip pre-summing and let ffmpeg amix all the PCMs.
- **VP-14 · Medium · VideoProcessor auto-processing is not disabled, so driver enhancements may alter footage** — `src-tauri/src/convert.rs:141-168`  
  The VideoProcessor setup (convert.rs:141-168) only sets colour spaces; there is no `VideoProcessorSetStreamAutoProcessingMode(.., FALSE)`. Auto-processing is on by default.  
  *Fix:* Disable auto-processing with VideoProcessorSetStreamAutoProcessingMode(.., FALSE).
- **VP-15 · Medium · VideoProcessor content description uses output size and sets no rects, so a resized window is stretched** — `src-tauri/src/convert.rs:141-168`, `src-tauri/src/wgc.rs:229-242`, `src-tauri/src/state.rs:774`  
  The content description in convert.rs:141-168 uses `InputWidth: width, InputHeight: height`, i.e. the output size rather than the source size, and no stream source/destination rectangle or background colour is set. The WGC frame pool is recreated on size change (wgc.rs:229-242) but window capture does not restart (state.rs:774).  
  *Fix:* Set stream source and destination rects that preserve aspect (letterbox) with a black background, and use the real source size in the content description.
- **VP-16 · Medium · ffmpeg/ffprobe runs have no timeout and no kill-on-close job object** — `src-tauri/src/muxer.rs:65-68`, `src-tauri/src/muxer.rs:432-458`, `src-tauri/src/muxer.rs:476-488` (+2 more)  
  muxer::run uses `command.output()` (muxer.rs:65-68); run_reporting (muxer.rs:432-458) does `.stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()` then `child.wait()`; ffprobe (muxer.rs:476-488) uses `.output()`. None has a timeout/kill, and children are not placed in a job object with KILL_ON_JOB_CLOSE.  
  *Fix:* Put ffmpeg/ffprobe in a job object with KILL_ON_JOB_CLOSE; wait with a timeout and kill.
- **VP-17 · Medium · ffmpeg inherits the parent's stdin (no Stdio::null, no -nostdin)** — `src-tauri/src/muxer.rs:432-458`  
  run_reporting (muxer.rs:432-458) spawns ffmpeg with piped stdout/stderr but no stdin setting and without `-nostdin`, so stdin is inherited.  
  *Fix:* Use Stdio::null() for stdin plus -nostdin.
- **VP-18 · Medium · Orphaned .part.mp4 files in the clip folder are never cleaned up** — `src-tauri/src/muxer.rs:323`, `src-tauri/src/lib.rs:677-688`, `src-tauri/src/edit.rs:422`  
  The in-progress output `<stem>_<id>.part.mp4` is written beside the target clip (muxer.rs:323), but the startup leftover cleanup (lib.rs:677-688) only cleans data_dir/temp.  
  *Fix:* Sweep stale *.part.mp4 at startup.
- **VP-19 · Medium · Closed or recreated window is never recovered during window capture** — `src-tauri/src/pipeline.rs:689-695`, `src-tauri/src/state.rs:772-776`  
  The WGC Closed handler only sets a flag and reports once (pipeline.rs:689-695). Drift detection returns None for windows: `\|\| config.target_kind != TargetKind::Monitor` (state.rs:772-776), so nothing restarts or re-targets capture.  
  *Fix:* Stop the buffer, or re-resolve the window by process and restart.
- **VP-30 · Medium · halt_pipeline holds the pipeline mutex during the stop** — `src-tauri/src/state.rs:559-561`, `state.rs:559`  
  state.rs:559-561 is `if let Some(mut pipeline) = self.pipeline.lock().take() { pipeline.stop(); }`; under Rust 2021 the temporary lock guard lives across stop().  
  *Fix:* Bind the take() result to a local first.
- **VP-8 · Medium · Hardware MFT is never shut down (no IMFShutdown / ShutdownObject / SET_D3D_MANAGER(0))** — `src-tauri/src/mft.rs:716`, `src-tauri/src/mft.rs:1440-1444`  
  The MFT is created by `candidates[pick].activate.ActivateObject()` (mft.rs:716) and the IMFActivate is dropped at the end of build. The run ends with only `ProcessMessage(MFT_MESSAGE_NOTIFY_END_STREAMING, 0)` (mft.rs:1440-1444). There is no IMFShutdown::Shutdown, no IMFActivate::ShutdownObject, and no MFT_MESSAGE_SET_D3D_MANAGER(0). Media Foundation makes the caller of ActivateObject responsible for shutdown, and async MFTs own event queues and worker threads.  
  *Fix:* Store the activate object. After the drain: SET_D3D_MANAGER(0), then cast::<IMFShutdown>()?.Shutdown(), then activate.ShutdownObject(). Send MFT_MESSAGE_NOTIFY_END_OF_STREAM before COMMAND_DRAIN.
- **VP-9 · Medium · Replay ring length and byte limits are not enforced when only one keyframe exists** — `src-tauri/src/buffer.rs:93`, `src-tauri/src/buffer.rs:128`  
  Trimming in the ReplayBuffer only runs `while self.keyframes.len() > 1 {` (buffer.rs:93), and the no-keyframe safety net returns early with `if !self.keyframes.is_empty() \|\| self.packets.is_empty() { return; }` (buffer.rs:128). With exactly one keyframe neither the pts-length nor the byte-budget limit applies and the safety net is skipped, so the ring grows without bound. The GOP read-back from the encoder is only logged. Existing tests only cover zero or many keyframes.  
  *Fix:* If no keyframe arrives within 2× GOP, force one (CODECAPI_AVEncVideoForceKeyFrame). When over budget with a single GOP, drop the ring and wait for the next IDR (memory should win, as the module's own doc says). Add a unit test for the single-keyframe case.

## Verified lower-severity issues

| ID | Severity | Title | Where | Fix |
|---|---|---|---|---|
| VP-20 | Low | Refresh-rate snapping produces 59 fps on 59.94 Hz panels | src-tauri/src/capture.rs:155-158, src-tauri/src/capture.rs:598-606 | Treat ±1 Hz around a standard step as that step, or read the rational refresh rate from QueryDisplayConfig. |
| VP-22 | Low | Encoder thread polls instead of blocking | src-tauri/src/mft.rs:1330, src-tauri/src/mft.rs:1354 | A helper thread doing a blocking GetEvent(0) and forwarding into a crossbeam channel, then select! over events and jobs (or BeginGetEvent). |
| VP-23 | Low | Encoder thread never initialises COM | src-tauri/src/mft.rs:1115-1131 | Call CoInitializeEx(MTA) at thread start. |
| VP-24 | Low | The last ~80 ms of every clip is silent | src-tauri/src/pipeline.rs:42, src-tauri/src/pipeline.rs:983-1026 | Wait up to ~100 ms for the mixer horizon to pass the clip end, or trim the trailing packets. |
| VP-25 | Low | Mixer timeline truncates per window, so long recordings drift | src-tauri/src/pipeline.rs:818 | Keep an origin plus a u64 sample counter. |
| VP-26 | Low | Pipeline stop order contradicts its own comment (capture dropped after encoder) | src-tauri/src/pipeline.rs:577-578, src-tauri/src/pipeline.rs:746-750 | Take and drop the capture first. |
| VP-31 | Low | Mixed German and English user-facing error messages | src-tauri/src/mft.rs, src-tauri/src/convert.rs | Not given explicitly in the report (implied: unify error language). |
| VP-38 | Low | convert.rs is #![cfg(windows)], so pure pacer and slot logic can't be tested | src-tauri/src/convert.rs | Not given explicitly in the report (implied: move pure logic out of the cfg(windows) module and test it). |
| VP-39 | Low | release.yml never runs cargo test | .github/workflows/release.yml | Not given explicitly in the report (implied: add cargo test to the workflow). |
| VP-43 | Low | 22 NV12 slots allocated up front waste VRAM (S4) | src-tauri/src/convert.rs | Not given explicitly here (see proposal: adaptive slot pool grown on demand using tracked samples). |
| VP-44 | Low | B-frames with raw Annex-B remux at -r, pts-filtered snapshot and discarded PTS is fragile (S5) | src-tauri/src/mft.rs, src-tauri/src/muxer.rs | Not given explicitly in the report. |
| VP-28 | Info (reported Low) | Screenshot path creates a new D3D device per screenshot | src-tauri/src/shot.rs:516 | Not given explicitly in the report (implied: reuse a device). |
| VP-29 | Info (reported Low) | Screenshot read_back runs for every arriving frame, not just the first | src-tauri/src/shot.rs:531-533 | Check whether the channel is already full before calling read_back. |
| VP-33 | Info (reported Low) | mft::run is ~225 lines and duplicates event handling in the drain loop | src-tauri/src/mft.rs | Not given explicitly in the report (implied: factor out shared event handling). |
| VP-34 | Info (reported Low) | muxer::run and run_reporting duplicate error formatting | src-tauri/src/muxer.rs | Not given explicitly in the report (implied: share one formatter). |
| VP-36 | Info (reported Low) | frames*SR/fps audio-length formula is duplicated | src-tauri/src/pipeline.rs:1002, src-tauri/src/recorder.rs:546 | Not given explicitly in the report (implied: one shared helper). |
| VP-37 | Info (reported Low) | Vestigial EncodedPacket.track / duration_us fields and audio cases in buffer tests | src-tauri/src/buffer.rs | Not given explicitly in the report (implied: remove them). |

## Unverifiable from source or disputed

| ID | Reported | Title | What is uncertain |
|---|---|---|---|
| VP-21 | Low | Async MF_E_TRANSFORM_STREAM_CHANGE handling consumes the HaveOutput event without retrying and never re-reads SPS/PPS | mft.rs:1015-1023: on MF_E_TRANSFORM_STREAM_CHANGE it calls `GetOutputAvailableType(0,0)` and `SetOutputType`, then `return Ok(None);`. mft.rs:1307-1323: the METransformHaveOutput branch treats Ok(None) as a no-op, with no retry and no `sent.pop_front()`. mft.… |
| VP-40 | Low | MinUpdateInterval equal to the frame period may skip alternate frames (S1) | src-tauri/src/wgc.rs:205-208 `if fps > 0 { let interval: TimeSpan = Duration::from_nanos(1_000_000_000 / fps as u64).into(); let _ = session.SetMinUpdateInterval(interval); }` - fps is recording.fps (pipeline.rs:671-676 passes `recording.fps`), and capture::f… |
| VP-41 | Low | MFTEnumEx called without adapter LUID; MFT may belong to a different adapter (S2) | src-tauri/src/mft.rs:219-226 `MFTEnumEx(MFT_CATEGORY_VIDEO_ENCODER, MFT_ENUM_FLAG_HARDWARE \| MFT_ENUM_FLAG_SYNCMFT \| MFT_ENUM_FLAG_SORTANDFILTER, Some(&input), Some(&output), ...)` - no adapter attribute; no LUID/MFTEnum2 anywhere in src-tauri/src (grep empty… |
| VP-42 | Low | fps choices up to panel refresh exceed H.264 Level 5.2 at 1440p/4K (S3) | src-tauri/src/capture.rs:598-606 `const STEPS = [30, 60, 120]; ... choices.push(refresh)` - fps up to panel refresh regardless of resolution; fit_to_target (capture.rs:641) only snaps fps to those steps. UI mirrors it: src/routes/Recording.tsx:14 `const HEIGH… |

## Claims rejected by verification

| ID | Claim | Why it was rejected |
|---|---|---|
| VP-35 | sum_tracks duplicates write_mix_wav | src-tauri/src/pipeline.rs:222-234 write_mix_wav: allocates `vec![0i16; frames * CHANNELS]`, sums `track.window(from_100ns, frames)` from the in-memory TrackRings, then `write_wav(path, &sum)` (with a RIFF header). src-tauri/src/recorder.rs:453-485 sum_tracks: opens the files, streams 64 KiB blocks… |

## What is solid

- ffmpeg/ffprobe command injection is not a problem: arguments go through Command::arg, there is no shell, and file names are sanitized.
- Screenshot read_back RowPitch handling is correct (shot.rs).
- MFT_OUTPUT_DATA_BUFFER ManuallyDrop handling is correct (take after ProcessOutput).
- MFTEnumEx activate array is released correctly.
- Media buffer Lock/Unlock pairing is correct.
- SetMultithreadProtected is set, plus ID3D11Multithread Enter/Leave around the D2D draw.
- ffmpeg stdout/stderr are drained concurrently, so there is no pipe deadlock.
- Atomic rename happens after the ffprobe check.
- CREATE_NO_WINDOW is set on spawned processes.
- Colour metadata (BT.709 / limited range) matches between the VideoProcessor and the media type.
- GOP trimming is correct in the multi-keyframe tests.
- Lock order across packets → recorder, tracks → recorder and tracks → ring has no cycle.

## Improvement opportunities and missing features

| ID | Idea | Why | Effort | Sketch |
|---|---|---|---|---|
| VP-P-1 | Real PTS in the mux (VFR) | Fixes finding 1 (A/V desync from CFR remux) and lets duplicate frames be dropped instead of encoded. | ? | Feed packet PTS into a VFR container (minimal NUT/MKV/TS writer piped to ffmpeg) instead of -f h264 -r fps in muxer.rs, or cut audio per packet at qpc_of(pts). |
| VP-P-2 | Device-loss watchdog and auto-restart, plus suspend/resume handling | Recover from TDR/driver resets and sleep without silent stale buffers or OOM aborts. | ? | Status-tick watchdog (no packet >2 s or GetDeviceRemovedReason() != S_OK → report + restart with new device); handle WM_POWERBROADCAST PBT_APMSUSPEND/resume by stopping and restarting the pipeline. |
| VP-P-3 | HEVC/AV1 MFTs | Better quality per bit on RTX 40, RDNA3, Arc, and level headroom for 1440p165 and 4K120. | ? | Enumerate and support HEVC/AV1 encoder MFTs alongside H.264 in mft.rs/encode.rs. |
| VP-P-4 | HDR capture support | BGRA8 capture on HDR displays looks washed out (screenshot path too). | ? | Capture R16G16B16A16Float, then tone-map to SDR, or encode HEVC Main10 PQ. |
| VP-P-5 | Scheduling priority for capture/encode threads and GPU work | Prevents the game from starving conversion and encoding. | ? | MMCSS (AvSetMmThreadCharacteristics 'Capture'/'Games') for the pacer, encoder and WGC threads; GPU priority via IDXGIDevice::SetGPUThreadPriority or D3DKMTSetProcessSchedulingPriorityClass. |
| VP-P-6 | Cheaper watermark | Removes a full-frame CopyResource plus D2D pass per frame. | ? | Composite the watermark as a second VideoProcessor input stream with alpha. |
| VP-P-7 | Resize in place | Resolution changes no longer trigger a restart that wipes the buffer (Drift::Resized). | ? | Handle resolution changes with letterboxing in the VideoProcessor instead of restarting the pipeline. |
| VP-P-8 | Capture options | More flexible capture targets and cursor control. | ? | Follow the game's monitor, multi-monitor capture, and a cursor on/off setting. |
| VP-P-9 | Adaptive slot pool | Avoids fixed 22-slot VRAM cost and slot-reuse hazards. | ? | Grow NV12 slots on demand using tracked samples (MFCreateTrackedSample). |
| VP-P-10 | RAM-aware budget and streaming saves | Prevents OOM on large buffers and avoids temp-file double writes. | ? | Size the default ring budget against available RAM (GlobalMemoryStatusEx), and stream saves into ffmpeg without temp files. |
