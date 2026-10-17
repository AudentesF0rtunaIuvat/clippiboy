# ClippiBoy screenshot / image editor: deep review (read-only)

I read every line of ShotAnnotate.tsx, ShotViewer.tsx, watermark.ts, pictureBox.ts, shot.rs, watermark.rs, clipboard.rs and thumbs.rs. I also read commands.rs end to end (focus on 394-668), the relevant parts of ipc.ts, types.ts, store.ts, Clips.tsx, clipMenu.tsx, ColorPicker/Controls/TextMenu, filing.rs, edit.rs (dir/sanitize/remove/repair), share.rs (receive path), links.rs and state.rs (`take_screenshot`). Nothing in the repo was modified.

## How the editor works, end to end

1. **Capture** (`state.rs:889-965`). `shot::grab` → `watermark::stamp` burns the mark into the PNG. The pixels that were under the mark go to `originals/<id>/under.png` plus `under.json`. ffmpeg writes the thumbnail.
2. **Open.** Clips.tsx mounts `ShotViewer` with `index = min(open, len-1)`. A mount effect calls `screenshot_edit(id)` (a sync command on the main thread). It returns `{crop, marks (opaque JSON), basePath (base.png, else image.png), originalPath, originalWidth/Height, underPath/underRect}`.
3. **Three modes, three "grounds".**
   - view: the finished file.
   - crop: the untouched `image.png`, with `CropLayer` working in original pixels.
   - draw: `base.png` (original with the crop, no marks), with `AnnotateLayer` working in crop-relative pixels.
4. **Preview.** `toSteps` splits shapes into "layer" runs and "blur" steps.
   - Each layer run is a `<canvas>` at full picture resolution, scaled down by CSS.
   - Each blur is a DOM div with CSS `backdrop-filter: blur(σ)`, where σ = √(r(r+1)/3).
   - The under-piece and the new watermark position (`markRect`, a TS copy of `rect_kept`) are drawn as `<img>` on top.
5. **Save** (`write`).
   - Shapes are shifted into original coordinates.
   - Each layer run is re-painted on a fresh full-size canvas, encoded with `toBlob` to PNG, and turned into `number[]`.
   - `write_screenshot(id, crop, steps, marksJSON)` goes over JSON IPC.
6. **Rust** (`edit_screenshot`).
   - `keep_original` copies the file to `image.png` once.
   - Decode the original and paste `under.png` back (removes the mark).
   - Clamp the crop, then compute `base` = crop(pristine).
   - Apply steps in order: `blur` is a separable one-pass box with a margin; `blend` decodes the layer PNG.
   - Crop, then `stamp_kept` (re-stamps the mark at the new corner).
   - Write `base.png` (in place), then `shot.json` (atomically via `.part`), then the main PNG (temp + rename).
   - ffmpeg rebuilds the thumbnail; `set_picture` updates width, height and size; `clips-changed` fires.
7. **Reload.** The UI calls `screenshot_edit` again, resets its shapes and history, and returns to view mode.
8. **Copy picture.** `copy_clip_image` decodes the PNG, builds a 24-bit `CF_DIB` (`Shot::dib`), then `GlobalAlloc`, copy, `SetClipboardData`.

**Checked and found sound:**
- Ids are sanitised everywhere (`muxer::sanitize`: ASCII alphanumerics, everything else `_`), and every shot command looks the id up in the library first. No path traversal.
- Serde shapes match TS: `Step` is internally tagged; ShotEdit, Rect and Clip are camelCase.
- `markRect` matches `rect_kept` exactly, including f32 vs f64 rounding (I checked the .5 cases).
- Zoom and pan maths are correct.
- `blur_rows` is tested against a naive version.
- No `u32` width×height overflow is reachable.

## Findings (most severe first)

