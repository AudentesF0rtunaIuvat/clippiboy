# Screenshot editor v2: incremental plan, quick wins first

**The plan:** six small releases, each shippable on its own.
1. Safety and integrity defects first, because they leak data or damage files without the user noticing.
2. Then cheap UX gains, then performance, then sharing, then new marks.

There is no rewrite: every task changes a function that already exists. Line numbers refer to 0.8.3 (`417f434`).

## 1. Goals, non-goals, success criteria

**Goals**
- For redaction, the file is never blurred less than the preview shows.
- A save never loses marks, and never leaves the PNG and its sidecars disagreeing.
- Keyboard-first editing: redo, tool hotkeys, Ctrl+S, Ctrl+C, Shift to constrain.
- Smooth on 4K and 8K. Saves send bytes, not JSON.
- Share from inside the editor: copy, export, fit under 10 MB.

**Non-goals (and why)**
- **Single WebView renderer (report 2, option A).**
  - It touches every save path and needs untainted pixels, which the CSP `default-src 'self'` (tauri.conf.json:31) blocks.
  - A regression there would hit every user's files.
  - Revisit when a second pixel-reading tool or "beautify" is approved; M0–M3 leave the code ready for it.
- **Rust renderer (option B).** Text in the file would stop matching the field the user typed into (ShotAnnotate.tsx:989-1033).
- **Re-encoding the JSON-in-JSON `Edit.marks`** (shot.rs:319). It brings users nothing and adds migration risk. Only a `version` gate is added.
- **ts-rs/specta, or splitting `ShotViewer.tsx` on its own.** A new build dependency for three types; extraction happens only where a milestone touches code.
- **Report 2 tiers 2–3** (pixelate, magnifier, rotate, layers panel, multi-select, OCR, beautify, one crop+draw session). Pixelate would add the kind of preview/file pair that M0 removes.