### 1. [High] [security] The redaction blur in the file is much weaker than the preview shows
- **Where:** `src-tauri/src/shot.rs:139-210`, `633-661`; `src/components/ShotAnnotate.tsx:1063`, `1079-1081`.
- **What's wrong:**
  - The file gets one box pass per axis: `blur_rows(&patch, &mut across, pw, ph, radius); … blur_rows(&turned, &mut done, ph, pw, radius);`.
  - The preview is a CSS Gaussian with the same variance: `backdropFilter: blur(${sigmaOf(step.radius) * box.scale}px)` with `Math.sqrt((radius*(radius+1))/3)`.
  - Matching the variance does not match legibility. A box kernel has sinc side-lobes; a Gaussian has none. I computed both: for r = 4/8/16 (the 1080p minimum and default), the box lets through about 22% of the contrast at periods of 6.3/11.9/23 px. The equal-variance Gaussian lets through about 3.5% — roughly 6× less.
  - Those periods are exactly text-stroke and letter scale. The hint "What you see here is what lands in the file" (ShotViewer.tsx:1107) is therefore false in the dangerous direction.
  - The Rust doc comment (shot.rs:129-134) says the preview "blurs the whole picture and clips afterwards". That is likely inaccurate: Chromium's backdrop-filter samples only within the element (mirror edges), so near edges the two differ the other way.
- **Failure scenario:** A user blurs a 14 px password or e-mail at the default radius. The preview looks fully smeared, but the PNG keeps ghosted strokes. Those are readable, or brute-forceable against the known game or UI font.
- **Fix:**
  - Approximate a Gaussian with 3 box passes per axis, keeping the variance: `r_i = ((1.0 + 4.0*r*(r+1)/3.0).sqrt() - 1.0) / 2.0`, rounded and ≥ 1. Run `blur_rows` 3× each way and widen the margin to `3*r_i`. Cost stays O(n).
  - Consider a minimum radius tied to the rectangle height (e.g. ≥ h/4) or pixelate-then-blur, and offer a solid "redact" fill.
  - Fix the comment.
- **Confidence:** likely. The maths is certain; the real-world readability depends on content.

### 2. [Medium] [bug] Sidecars are committed before the main file, so a failed or crashed save leaves the store and the PNG out of sync. Several writes are not atomic.
- **Where:** `commands.rs:589-598`, `606-620`, `627-638`; `shot.rs:251-258`, `385-400`.
- **What's wrong:**
  - Order: `shot.write_png(&base_path)` (`File::create` truncates in place), then `write_edit(...)`, then `write_picture` (temp + `replace_file`, retried 10× then fails "Is it open somewhere right now?").
  - The comment at 555 ("Everything that can fail happens before the first byte is written") is not true for the three writes.
- **Failure scenarios:**
  - The screenshot is open in an external viewer that locks it, or AV holds it. `replace_file` fails, but `base.png` and `shot.json` already describe the new crop and marks while the PNG is the old one. The UI catches the error and keeps its old `edit`. The next open shows a state the file doesn't have.
  - Disk full during `base.png` leaves a truncated `base.png`, so draw mode shows "File not found".
  - A crash or disk-full during `keep_original`'s `std::fs::copy` leaves a partial `image.png`. `has_original` is then true forever, so every later edit, including Reset, fails in `read_png(original)`.
- **Fix:**
  - Write all three to temp names first: `base.png.part`, `shot.json.part`, `<name>.<uuid>.new.png`.
  - Then `replace_file` the main PNG, the operation most likely to fail. Only if that succeeds, rename the sidecar parts (or remove `base.png`). On failure, delete the parts.
  - `keep_original`: copy to `image.png.part` and rename.
  - `write_under`: do the same for `under.png` and `under.json`.

### 3. [Medium] [bug] A save racing a file move or delete leaves an orphaned edited PNG and leaves the database pointing at the unedited file
- **Where:** `commands.rs:518`, `632-651`; `src/routes/Clips.tsx:327-336`; `filing.rs:114-139`.
- **What's wrong:**
  - `edit_screenshot` reads `clip.path` once at the start and writes to it about 0.5–2 s later. Nothing locks the clip.
  - Closing the viewer runs `for (const id of seen) if (left.has(id)) void fileClip(id);`. `file_clip` moves the file (favorites or game folder) and updates the path in the database.
  - The close button, Escape after Discard, and the footer Delete button all stay enabled while `busy`.
- **Failure scenario:**
  - The user changes the game or hits the heart in the viewer, clicks Save marks, and closes right away.
  - `filing::place` renames P1 to P2 (and may prune P1's folder). `write_picture` then creates a new file at P1 (rename onto a missing target succeeds) and runs `set_picture` with the edited dimensions.
  - Result: the database points at the unedited P2 with wrong dimensions, the thumbnail comes from the orphan at P1, and P1 stays in the user's folder forever.
  - Delete during a save similarly recreates `originals/<id>/…` and the PNG as orphans.
- **Fix:**
  - Add a per-clip busy set in `AppState`. `write_screenshot` holds it; `file_clip` and `delete_clip` wait for it or skip.
  - Re-read `lib.get(id).path` immediately before `replace_file`, inside the library lock.
  - Disable Close and Delete while `busy`, or defer `fileClip` until the save resolves.
- **Confidence:** likely (I traced both paths; the window is about one save long).

### 4. [Medium] [bug] The editor is usable before its store loads, and a failed load is swallowed; a save then deletes the crop and old marks and misplaces blurs
- **Where:** `ShotViewer.tsx:341-356`, `426-448`, `956`, `1066`.
- **What's wrong:**
  - The load is `api.screenshotEdit(id).then(...).catch(() => {})`, and tools are enabled with `disabled={!inTauri || broken}` — no check of `edit`.
  - While `edit` is null: `full = clip.width` (the cropped size), `origin = {0,0}`, draw mode draws on `clip.path` (marks already baked in), and Save sends `write(edit?.crop ?? null, shapes)`, i.e. crop = null.
- **Failure scenarios:**
  - `screenshot_edit` errors (e.g. "clip database is not available"), or the main thread is busy so it resolves late.
  - On a cropped shot with only blur marks: the crop is silently removed, previously saved marks are dropped from `shot.json`, and the blur rectangles (crop-relative) are applied to the uncropped original at the wrong spot. The redaction misses its target.
  - With non-blur marks, Rust fails with "The layer does not fit the picture."
  - If the load resolves late, `resetShapes(readMarks(loaded))` silently wipes whatever was drawn meanwhile.
- **Fix:** Disable tools and Save until `edit !== null`. Show the load error instead of swallowing it. Have `write` refuse to run while `edit` is null.
- **Confidence:** confirmed code path; low likelihood.

### 5. [Medium] [bug] A save's async continuation isn't tied to the clip it started for: discard, navigate, and the old picture's edit lands on the new picture
- **Where:** `ShotViewer.tsx:604-644`; Discard is always enabled at `1089`; the step buttons are enabled once `mode === "view"` (`1249`, `1259`).
- **What's wrong:** After `await writeScreenshot(...)`, the code runs `const loaded = await api.screenshotEdit(clip.id); setEdit(loaded); resetShapes(readMarks(loaded));`. The closure's `clip` is the old one. Nothing checks that the viewer still shows it.
- **Failure scenario:**
  - Save, then Discard (mode → view), then → to the next shot. The new shot's load resolves fast; the old continuation resolves later and wins.
  - The viewer now holds the previous screenshot's `edit` (its `basePath`, crop and marks) for the new picture.
  - Entering draw mode shows the wrong base. Saving writes the foreign crop and marks into the new screenshot.
- **Fix:** Keep `const shownId = useRef(id)` and bail out after each await when `shownId.current !== clip.id`. Better: have `write_screenshot` return the new `ShotEdit` directly.

### 6. [Medium] [bug] Marks drawn while a save is running are silently thrown away
- **Where:** `ShotViewer.tsx:880-892` (AnnotateLayer stays interactive during `busy`) and `632-638`.
- **What's wrong:** When the save finishes, `resetShapes(readMarks(loaded)); … setMode("view")` replaces everything drawn since Save was clicked.
- **Failure scenario:** On a 4K picture (about a 1 s save), the user adds one more arrow right after clicking Save. The arrow vanishes without a question, and there is no undo.
- **Fix:** Block input while `busy` (`pointer-events: none` overlay or BusyVeil). Alternatively, snapshot `current.current` at save time and re-apply any newer shapes as unsaved.

### 7. [Medium] [security] PNG decompression bomb in `read_png` and `read_rgba` (allocation before any size check); reachable from friend-shared screenshots
- **Where:** `shot.rs:455-472` (`let mut buffer = vec![0u8; reader.output_buffer_size()];`) and `676-689`; `share.rs:633-692` (received files go straight into the library).
- **What's wrong:**
  - The frame buffer is sized from IHDR alone. png 0.17's `Limits` only cap row and chunk buffers, not this caller-allocated buffer.
  - `read_rgba` checks the layer's dimensions against the picture only *after* the full allocation and decode (`blend`, shot.rs:105-108).
- **Failure scenario:**
  - A malicious or compromised friend sends a ~100-byte "screenshot" whose IHDR says 60000×60000 (about 14 GB RGBA).
  - The user right-clicks it and chooses Copy picture (`copy_clip_image`), or edits it.
  - `vec!` fails, `handle_alloc_error` aborts the whole app, replay buffer included. If the allocation succeeds instead, the RGB conversion and `dib()` allocate another 2–3×.
- **Fix:**
  - After `read_info`, reject anything with `w as u64 * h as u64 > 1<<28` (and dimensions > 16384), then allocate with `try_reserve_exact`.
  - Give `read_rgba` the expected `(w, h)` and reject on mismatch before allocating.
  - At receive time, read the header and store the real dimensions (see 11).
- **Confidence:** the code path is confirmed; that the png crate does not cap this buffer is likely.

### 8. [Medium] [perf] Layer PNGs travel as JSON arrays of numbers and are buffered by serde as `Content`
- **Where:** `ShotViewer.tsx:1607-1624` (`return Array.from(new Uint8Array(await blob.arrayBuffer()));`); `types.ts:527`; `commands.rs:400-419` (`#[serde(tag = "kind")] … Layer { png: Vec<u8> }`).
- **What's wrong:**
  - Each byte becomes about 3.6 JSON characters plus a JS `Number`; the stringify runs on the UI thread.
  - Because `Step` is internally tagged, serde first buffers the whole object into `Content`: one ~32-byte `Content::U64` per PNG byte, then copies it into `Vec<u8>`.
  - A 2 MB layer (text-heavy, large fills, 8K) costs roughly 7 MB of JSON plus 64 MB of transient Rust memory.
- **Fix:**
  - Send layers as raw IPC bodies: `invoke("write_shot_layer", bytes, { headers: { id, index } })`, and in Rust read `tauri::ipc::Request` → `InvokeBody::Raw`.
  - Or at minimum base64 strings with `#[serde(with = "...")]` and an adjacently or externally tagged enum.
  - Encode in a worker with `OffscreenCanvas.convertToBlob`.

### 9. [Medium] [perf] Full-resolution canvases everywhere in the preview, plus a whole-panel re-render on every pointer move
- **Where:** `ShotAnnotate.tsx:566-586` (`<canvas width={width} height={height}>` = picture pixels), `679-683`, `796-802`; `ShotViewer.tsx:609-618`.
- **What's wrong:**
  - Every blur splits the runs, so N blurs mean N+1 canvases of 3840×2160 (33 MB each), and 132 MB each at 8K, plus the draft canvas.
  - Save allocates another N full-size canvases at once (`Promise.all(... layerBytes ...)`). At 8K this can fail; `getContext`/`toBlob` → null throws and is swallowed (see 16).
  - A move or resize drag repaints the entire run containing the mark (all of its siblings) on a full-res canvas.
  - Each move also calls `editShapes` → `setShapes` in ShotViewer, re-rendering the whole 1650-line tree: sidebar, ColorPicker, ClipMeta.
- **Fix:**
  - Size preview canvases to `round(box.width*devicePixelRatio)` with `ctx.setTransform(scale…)`, repainting on zoom.
  - During a drag, lift the dragged mark into its own layer, as is already done for drafts. Throttle with rAF.
  - Memoise the side panel.
  - Serialise `layerBytes` (or use OffscreenCanvas in a worker).

### 10. [Medium] [bug / privacy] A failed thumbnail rebuild keeps the pre-edit (unredacted) thumbnail with no signal
- **Where:** `commands.rs:640-643` (`if let Err(err) = make_still(..) { log::warn!(..) }`); `clips.rs:290-302` (`set_picture` doesn't touch `thumb_path`).
- **What's wrong:** On failure the old `thumbs/<id>.jpg` stays. The gallery and console tiles use `?v=${clip.sizeBytes}`, which changes, so they re-fetch the *old* JPEG.
- **Failure scenarios:**
  - When ffmpeg is unavailable or fails at edit time (it was fine at capture), a 480-px thumbnail of the unblurred content remains in the gallery and console.
  - If the capture-time thumbnail failed, `thumb_path` stays NULL forever even after a successful rebuild.
- **Fix:** Build the thumbnail in-process from the `picture` already in memory (box downscale + the existing `png` crate; drop the ffmpeg dependency for stills). On any failure, delete the old thumbnail and set `thumb_path` to NULL. Have `set_picture` also set `thumb_path`.

### 11. [Low/Medium] [bug] The editor trusts database dimensions when there is no original yet; received screenshots carry sender-claimed dimensions and may not be PNG
- **Where:** `commands.rs:457-461` (`} else { (clip.width, clip.height) }`); `share.rs:129-136`, `841-851` (`width: meta.width, height: meta.height`); `share.rs:1187-1190` accepts `jpg/jpeg` for screenshots.
- **Failure scenarios:**
  - A friend's screenshot with wrong meta: the canvas, `full` and the CropLayer are sized wrongly, and `object-contain` letterboxes the real image inside a box of the wrong aspect. Marks and blurs misalign, and Rust rejects layers ("does not fit").
  - A `.jpg` screenshot: `keep_original` copies the JPEG to `image.png`, then `read_png` fails forever. Copy picture also fails.
- **Fix:** In `screenshot_edit` always use `size_of_png(clip.path)` when there is no original. At receive time, verify the header (PNG magic, sane dimensions) and store the real dimensions; reject non-PNG screenshots.

### 12. [Low/Medium] [bug] AnnotateLayer and CropLayer never clear an interrupted drag
- **Where:** `ShotAnnotate.tsx:917-925` (only down/move/up), `764-767` (`onMove` never checks `event.buttons`); `ShotViewer.tsx:1408-1414`.
- **What's wrong:** The stage's own pan handler already knows the case: "Let go somewhere the stage never heard of it (Alt+Tab mid-drag)" (`ShotViewer.tsx:786-790`). The two layers don't handle it.
- **Failure scenario:** After Alt+Tab or a lost capture mid-drag, `drag.current` or `draft` stays set. The selected mark (or the crop) follows the hovering mouse with no button held and keeps adding to the same undo step until the next click.
- **Fix:** Add `onPointerCancel` and `onLostPointerCapture` handlers that reset `drag.current` and the draft, and ignore moves with `event.buttons === 0`.

### 13. [Low/Medium] [privacy / design] The unredacted original lives forever in `%APPDATA%\ClippiBoy\originals\<id>\image.png`, with no way to purge it short of deleting the screenshot
- **Where:** `shot.rs:287-289`; `edit.rs:139-151` (`discard_original` only knows `video.mp4` and returns "There is no original to throw away." for screenshots).
- **Fix:** Add a "Flatten — discard history" action that removes `image.png`, `base.png` and `shot.json` (keeping `under.*`). Mention in the UI that blur and crop stay reversible locally.

### 14. [Low/Medium] [bug] External edits are overwritten silently
- **Where:** `commands.rs:539` (rebuilds from the stored original every time).
- **Failure scenario:** After the first ClippiBoy edit, the user edits the PNG in Paint or Photos. The next ClippiBoy save rebuilds from the stale `image.png` and drops those changes. The database dimensions are also stale.
- **Fix:** Record the size and mtime (or a hash) of the last written file in `shot.json`. On mismatch, warn, or adopt the external file as the new original.

### 15. [Low/Medium] [perf / architecture] `#[tauri::command(async)]` on a sync fn runs on a tokio worker (likely)
- **Where:** `write_screenshot`, `copy_clip_image`, `take_screenshot`.
- **What's wrong:** They run blocking work on async runtime workers: PNG decode and encode, blur loops, `replace_file`'s sleep-retry (up to about 2.75 s), the blocking ffmpeg wait, and `Clipboard::open`'s sleeps. This can stall other async tasks (share, links, updater).
- **Fix:** Make them `async fn` and wrap the body in `tauri::async_runtime::spawn_blocking`.
- **Confidence:** likely (Tauri 2.11 macro behaviour, not checked against its source here).

### 16. [Low] [bug] Failures are swallowed
- **Where / what's wrong:**
  - `ShotViewer.tsx:639-641` `catch { /* "The notice is already in the store." */ }`. That's true only for `writeScreenshot`. A throw in `layerBytes` (`getContext`/`toBlob` null) or in the follow-up `screenshotEdit` shows nothing, and the UI stays "unsaved" although the file may have been written.
  - `clipMenu.tsx:170` `onSelect: () => void api.copyClipImage(clip.id)` has an unhandled rejection and gives no success or failure toast (e.g. "Another program is holding the clipboard").
- **Fix:** Set `lastError` in those catches. Add `.then(() => notify("Picture copied"), err => set({ lastError }))`.

### 17. [Low] [bug] The undo history merges actions that should be separate steps
- **Where:** `ShotAnnotate.tsx:638`, `735`, `976`; `ShotViewer.tsx:285-288`, `682`.
- **What's wrong:**
  - Gesture ids come from `gestures.current`, which restarts at 0 whenever AnnotateLayer remounts (draw → crop → Cancel → draw). `lastGesture` in ShotViewer survives the remount, so the first drag afterwards (`drag-1`) merges with the last drag before (`drag-1`), and one Undo reverts both.
  - `restyle` uses the key `style-${id}-${keys}`, which is the same for separate clicks. Red → blue → green on one mark is a single undo step.
- **Fix:** Use a module-level or `useId`-based counter, reset `lastGesture` on mode change, and give restyle a per-interaction token (pointerdown or slider-start).

### 18. [Low] [bug] Temp-file naming and cleanup
- **Where:** `commands.rs:633` `target.with_extension(format!("{}.new.png", std::process::id()))`; `shot.rs:353` fixed `shot.json.part`.
- **What's wrong:** Two concurrent saves of the same clip in one process write the same temp files. A crash between write and rename leaves `Name.1234.new.png` in the user's Screenshots folder, and nothing ever cleans it up.
- **Fix:** Use uuid or atomic-counter suffixes plus a startup sweep of `*.new.png` and `*.part`. The per-clip lock from 3 also fixes the collision.

### 19. [Low] [quality / latent bug] Hooks after an early return in ShotViewer
- **Where:** `ShotViewer.tsx:421` `if (!clip) return null;` is followed by `useEffect` (453) and `useRef` (462).
- **What's wrong:** If `clip` ever went from defined to undefined, React would throw "Rendered fewer hooks than expected". It can't happen today: Clips.tsx always passes `min(open, len-1)` with `len > 0`. The early return is effectively dead, and rules-of-hooks lint fails.
- **Fix:** Move those hooks above the guard.

### 20. [Low] [bug] Hit-test margin uses `size` for blur, which is a radius, not a stroke
- **Where:** `ShotAnnotate.tsx:372` `const margin = shape.tool === "text" ? slack : Math.max(slack, shape.size);`.
- **What's wrong:** Blur sizes run up to `base*12` (96 px at 4K, 192 px at 8K). An invisible halo that wide lets a recent blur steal Select clicks meant for nearby marks.
- **Fix:** Use `slack` for blur (and text).

### 21. [Low] [bug] Smaller preview-vs-file mismatches (likely)
- **Watermark upscaling:** `watermark.rs:179-205` `scale()` assumes downscaling. For frames taller than about 2171 px (152/0.07 — e.g. 5K, 8K, or tall crops) it degenerates to nearest/box sampling and the mark comes out blocky, while the `<img>` preview is smooth. Fix: bilinear when `step < 1`.
- **Blur edges:** the backdrop-filter clips and mirrors at the patch edge; Rust reads `radius` pixels beyond it. A saved blur cut by a crop also reads black stage in the preview but real pixels in Rust.
- **Caption field vs canvas:** the field uses `lineHeight: 1.25` (CSS half-leading); the canvas uses `textBaseline = "top"` (em-box top). Text can jump a few pixels vertically on commit (`ShotAnnotate.tsx:248-258` vs `1019-1020`).

### 22. [Low] [security] Clipboard text read is unbounded by the block size
- **Where:** `clipboard.rs:158-175`.
- **What's wrong:**
  - `read_text` scans up to 4,000,000 `u16` after `GlobalLock` without `GlobalSize`. A non-NUL-terminated `CF_UNICODETEXT` from another app reads up to 8 MB past the block, which can be an access violation.
  - Fix: `let max = GlobalSize(block) / 2;` and loop to `max`.
- **Separately (speculative):** `OpenClipboard(HWND::default())`. MSDN says that with a NULL owner, `EmptyClipboard` sets the owner to NULL and `SetClipboardData` "fails". It works in practice, but it's unsupported; pass the main window's HWND.
- **Checked and correct:** the `unsafe` in `put` (frees the handle on lock or Set failure, size ≥ len), the `DROPFILES` layout, and the DIB stride and padding.

### 23. [Low] [bug] Thumbnail filter for extreme crops
- **Where:** `thumbs.rs:59` `-vf scale=480:-1`.
- **What's wrong:** A 16×4000 crop asks for 480×120000. That exceeds the JPEG limit, so ffmpeg fails and finding 10 applies; tiny crops get upscaled.
- **Fix:** `scale='min(480,iw)':'min(480,ih)':force_original_aspect_ratio=decrease`.

### 24. [Low] [bug] Out-of-range rectangles become a 1-pixel strip instead of an error
- **Where:** `shot.rs:48-57`, `59-63`, `148-151` (`x.min(self.width.saturating_sub(1))`).
- **What's wrong:** A rectangle entirely right of or below the picture is turned into a 1-px strip at the edge (`clamped` and `crop` succeed; `blur` writes one column) instead of being rejected. The UI never sends one; a buggy or compromised caller could.
- **Fix:** `if area.x >= self.width || area.y >= self.height { return Err(..) }`.

### 25. [Low] [quality] Dead code, merged docs, duplicated logic, missing tests
- **Dead IPC call:** `ipc.ts:150-151` `screenshotHasOriginal` invokes `"screenshot_has_original"`, which isn't registered in `lib.rs`.
- **Merged doc comments:** `crop`'s doc ("Cut a rectangle out of the picture…") is merged into `clamped`'s doc (`shot.rs:34-47`), and `crop` has none.
- **Duplicated clamping:** `clamped` and `crop` repeat the same logic; `crop(Rect)` should take the already-clamped rect.
- **Duplicated constants:** `MAX_ZOOM_STEP = 8` (ShotViewer.tsx:60) duplicates `MAX_ZOOM = 8` (pictureBox.ts:78). The watermark asset size 535×152 is hard-coded in TS (`watermark.ts:142-143`); a regenerated asset would silently break the preview.
- **Double-encoded marks:** `shot.json` stores the marks as JSON-in-JSON (`Edit.marks: String`); `serde_json::value::RawValue` would avoid that. Point coordinates are saved at full float precision (bloat).
- **No frontend tests:** there are none (no test files or runner). Add vitest coverage for `markRect` vs fixtures from the Rust `rect_kept` tests, `fitRatio`/`spanned`, `toSteps`, `shift`/`readMarks` round-trip, `sigmaOf`, and the move-clamp in `onMove`.

### 26. [Low] [quality] ShotViewer.tsx is 1650 lines
- **What's wrong:** It holds the viewer, load/save state machine, undo history, crop layer, side panel, Disclosure/PanelButton and `layerBytes`. Several bugs above (4, 5, 6) come from that state being spread across closures.
- **Fix:** Extract `useShotEdit(id)` (load, save, race guards, busy, error), `useUndoHistory`, `CropLayer.tsx`, `AnnotatePanel.tsx`, and a `layers.ts` (render and encode).

### 27. [Low] [perf] Avoidable work per save and per mode switch
- **Per save:** `base.png` is re-encoded on every marks-only save, even when the crop is unchanged (skip if `read_edit(id).crop == crop && base exists`). An ffmpeg process is spawned per save for the thumbnail (see 10). The Reset path re-encodes the original instead of copying it.
- **Mode switches:** view, draw and crop each decode a different full-res PNG in the WebView.
- **Pen strokes:** they copy the whole points array per sample (O(n²)) and miss `getCoalescedEvents()` (angular fast strokes). `bounds()` uses `Math.min(...xs)` spreads, which can hit V8's argument limit on extremely long strokes (speculative).

### 28. [Low] [bug] Slider steps from a fractional minimum
- **Where:** `Controls.tsx:95` with `range()` min `Math.max(1, base*0.4)` (ShotAnnotate.tsx:127).
- **What's wrong:** With `step = 1` and min 0.8 or 3.2, values snap to 0.8, 1.8 … The knob shows a value different from the default size until it is touched.
- **Fix:** Pass `step = min` or round `min`.

## Priority suggestions
1. Blur kernel (1).
2. Commit order and atomic writes (2).
3. Per-clip lock and disabling close/delete/draw while busy (3, 6).
4. Gate the editor on a loaded `edit` and guard the async continuation by id (4, 5).
5. PNG size caps (7).
6. Raw-binary IPC for layers (8).
7. Display-resolution preview canvases (9).
8. In-process thumbnails with failure cleanup (10).