**Success criteria**
1. **Redaction.** For blur radii 2–24 and stripe periods 3–48 px, the file keeps at most Gaussian-preview contrast + 1.5 pp (sketch A's test).
   - Today a single box keeps up to 22 % where the preview keeps about 3.5 %. The 3-pass simulation's worst case is +0.78 pp.
   - Redact leaves exactly the fill colour.
2. **Integrity.** A failure injected at any write point (staged PNG, staged `base.png`, staged `shot.json`, main replace) leaves the PNG, `base.png`, `shot.json` and `image.png` describing one state.
   - No code path resets marks while input is enabled.
3. **4K performance** (3840×2160, 20 marks including 5 blurs, 4-core iGPU laptop):
   - p95 drag frame ≤ 16.7 ms.
   - Preview canvas memory ≤ 4·stage px·dpr² bytes, whatever the mark count (today 33 MB per run).
   - IPC bytes ≤ 1.01× the layer PNG bytes (today about 3.6×).
   - Save at p50 ≤ 800 ms, against a baseline measured in M3.
4. **8K safety.** Peak Rust memory per 7680×4320 save ≤ 450 MB. A PNG header claiming more than 16384 px a side or more than 2²⁷ px is refused before allocating.
5. **Sharing.** Edited shot to clipboard in two keystrokes, with a toast. "Copy for Discord" is ≤ 10 MiB for any 4K or 8K shot.

## 2. Milestones (≈ 28 developer-days)

### M0: Redaction you can trust (0.8.4 hotfix, 3 d)
**What users get:** the file is as unreadable as the preview, a **Redact** tool, and "Re-apply blur" on old blurs.

**Tasks**
- **`Shot::blur`** (shot.rs:139-210): 3 box passes per axis (sketch A).
  - The margin becomes the sum of the pass radii.
  - A rectangle starting outside the picture is skipped, instead of being clamped to a 1-px edge strip (148-151).
  - Fix the doc comment (129-134).
- **`sigmaOf`** (ShotAnnotate.tsx:1079-1081) stays as it is, so saved marks preview unchanged.
- **Blur hint** (ShotViewer.tsx:1107) becomes "Blur hides shapes. For passwords, names and codes use Redact."
- **Redact.**
  - Add a `TOOLS` entry (ShotAnnotate.tsx:16-24). `onDown` (746-761) creates `{tool:"box", fill:true, outline:false, color:"#08080a", redact:true}`.
  - In ShotViewer, `active` (652) maps `chosen.redact` to `"redact"`, which gets a colour control only.
  - It paints through the existing box path (190-198), so 0.8.3 renders it the same and a downgrade cannot unredact.
- **Version gate.**
  - `Edit` (shot.rs:311-320) gets `#[serde(default)] version: u32`, written as `EDIT_VERSION = 2`.
  - `ShotEdit` (commands.rs:426-444) gets `version` and `readOnly: Option<String>`, set when the version is newer.
  - The UI disables the tools and shows the reason. Every later milestone relies on this gate.
- **Legacy-blur notice.** If `version < 2` and any mark is a blur, the panel shows a line and a "Re-apply blur" button that runs `write(edit.crop, shapes)`.
- **Bomb caps.**
  - `read_png` (455-472) and `read_rgba` (676-689) check the size after `read_info` and use `try_reserve_exact`.
  - `read_rgba(bytes, expect)` rejects a wrong size before decoding. Today `blend` checks it only after allocating (105-108).
- **`clipboard.rs::read_text`** (167): bound the scan by `GlobalSize(block)/2`.

**IPC changes:** `Edit.version`, `ShotEdit.version`/`readOnly`, `Shape.redact?`.

**Tests:** the stripe test (sketch A); `pass_radii` variance ≥ the preview's for r = 1..=200; the existing margin tests (shot.rs:765-797); `check_frame(60000, 60000)` returns an error; a blur beside the picture writes nothing.

**Risks:** old files keep the weak blur until saved again (open question 2). Blur CPU roughly triples but stays O(n).

### M1: Saves that cannot lose or corrupt (0.8.5, 5.5 d)
**What users get:** errors as toasts; a locked editor while saving; safe close, paging and delete mid-save.

**UI tasks** (ShotViewer.tsx, sketch C)
- **Gate on the load.**
  - Tools and presets (956, 1129) wait for a loaded, non-`readOnly` `edit`.
  - The load error, swallowed at 356 today, is shown.
  - `write` refuses to run without `edit`.
- **Tie saves to their picture.** A `shownId` ref guards the continuations after each `await`. Hooks at 453 and 462 move above the early return at 421.
- **Freeze while busy.**
  - `AnnotateLayer` and `CropLayer` get `disabled`, and `onKey` (374) returns early.
  - Close (749-758), Delete (1223-1230), Discard (1089) and the step buttons (1249, 1259) are disabled.
- **✕ uses `requestClose()`.** Today it calls `close` directly (751) and skips the unsaved check.
- **Show save errors.** The catch at 639-641 sets `saveError`. `store.writeScreenshot` (store.ts:598-609) returns the result.

**Core tasks**
- **`write_screenshot` returns `ShotSaved {clip, edit}`.** `screenshot_edit` (447-482) becomes `fn shot_edit(&Clip)`, which removes the reload at ShotViewer.tsx:632.
- **Staged commit** (sketch B). It replaces the write order at commands.rs:589-598 and the pid-based temp name at 633.
- **Atomic writes.** `keep_original` (606-620) and `write_under` (shot.rs:385-400) write `.part` files and rename them.
- **`shot::Saving` in-flight set.**
  - `file_clip` (745) skips a screenshot mid-save; `filing::tidy` catches it up later, as the comment at 740-743 already describes.
  - `delete_clip` (676) refuses with "Still saving…".
- **Blocking work moves off the async workers.** `write_screenshot` and `copy_clip_image` become `async fn` + `spawn_blocking`.
- **Errors reach the user.**
  - On `Err`, `crate::notify(&app, "error", …)` (lib.rs:57) emits the error, which Toasts.tsx:44 shows.
  - A copy confirms with "Picture copied"; clipMenu.tsx:170 is silent today.
- **Thumbnails.**
  - If the rebuild fails, delete `thumbs/<id>.jpg` and NULL it via the existing `Library::set_thumb` (clips.rs:253). Today the unredacted thumbnail survives (commands.rs:641-643).
  - Change thumbs.rs:59 to `scale='min(480,iw)':'min(480,ih)':force_original_aspect_ratio=decrease`.
- **Real sizes.** With no original, `screenshot_edit` (457-461) uses `size_of_png(clip.path)`. A file that is not a PNG becomes `readOnly`.
- **Dead code.** Delete `screenshotHasOriginal` (ipc.ts:150-151).

**IPC and sidecar changes:** the `ShotSaved` return; `shot.json` gains `written: {bytes, modifiedMs}`.

**Tests (Rust):** an injected rename failure leaves the old `base.png`/`shot.json` byte-identical with no parts left; a delete between render and commit returns `Err` and creates no file; a failed `keep_original` leaves no `image.png`.

**Manual check:** lock the PNG with PowerShell `[IO.File]::Open(p,'Open','Read','None')` and save. A toast appears and the old state is intact.

**Risks:** a rename runs under the library mutex, but retries sleep outside it. A favorited shot that is saved and then closed is filed at the next start.

### M2: Editor quick wins (0.9.0, 5 d)
**What users get:** redo, hotkeys, Shift-constrain, remembered styles, Copy and "Save & copy", and shortcuts that keep working.

**Tasks**
- **Keys that keep working.**
  - `onKey` skips every `INPUT`, sliders included (378); it should skip only text-entry fields.
  - `AnnotateLayer.onDown` blurs the focused control. Its `preventDefault` (718) currently leaves the slider focused.
- **Hotkeys.**
  - V/A/R/E/P/T/B/X/C pick the tools.
  - Ctrl+S saves. lockdown.ts:39 only cancels the browser default, so the editor still receives the key.
  - Ctrl+C copies; Ctrl+0 fits; Ctrl+1 shows 1:1.
  - Escape deselects, then picks Select, then leaves. Today it jumps straight to the discard question (385).
  - Arrows nudge by 1 px, Shift+arrows by 10 px.
- **Redo.**
  - A `future` stack beside `history` (256), on Ctrl+Y and Ctrl+Shift+Z.
  - History moves into a pure `src/lib/shotHistory.ts`.
  - A module-level gesture counter: `gestures` (638) restarts on remount today.
  - Each pointerdown gets its own restyle token, so `restyle` (682) stops merging separate clicks.
- **Shift-constrain** in `onMove` (838-841) and `spanned` (1323-1348).
- **Remembered styles.**
  - Restyling the selected mark also updates `styles[tool]` (676-687).
  - Persist `{color, altColor, sizeFactor}` in localStorage, wrapped in try/catch.
- **Panel:** Copy and "Save & copy"; stay in draw after saving (638); Save only when `unsaved` (1065); confirm "Remove all marks" (1095-1104).
- **Small bugs.**
  - Handle `onPointerCancel` and `onLostPointerCapture`, and ignore `buttons===0` (917-925, 1408-1414).
  - Blur hit margin = `slack` (372).
  - Round the slider minimum (127).
  - Plain wheel pans in draw and crop modes (771-779).
- **External edits.** If the file differs from `written`, show a banner "Changed outside ClippiBoy — start from this version". Accepting it calls `forget` (shot.rs:433).
- **Also:** `role="dialog" aria-modal` (709-716), and `Feature::ShotEditor = 6` (telemetry.rs:28-35).

**Tests:** `node --test`, with no new dependency, as in server/package.json:10 (tsconfig already allows `.ts` imports).
- `shotHistory.test.ts`.
- `shotGeometry.test.ts`: constrain, nudge never going negative, the `shift`/`readMarks` round trip.
- Watermark parity: `markRect` moves to an asset-free `src/lib/markRect.ts`. A shared `watermark_rects.json` fixture is checked from both Rust and node.

**Risk:** Ctrl+C must yield to selected text and to the caption field.

### M3: Fast on 4K and 8K (0.9.1, 4.5 d)
**What users get:** smooth drags, faster saves, no memory spikes with many blurs.

**Tasks**
- **Binary IPC.**
  - `write_screenshot` takes a `tauri::ipc::Request` (Tauri 2.11.5, Cargo.lock:5586). The body is a frame sequence: `[u32 len][json][u32 len][png]…`.
  - `Step::Layer { layer: u32 }` refers to a frame by index.
  - This removes the `number[]` path (ShotViewer.tsx:1623, types.ts:527) and serde's `Content` buffering of the tagged `Step` (commands.rs:400-419).
- **`layerBytes`** reuses one canvas and encodes the layers one after another.
- **Stage-sized preview canvases.**
  - `ShapeLayer` (566-586) becomes stage × dpr in size and paints after `setTransform(dpr·scale,…,dpr·left,dpr·top)`.
  - `paint` uses user-space units only, so the result scales exactly. `sameLayer` already repaints when `box` changes.
- **Drag isolation.**
  - During a drag, `layers` (679-682) splits the run into before, the dragged mark, and after. Z-order is unchanged.
  - Pointer moves are coalesced with rAF.
- **Skip `base.png`** when the crop is unchanged and the file exists (commands.rs:589-595).
- **Harness.** A Rust `#[ignore]` 4K timing test, plus a dev-only frame-time meter.

**Risk:** text antialiasing in the preview shifts slightly. It was already CSS-downscaled.

### M4: Ready to share (0.9.2, 5 d)
**What users get:** "Copy for Discord", "Export…" as PNG or JPEG, and pasting into more apps.

**Tasks**
- **Clipboard.** `copy_image` (clipboard.rs:131-135) also places the registered `"PNG"` format, using the file's own bytes.
- **Discord export.**
  - Add the pure-Rust `jpeg-encoder` crate.
  - `export_shot_for_discord` mirrors `export_for_discord` (commands.rs:857-890), which refuses stills at 865.
  - Order of attempts: PNG, then a JPEG quality search from 92 down to 60, then scaling to 75 % and 50 %.
  - `ShotExportDialog` reuses the presets in ExportDialog.tsx:18-22.
- **In-process still thumbnails.**
  - Box-downscale plus JPEG, in the commit and in `take_screenshot` (state.rs:933-939).
  - This takes ffmpeg off the stills path and closes bug 10.
- **Optional "copy after capture"** while the `Shot` is still in memory (state.rs:902-917).

**Tests:** fit-under-10 MiB on noisy 4K/8K shots; thumbnail aspect for a 16×4000 crop. **Risk:** one new crate; no WebP (needs libwebp).

### M5: More marks, no core change (0.9.3, 5 d)
**What users get:** line, double arrow, highlighter, numbered steps, rounded boxes, text background, and marks that stay visible while cropping.

**Tasks**
- **Arrows.** Add `head?: "end"|"both"|"none"`. The shaft stops at the head, so the cap no longer pokes past the tip (214-231).
- **Highlighter.** Add `opacity?`; today it is forced to 1 (181). The "Marker" preset is pen at 0.35.
- **Steps.** A `step` tool stores `n = max+1` at creation, never inside `paint`, because `paint` runs per layer.
- **Shapes and text.** `radius?` via `ctx.roundRect`; `bg?` for text.
- **Crop mode.** It mounts read-only shifted layers; today they mount only in draw mode (880-892).
- **Versioning.** The UI sends `version: 3` when new fields are used, so builds since M0 open such files read-only.

**Tests:** `node --test` for `toSteps`, `bounds` and `hit`.

**Risk:** 0.8.3 has no version gate (open question 7).

### Report-1 defects: where each one lands
- **M0:** 1 blur, 7 bomb caps, 22 clipboard read, 24 out-of-range rect.
- **M1:**
  - 2 commit order, 3 save race, 4 load gate, 5 continuation, 6 marks lost while busy.
  - 10 stale thumbnail (part), 11 real sizes, 14 recording the file stamp, 15 `spawn_blocking`, 16 swallowed errors.
  - 18 uuid names, 19 hooks, 23 thumbnail filter, 25 dead IPC.
- **M2:** 12 interrupted drags, 14 the warning, 17 undo merging, 20 hit margin, 21 caption baseline (verify), 25 tests, 28 slider step.
- **M3:** 8 IPC, 9 canvases, 27 `base.png` skip.
- **M4:** 10 completed.
- **Deferred:** 13 (question 4); 18 orphan sweep; 21 watermark upscaling (cosmetic, >2171 px tall only); 26 (per-milestone extraction); the rest of 27; receive-side checks for 7/11 (share.rs) go to the share workstream.

## 3. Code sketches

### A. A 3-pass blur that is never weaker than the preview (shot.rs)

```rust
/// Box half-widths whose variances sum to at least the preview Gaussian's
/// (`sigmaOf`: σ² = r(r+1)/3). One box has variance r(r+1)/3; three boxes are
/// within ~1 % of a Gaussian, while one lets ~22 % of text-stroke contrast through.
fn pass_radii(radius: usize) -> [usize; 3] {
    let target = (radius * (radius + 1)) as f64 / 3.0;
    let var = |r: usize| (r * (r + 1)) as f64 / 3.0;
    let low = ((-1.0 + (1.0 + 4.0 * target).sqrt()) / 2.0).floor() as usize;
    for wide in 0..=3 {
        let radii = [low + usize::from(wide >= 3), low + usize::from(wide >= 2), low + usize::from(wide >= 1)];
        if radii.iter().map(|&r| var(r)).sum::<f64>() + 1e-9 >= target {
            return radii;
        }
    }
    [low + 1; 3]
}

pub fn blur(&mut self, x: u32, y: u32, width: u32, height: u32, radius: u32, ellipse: bool) {
    if x >= self.width || y >= self.height {
        return; // beside the picture: nothing, not a 1-px strip
    }
    let passes = pass_radii(radius.max(1) as usize);
    let reach: usize = passes.iter().sum(); // three passes read three times as far
    // ... `left/top/w/h`, the read area and `done` (the copied patch) exactly as
    //     today (shot.rs:148-176), with `reach` in place of `radius` ...
    let mut spare = vec![0u8; done.len()];
    let run = |data: &mut Vec<u8>, spare: &mut Vec<u8>, width: usize, height: usize| {
        for &r in passes.iter().filter(|&&r| r > 0) {
            blur_rows(data, spare, width, height, r);
            std::mem::swap(data, spare);
        }
    };
    run(&mut done, &mut spare, pw, ph);
    transpose(&done, &mut spare, pw, ph);
    std::mem::swap(&mut done, &mut spare);
    run(&mut done, &mut spare, ph, pw);
    transpose(&done, &mut spare, ph, pw);
    let across = spare;
    // ... write-back with the oval mask unchanged (shot.rs:191-209) ...
}

#[test]
fn the_file_blurs_text_strokes_at_least_as_hard_as_the_preview() {
    use std::f64::consts::PI;
    for radius in [2u32, 4, 8, 16, 24] {
        let sigma2 = (radius * (radius + 1)) as f64 / 3.0;
        for period in [3usize, 4, 6, 8, 12, 16, 24, 32, 48] {
            let (w, h) = (period * 40, 8usize);
            let rgb = (0..w * h)
                .flat_map(|i| {
                    let v = 127.5 + 127.5 * (2.0 * PI * (i % w) as f64 / period as f64).cos();
                    [v.round() as u8; 3]
                })
                .collect();
            let mut shot = Shot { width: w as u32, height: h as u32, rgb };
            shot.blur(0, 0, w as u32, h as u32, radius, false);
            let row = &shot.rgb[(h / 2) * w * 3..(h / 2 + 1) * w * 3];
            let mid: Vec<f64> = row.chunks_exact(3).skip(w / 4).take(w / 2).map(|p| p[0] as f64).collect();
            let kept = (mid.iter().cloned().fold(0.0, f64::max) - mid.iter().cloned().fold(255.0, f64::min)) / 255.0;
            let gaussian = (-2.0 * PI * PI * sigma2 / (period * period) as f64).exp();
            assert!(kept <= gaussian + 0.015 + 2.0 / 255.0, "r={radius} P={period}: {kept:.3} > {gaussian:.3}");
        }
    }
}
```

I simulated this exact integer pipeline (`scratchpad/editor/blursim.py`). The worst excess is 0.0078. For r = 8 the passes are [4, 5, 5], with a margin of 14 instead of 8.

### B. Staged commit to the path the library holds now (commands.rs)

```rust
struct Staged { picture: PathBuf, base: Option<PathBuf>, note: PathBuf }

impl Staged {
    fn write(clip: &Clip, ground: Option<&Shot>, edit: &Edit, picture: &Shot) -> Result<Staged> {
        let token = uuid::Uuid::new_v4().simple().to_string();
        let staged = Staged {
            picture: PathBuf::from(&clip.path).with_extension(format!("{token}.new.png")),
            base: ground.map(|_| shot::base_path(&clip.id).with_extension(format!("{token}.part"))),
            note: shot::edit_path(&clip.id).with_extension(format!("{token}.part")), // now pub(crate)
        };
        let written = (|| {
            picture.write_png(&staged.picture)?;
            if let (Some(g), Some(path)) = (ground, &staged.base) {
                g.write_png(path)?;
            }
            shot::write_edit_to(&staged.note, edit) // today's write_edit without the rename
        })();
        match written {
            Ok(()) => Ok(staged),
            Err(err) => { staged.discard(); Err(err) }
        }
    }

    fn discard(&self) {
        for path in [Some(&self.picture), self.base.as_ref(), Some(&self.note)].into_iter().flatten() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// The screenshot first: it is what Windows refuses to replace while a viewer
    /// holds it, and a refusal then leaves old picture + old notes, which agree.
    fn commit(self, state: &State<'_, AppState>, id: &str, size: (u32, u32)) -> Result<Clip> {
        let mut last = String::new();
        for attempt in 0..10u64 {
            // Under the library lock, at the path it holds now: `file_clip` moves
            // files under this same lock (commands.rs:747-756), `Library::delete`
            // removes row and file under it (clips.rs:315-323).
            let placed = with_library(state, |lib| {
                let clip = lib.get(id).map_err(|e| e.to_string())?
                    .ok_or("The screenshot was deleted while it was being saved.")?;
                let target = PathBuf::from(&clip.path);
                Ok(match std::fs::rename(&self.picture, &target) {
                    Ok(()) => {
                        let bytes = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                        lib.set_picture(id, size.0, size.1, bytes).map_err(|e| e.to_string())?;
                        Ok(target)
                    }
                    Err(err) => Err(err.to_string()), // retried below, outside the lock
                })
            });
            match placed {
                Ok(Ok(target)) => {
                    match &self.base {
                        Some(part) => crate::muxer::replace_file(part, &shot::base_path(id))?,
                        None => { let _ = std::fs::remove_file(shot::base_path(id)); }
                    }
                    crate::muxer::replace_file(&self.note, &shot::edit_path(id))?;
                    refresh_still(state, id, &target); // on failure: delete thumb, NULL it
                    return with_library(state, |lib| lib.get(id).map_err(|e| e.to_string())?
                        .ok_or_else(|| "clip not found".into()));
                }
                Ok(Err(err)) => {
                    last = err;
                    std::thread::sleep(std::time::Duration::from_millis(50 * (attempt + 1)));
                }
                Err(fatal) => { self.discard(); return Err(fatal); }
            }
        }
        self.discard();
        Err(format!("Could not replace the screenshot ({last}). Is it open somewhere right now?"))
    }
}
```

`edit_screenshot` runs in three steps:
1. It opens with `let _saving = shot::Saving::begin(&id)?;`. This is a `Mutex<BTreeSet<String>>` guard that clears itself on `Drop`.
2. It renders as it does today (539-587).
3. It ends with `Staged::write(..)?.commit(state, &id, (picture.width, picture.height))`.

### C. Save tied to its picture, editor frozen while busy (ShotViewer.tsx)

```tsx
// Above `if (!clip) return null` (421) — hooks must not sit after it.
const shownId = useRef(id);
shownId.current = id;
const [saveError, setSaveError] = useState<string | null>(null);
/** Without the store, `full` and `origin` are guesses (bug 4). */
const ready = inTauri && !broken && !busy && edit !== null && !edit.readOnly;

const write = async (crop: Rect | null, marks: Shape[]) => {
  if (!edit || busy) return;
  const forId = clip.id;
  setBusy(true);
  setSaveError(null);
  try {
    const placed = shift(marks, origin.x, origin.y);
    const steps: ShotStep[] = [];
    for (const step of toSteps(placed)) {
      // One full-size canvas at a time, not one per layer at once.
      steps.push(step.kind === "blur"
        ? step
        : { kind: "layer", png: await layerBytes(step.shapes, full.width, full.height) });
    }
    const saved = await writeScreenshot(
      forId,
      crop && roundRect(crop), // today's Math.round block, 621-626
      steps,
      placed.length ? JSON.stringify(placed) : "",
    );
    if (shownId.current !== forId) return; // paged on or closed meanwhile (bug 5)
    setEdit(saved.edit);
    resetShapes(readMarks(saved.edit));
    setUnsaved(false);
    setSel(null);
    setMode((was) => (was === "crop" ? "view" : was)); // M2: stay in draw
  } catch (err) {
    if (shownId.current === forId) setSaveError(String(err));
  } finally {
    setBusy(false);
  }
};

const requestClose = () => (editing && unsaved ? setAskingDiscard(true) : close());
// ✕ onClick={requestClose}; <AnnotateLayer disabled={!ready}> (pointer-events none);
// tools disabled={!ready}; Delete/Discard/‹ › disabled={busy || …}; onKey: if (busy) return;
```

## 4. Open questions for the maintainer

1. **Blur strength.** M0 makes the file match the preview σ. Should it also have a floor (for example, radius ≥ height/4)? Should rectangles under about 40 px default to Redact?
2. **Older weak blurs.** Is the per-picture notice enough? The alternative is a Settings button, "Strengthen blur on N screenshots": a UI loop that takes about 1 s per file and rewrites user files.
3. **Watermark.** It is re-stamped whenever `under.png` exists, even after the user switches the watermark off (commands.rs:585-587). Should the setting be honoured?
4. **Unredacted originals.** They stay in `originals/<id>/image.png` (shot.rs:287-289) forever. Add "Flatten — discard history" in M4 or M5?
5. **Test runner:** `node --test` (no dependency, like the server) or vitest?
6. **Order:** swap M4 and M5 if requests for marks outnumber Discord-size complaints.
7. **Downgrades:** may 0.8.3 drop version-3 marks, or should M5 wait a release after M0?
8. **Benchmarks:** which reference machine counts for criterion 3?
9. **Filing during a save.** Should `file_clip` skip a screenshot mid-save (as above), or should Clips.tsx:327-336 queue `fileClip` until the save finishes? Queuing costs about 0.5 d more.
