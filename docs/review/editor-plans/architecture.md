# Screenshot editor v2: architecture-first plan

Grounded in ClippiBoy 0.8.3 (`417f434`) and in reports 01 (bugs) and 02 (features). `file:line` always refers to today's code.

## 0. Today's foundation

- **Two rasterisers per pixel op.**
  - Vector marks are painted in the WebView by `paint` (ShotAnnotate.tsx:173-270).
  - The blur preview is a CSS `backdrop-filter` (ShotAnnotate.tsx:1045-1067).
  - The file blur is one box pass in Rust (shot.rs:178-185).
- **The model is opaque to the core.**
  - `Edit { crop, marks: String }` has no version (shot.rs:311-320).
  - A parse failure becomes "no edit" (shot.rs:335-340).
  - Old marks are recognised by missing fields (ShotAnnotate.tsx:192-194).
- **Preview memory grows with blur count.** There is one full-resolution canvas per run of marks (ShotAnnotate.tsx:566-586), and N more are created per save (ShotViewer.tsx:609-618, 1607-1624).
- **Layer PNGs travel as JSON `number[]`** (ShotViewer.tsx:1623, commands.rs:400-419).
- **The commit is not crash-safe.**
  - `base.png` is truncated in place (shot.rs:257).
  - Then `shot.json` is written, then the picture (commands.rs:589-598).
  - `keep_original` uses a plain `fs::copy` (commands.rs:617).
- **No lock covers a save.** The clip is read once (commands.rs:518). `file_clip` moves files under the library lock only (commands.rs:747-765). The clip editor has `WORKING` (edit.rs:53).
- **The UI opens before its document arrives.** The load error is swallowed (ShotViewer.tsx:356). Tools are enabled without `edit` (956). After a save, the reload overwrites newer marks (632-638).
- **The watermark is stamped by Rust** (commands.rs:585-587). Its rectangle logic is copied by hand into TS (watermark.ts:6-14).
- **"Inter" is an `@font-face` alias to local fonts** (index.css:5-11). Canvas text relies on it (ShotAnnotate.tsx:284-286).
- **No TS test runner and no CI test job.** package.json has no test script; `.github/workflows` has only release, pages and ffmpeg.

## 1. Goals, non-goals, success criteria

**Goals:**
- One rasteriser for both preview and file.
- A Rust-owned, versioned, typed document that is migrated, validated and never silently dropped.
- A crash-safe commit, serialised per clip and respected by moves and deletes.
- Binary IPC.
- Deterministic redaction, pinned by tests.
- Every existing screenshot still opens, renders and re-saves, and 0.8.x can read v2 sidecars.

**Non-goals:**
- Tools beyond Redact and Pixelate.
- OCR or beautify.
- A Rust or WASM renderer.
- Editing non-screenshots.
- Changing the watermark rules (Q4).

**Success criteria** (baselined by the M1 perf harness):
1. **Redaction.**
   - The file kernel is a 3-pass box. Its largest frequency side-lobe is at most 2% for every radius in `range("blur")` (ShotAnnotate.tsx:124-125); today it is about 22%.
   - The preview radius is never larger than the file's at any zoom.
   - After any commit, failed or not, no thumbnail older than the committed pixels survives.
2. **Parity.**
   - The committed PNG equals the export render bit-for-bit outside the watermark rectangle.
   - TS `blur3` equals Rust `blur3` bit-for-bit.
   - Preview and export at scale 1 differ in at most 0.5% of pixels by more than 4/255.
3. **Interaction.** Test scene: 3840×2160, 50 marks, 10 of them blurs.
   - Mark drag p95 is at most 16.7 ms.
   - The side panel does not re-render during the drag.
   - The editor adds at most 150 MB to WebView memory. Today that scene holds up to 11 canvases of 33 MB each.
4. **Save.**
   - 4K: at most 900 ms p95 from click to new document. 8K: at most 3 s.
   - No main-thread task runs longer than 50 ms.
   - Kill the process at every commit step: `recover()` always ends with `shot.json.output` matching the file, and `image.png` intact.
5. **Compatibility.**
   - Every v1 fixture opens. Valid ones are editable; invalid ones open read-only and are never overwritten.
   - A v2 file parsed with 0.8.3's `Edit` yields the same crop and marks.

## 2. ADR-1: one rasteriser

| Criterion | **A: WebView scene renderer** (worker, OffscreenCanvas) | B: Rust tiny-skia | C: contained hybrid |
|---|---|---|---|
| Preview equals file | By construction: the committed pixels are the render | Only via WASM or a round trip per change | Parity tests per op, forever |
| Redaction determinism | Integer typed-array kernels, bit-pinned to Rust fixtures | Best | Rust side only; the preview stays CSS |
| Text matches the typing field (ShotAnnotate.tsx:1011-1031) | Yes, same Chromium text stack | No: needs bundled fonts and shaping | Yes |
| 60 fps drag | Display-resolution canvas, off the main thread | Needs WASM | Vectors yes, blur via CSS |
| Cost of a new pixel tool | 1 implementation | 1 implementation plus WASM | 2 implementations plus parity |
| Headless use | Needs the hidden WebView | Yes | Partial |

**Decision: A.**
- The WebView rasterises the preview into one display-resolution canvas and the export into one full-resolution canvas, both inside a render worker.
- Rust keeps authority over:
  - source bytes and size caps;
  - the document schema and its migration;
  - the watermark (`stamp_kept`);
  - per-clip locks and the crash-safe commit;
  - thumbnails and the clipboard.
- C ships first as M0, as a stop-gap. B is rejected unless headless rendering becomes a requirement; the v2 scene format allows adding it later without migration.

**Consequences:**
- Pixel ops are pure TS over `Uint8ClampedArray`. The composite is opaque, so canvas premultiplication is lossless.
- `Shot::blend` and `Shot::blur` are retired in M3; Rust `blur3` stays as the test reference.
- Each save sends one RGBA frame over IPC (33 MB at 4K).

**IPC:**
- `screenshot_open(id) -> ShotOpen` (JSON): the document, its state, the original's dimensions read from the PNG header, `under`, and `rev`.
- `screenshot_source(id, part) -> tauri::ipc::Response`: raw `image.png` or `under.png` bytes, decoded with `createImageBitmap(blob)` into an origin-clean bitmap. Today's grounds are asset-protocol `<img>` elements (ShotViewer.tsx:449, ipc.ts:31-34), which may taint a canvas.
- `commit_screenshot(tauri::ipc::Request)`: a raw body of `"CBS2"`, a u32 length, the header JSON, then RGBA. It returns the new `ShotOpen`, so the second load at ShotViewer.tsx:632 goes away.

## 3. Milestones

### M0: "Stop the bleeding" (0.8.4), 4 dev-days
**Outcome:**
- The file blur is at least as strong as its preview.
- Saving cannot misplace, lose or race work.
- Hostile PNGs no longer abort the app.
- Errors are visible.

**Tasks:**
1. **Blur.**
   - `Shot::blur` (shot.rs:139-210) runs (rows, columns) × 3 passes of `blur_rows`, with `r_i = boxRadii(r)` and a margin of `3·r_i`.
   - Fix the doc comment (123-134).
   - Add `#[serde(default)] blur_passes: u8` to `Edit` (313).
   - `sigmaOf` (ShotAnnotate.tsx:1079) is unchanged because the variance is kept.
2. **Size caps.** `read_png` (shot.rs:455-506) and `read_rgba` (676-707) reject `w·h > 2^27` or a side over 16384 after `read_info`, then use `try_reserve_exact`. `read_rgba` takes the expected `(w,h)`, so `blend` (104-108) rejects before allocating.
3. **Write order.**
   - Write `base.png.part`, `shot.json.part` and a uuid-named temp picture.
   - `replace_file` the picture, then rename the parts (commands.rs:589-598, 633).
   - `keep_original` copies through a `.part` file (606-620).
4. **Thumbnail.** When `make_still` fails (commands.rs:641-643), delete `thumbs/<id>.jpg` and set `thumb_path = NULL` through a new `Library::set_thumb` (beside clips.rs:290).
5. **ShotViewer.tsx.**
   - Gate the tools, presets and Save on `edit` (956, 1129, 1065).
   - Show the load error (356).
   - Keep a `shownId` ref and check it after every await in `write` (604-644).
   - Pass `busy` to both layers.
   - Disable ✕, Delete and Discard while busy (749-758, 1223-1230, 1089).
   - Route ✕ through the unsaved check.
   - The `catch` at 639-641 shows the error.
6. **ShotAnnotate.tsx.** Handle `onPointerCancel` and `onLostPointerCapture`, and ignore moves with `buttons === 0` (917-925, 764). The blur hit margin becomes `slack` (372).

**Tests (Rust):**
- `blur3_equals_naive_triple_box`
- `blur3_sidelobes_below_2pct` (DFT of the impulse response)
- `read_png_rejects_huge_ihdr` (a hand-built 60000² IHDR)
- `failed_replace_keeps_old_sidecar`

**Risk:** blur costs 3× (still O(n); about 10 ms for a 1000×300 region at 4K).

### M1: "Typed document, locks, safe commit", 8 dev-days
**Outcome:**
- A crash, a full disk or a viewer holding the file cannot desynchronise the picture and its history.
- Closing or deleting right after Save is safe.
- Paint edits are detected instead of overwritten.
- Screenshots saved with the old weak blur can be re-redacted (opt-in).

**Tasks:**
1. **`src-tauri/src/shot_doc.rs`** (sketch 1).
   - Typed v2 `ShotDoc`/`Mark` that serialise to TS through `cfg_attr(test, derive(ts_rs::TS))` into `src/lib/generated/`.
   - v1→v2 migration, validation limits, and an `Opened` state.
   - Replaces shot.rs:306-356.
   - Out-of-range rectangles become errors (01#24).
2. **`src-tauri/src/shot_commit.rs`** (sketch 2).
   - `ClipLocks` in `AppState` (state.rs:18/318).
   - `stage`, `publish` and `recover`; `recover_all()` runs in `setup` (lib.rs:1189).
   - `Shot::encode_png` (from 251-279), so bytes are fingerprinted before they touch the disk.
3. **commands.rs.**
   - `screenshot_open` replaces `screenshot_edit` (446-482). It takes its dimensions from `size_of_png`, not the database (01#11, 457-461). It keeps `basePath` until M2.
   - `write_screenshot` keeps its JSON `steps` for now but gains `baseRev` and typed marks, and commits through `stage`/`publish` under the clip lock, re-reading `clip.path` there.
   - `file_clip` (745-769) and `delete_clip` (676-705) take the lock. They become async with `spawn_blocking`, so the main thread never waits.
   - Blocking bodies move into `spawn_blocking` (01#15, 389-392, 659-668).
4. **Concurrency and external edits.**
   - A `baseRev` mismatch returns "changed elsewhere — reopen".
   - An `output` fingerprint mismatch (bytes, then mtime, then sha256) puts the document in `ChangedOutside`, offering "Use this file as the original" (new `adopt_screenshot_file`) or "Overwrite" (01#14).
5. **Thumbnails.** `thumbs::still_from(&Shot, id)` replaces ffmpeg for stills (thumbs.rs:51-64; 01#10, #23): a box downscale to ≤480 px encoded with `jpeg-encoder`. It is used by `write_picture` and `take_screenshot` (state.rs:933).
6. **Legacy blurs.** `legacy_blur_shots()` returns ids with blur marks and `renderer == RustBox1`. A one-time gallery card re-commits them.
7. **TS.**
   - `src/editor/useShotDoc.ts` takes over load, error, busy and rev guard from ShotViewer.tsx:326-363 and 604-644.
   - `toShape`/`toMark` adapters convert at the boundary.
   - Delete `screenshotHasOriginal` (ipc.ts:150-151).
8. **Infrastructure.**
   - vitest.
   - `ci.yml` on Ubuntu: `cargo test --lib`, `tsc`, vitest, and `git diff --exit-code src/lib/generated`.
   - `npm run perf:editor`.
   - Bilinear watermark upscaling in `scale()` (watermark.rs:179-205).

**Tests:**
- Fixtures in `src-tauri/tests/fixtures/shot/`, each mapped to an expected v2 document or read-only state: 0.8.0 marks; 0.8.3 blur and ellipse; an 0.8.1 `under.json` without `picture`; garbage; `version: 3`.
- Downgrade: a v2 file parsed by a copy of 0.8.3's `Edit`.
- Kill after stage / replace / rename, then `recover`, then check the invariant.
- A `ClipLocks` thread test showing that `file_clip` waits.
- vitest: a stale continuation is ignored.

**Risks:**
- ts-rs output for `flatten` plus internally tagged enums (review the generated `.ts`).
- Opening costs about 30 ms of sha256, but only on a size or mtime mismatch.

### M2: "One renderer", 11 dev-days
**Outcome:**
- The saved picture is byte-for-byte the render.
- 4K and 8K editing is smooth.
- Marks stay visible while cropping.
- Redo works.
- Saving keeps you in the editor and keeps marks drawn during the save.

**Tasks:**
1. **`src/editor/raster.ts`** (sketch 3). `blur3` is ported from `blur_rows` (shot.rs:633-661), with a feathered oval coverage mask. Rust gets the same `oval_coverage`.
2. **`src/editor/paint.ts`.** Move `paint` (ShotAnnotate.tsx:173-270) here and type it for `OffscreenCanvasRenderingContext2D`. Field and canvas both take their text baseline from `fontBoundingBoxAscent` (01#21).
3. **`src/editor/render.ts`.** `renderScene` draws one canvas with ordered ops. It replaces `toSteps` (491-525), `ShapeLayer` (566-586) and `BlurPatch` (1045-1067).
4. **`src/editor/renderer.worker.ts`.**
   - The worker owns the only full-resolution source; the `screenshot_source` buffer is transferred to it.
   - The preview canvas comes from `transferControlToOffscreen()` at stage × DPR. Its region is the viewport plus the blur margin.
   - A drag caches the prefix `[0,k)`; each frame draws that cache, then mark k, then the suffix.
   - `export` renders crop ∪ blur reach at scale 1 and builds the CBS2 frame.
5. **Rust `commit_screenshot`** (sketch 2).
   - Check length and dimensions against the clamped crop.
   - Convert RGBA→RGB, stamp, stage and publish.
   - Record `Scene1` and delete `base.png`.
6. **`src/editor/sceneStore.ts`** (Zustand).
   - Undo and redo, with gesture tokens from a module counter (01#17; ShotAnnotate.tsx:638, ShotViewer.tsx:682).
   - Marks newer than the save snapshot survive as unsaved (01#6).
   - "Remove all" becomes undoable (ShotViewer.tsx:1095-1104).
7. **Split ShotViewer.tsx** (1649 lines) into `Stage`, `EditorPanel` (memoised) and `CropOverlay`. Hooks move above the guard (01#19; 421 vs 453).
8. **`editor.renderer` flag** (`scene` or `legacy`). Fall back to legacy for one release if the worker fails.

**Tests:**
- vitest: `blur3` against `fixtures/editor/blur/*.rgba`, emitted by `cargo test -- --ignored regen_fixtures`.
- vitest: `markRect` against a `rect_kept`-emitted `watermark_rects.json`.
- Playwright Chromium (software raster): 12 golden scenes, plus main-thread vs worker text parity.
- Rust: bad frames are rejected; the committed PNG decodes back to the sent RGBA outside the mark.

**Risks:**
- The `@font-face` alias does not exist in workers. Register it with `new FontFace("Inter", 'local("Inter"), local("Inter Variable"), local("Segoe UI Variable Display")')` in `self.fonts`.
- `getImageData` readback cost: use `willReadFrequently` only on the blur path.
- 8K means about 264 MB peak; band exports above 33 MP.
- Verify `transferControlToOffscreen` and a 64 MB raw `invoke` in WebView2 on day 1.

### M3: "Retire the duplicate, build on it", 6 dev-days
**Outcome:**
- Redact and Pixelate tools.
- Copy or export without saving.
- "Make permanent" purges the unredacted original (01#13).

**Tasks:**
- Delete `Shot::blend` (shot.rs:104-121), `Step` (commands.rs:400-419), `layerBytes` (ShotViewer.tsx:1607-1624), the legacy flag and `base.png`. `blur3` becomes `#[cfg(test)]`.
- Add `MarkKind::{Redact, Pixelate}`. Pixelate is an integer block average.
- `copy_scene_image`: worker frame → `stamp_kept` → `Shot::dib` → `clipboard::copy_image` (clipboard.rs:131-135).
- `flatten_screenshot`: under the lock, save the clean watermark area as a new `under.png`, then remove `image.png` and `shot.json`.

**Tests:**
- Pixelate goldens; no source pixel survives a block ≥2.
- After flatten, the next commit still re-stamps the mark.

**Risk:** downgrading after M3 loses mark editing unless the v1 shadow is still written (Q1).

### Defect disposition (report 01)

| # | Fixed in |
|---|---|
| 1 blur weaker than preview | M0 3-pass blur; M2 makes it impossible by construction; M1 re-redacts old files (opt-in) |
| 2, 18 commit order and temp files | M0 reorder; M1 journal and `recover` |
| 3 race with move or delete | M0 disables close/delete while busy; M1 `ClipLocks` |
| 4, 5, 16 load gate, stale continuation, swallowed errors | M0 guards; M1 `useShotDoc` and `baseRev` |
| 6 marks lost while saving | M0 input freeze; M2 rebase |
| 7 PNG bomb | M0 |
| 8 `number[]` IPC | M2. Deferred because the layer payload is deleted rather than optimised. |
| 9 canvases and re-renders | M2 |
| 10, 23 stale thumbnail, scale | M0 delete; M1 in-process thumbnail |
| 11 trusted dimensions, JPEG | M1 header dimensions; non-PNG opens read-only. The sender side is deferred to sharing. |
| 12, 20 interrupted drags, blur hit halo | M0 |
| 13 original kept forever | M3 |
| 14 external edits overwritten | M1 |
| 15 blocking on async workers | M1 |
| 17, 19, 26 undo merging, hook order, file size | M2 |
| 21 watermark upscale, blur edges, caption jump | M1 and M2 |
| 24, 25 out-of-range rectangles, dead code, no tests | M1 |
| 22, 28 clipboard read, slider step | Deferred: #22 is outside the editor (separate ticket); #28 is done in M2's panel |

Total: about 29 dev-days.

## 4. Code sketches

### Sketch 1: `shot_doc.rs` — typed v2, migration and downgrade shadow
```rust
use serde::{Deserialize, Serialize};
use crate::shot::Rect;
pub const VERSION: u32 = 2;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, export_to = "../../src/lib/generated/"))]
#[serde(rename_all = "camelCase")]
pub struct Mark {
    pub id: u32,
    pub x1: f64, pub y1: f64, pub x2: f64, pub y2: f64,
    pub size: f64, // line width, text height or blur radius, original px
    #[serde(flatten)]
    pub kind: MarkKind,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS), ts(export, export_to = "../../src/lib/generated/"))]
#[serde(tag = "tool", rename_all = "camelCase")]
pub enum MarkKind {
    Arrow { color: String },
    #[serde(rename_all = "camelCase")]
    Box { color: String, fill: bool, outline: bool, alt_color: Option<String> },
    #[serde(rename_all = "camelCase")]
    Ellipse { color: String, fill: bool, outline: bool, alt_color: Option<String> },
    Pen { color: String, points: Vec<[f64; 2]> },
    #[serde(rename_all = "camelCase")]
    Text { color: String, alt_color: Option<String>, text: String },
    Blur { round: bool },
}

/// shot.json, any version. serde ignores unknown keys — that is what lets
/// 0.8.x (`Edit { crop, marks: String }`) keep opening a v2 file.
#[derive(Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct OnDisk {
    version: u32, // 0 = written by 0.8.x
    rev: u64,
    crop: Option<Rect>,
    marks: String, // v1 payload; v2 keeps writing it as the downgrade shadow
    scene: Option<Vec<Mark>>,
    output: Option<Fingerprint>,
    renderer: Option<Renderer>,
    blur_passes: u8, // 0.8.4
    #[serde(skip_serializing_if = "Option::is_none")]
    pending: Option<String>, // only in a staged .part: its temp picture
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ShapeV1 {
    id: u32, tool: String, #[serde(default)] color: String, size: f64,
    fill: Option<bool>, outline: Option<bool>, round: Option<bool>,
    alt_color: Option<String>, points: Option<Vec<[f64; 2]>>, text: Option<String>,
    x1: f64, y1: f64, x2: f64, y2: f64,
}

impl TryFrom<ShapeV1> for Mark {
    type Error = String;
    fn try_from(s: ShapeV1) -> Result<Self, String> {
        // Missing `outline` = drawn before the choice existed (ShotAnnotate.tsx:192-194).
        let (fill, outline) = (s.fill.unwrap_or(false), s.outline != Some(false));
        let kind = match s.tool.as_str() {
            "arrow" => MarkKind::Arrow { color: s.color },
            "box" => MarkKind::Box { color: s.color, fill, outline, alt_color: s.alt_color },
            "ellipse" => MarkKind::Ellipse { color: s.color, fill, outline, alt_color: s.alt_color },
            "pen" => MarkKind::Pen { color: s.color, points: s.points.unwrap_or_default() },
            "text" => MarkKind::Text { color: s.color, alt_color: s.alt_color, text: s.text.unwrap_or_default() },
            "blur" => MarkKind::Blur { round: s.round.unwrap_or(false) },
            other => return Err(format!("unknown tool '{other}'")),
        };
        let numbers = [s.x1, s.y1, s.x2, s.y2, s.size];
        if numbers.iter().any(|v| !v.is_finite()) || s.size <= 0.0 || s.size > 4096.0 {
            return Err(format!("mark {} has impossible numbers", s.id));
        }
        Ok(Mark { id: s.id, x1: s.x1, y1: s.y1, x2: s.x2, y2: s.y2, size: s.size, kind })
    }
}

pub enum Opened { Fresh, Ready(ShotDoc), TooNew(u32), Unreadable(String) }

pub fn read(path: &std::path::Path) -> Opened {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Opened::Fresh,
        Err(e) => return Opened::Unreadable(e.to_string()),
    };
    let disk: OnDisk = match serde_json::from_str(&text) {
        Ok(disk) => disk,
        Err(e) => return Opened::Unreadable(e.to_string()), // never "no edit" again
    };
    if disk.version > VERSION { return Opened::TooNew(disk.version); }
    let marks = match disk.scene {
        Some(scene) if disk.version == VERSION => Ok(scene),
        _ if disk.marks.is_empty() => Ok(Vec::new()),
        _ => serde_json::from_str::<Vec<ShapeV1>>(&disk.marks)
            .map_err(|e| e.to_string())
            .and_then(|v| v.into_iter().map(Mark::try_from).collect()),
    };
    let marks = match marks { Ok(m) if m.len() <= 2_000 => m, Ok(_) => return Opened::Unreadable("too many marks".into()), Err(e) => return Opened::Unreadable(e) };
    let renderer = disk.renderer.unwrap_or(if disk.blur_passes >= 3 { Renderer::RustBox3 } else { Renderer::RustBox1 });
    Opened::Ready(ShotDoc { rev: disk.rev, crop: disk.crop, marks, output: disk.output, renderer })
}

/// A v2 `Mark` serialises to a v1-compatible `Shape`, so the shadow is simply
/// the marks as a string — 0.8.x reads it and keeps editing.
pub fn to_disk(doc: &ShotDoc, pending: Option<&std::path::Path>) -> Result<Vec<u8>, String> {
    serde_json::to_vec(&OnDisk {
        version: VERSION, rev: doc.rev, crop: doc.crop,
        marks: serde_json::to_string(&doc.marks).map_err(|e| e.to_string())?,
        scene: Some(doc.marks.clone()), output: doc.output.clone(),
        renderer: Some(doc.renderer), blur_passes: 3,
        pending: pending.map(|p| p.to_string_lossy().into_owned()),
    }).map_err(|e| e.to_string())
}
```
`ShotDoc { rev, crop, marks, output, renderer }`, `Fingerprint { bytes, modified_ms, sha256 }`, `Renderer { RustBox1, RustBox3, Scene1 }` and `shot::Rect` carry the same `cfg_attr` derives. `Fingerprint::of(bytes, path)` (len + mtime + sha256 via the existing `sha2` dependency, Cargo.toml:53), `Fingerprint::matches(path)` and `pending_of(part)` live in this module.

### Sketch 2: `shot_commit.rs` — per-clip lock, crash-safe commit, binary command
```rust
use std::{collections::HashSet, io::Write, path::{Path, PathBuf}};
use parking_lot::{Condvar, Mutex};
use tauri::Manager;
use crate::shot_doc::{self, Fingerprint, ShotDoc};

/// One commit/move/delete per clip. Lock order: clip, then library.
#[derive(Default)]
pub struct ClipLocks { busy: Mutex<HashSet<String>>, freed: Condvar }
pub struct ClipGuard<'a> { locks: &'a ClipLocks, id: String }

impl ClipLocks {
    pub fn lock(&self, id: &str) -> ClipGuard<'_> {
        let mut busy = self.busy.lock();
        while busy.contains(id) { self.freed.wait(&mut busy); }
        busy.insert(id.to_owned());
        ClipGuard { locks: self, id: id.to_owned() }
    }
}
impl Drop for ClipGuard<'_> {
    fn drop(&mut self) {
        self.locks.busy.lock().remove(&self.id);
        self.locks.freed.notify_all();
    }
}

pub struct Target<'a> { pub picture: &'a Path, pub store: &'a Path }
pub struct Staged { temp: PathBuf, part: PathBuf }

fn write_new(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// (1) Stage everything beside its destination, synced, invisible.
pub fn stage(t: &Target, png: &[u8], mut doc: ShotDoc) -> Result<(Staged, ShotDoc), String> {
    let temp = t.picture.with_extension(format!("{}.cbtmp", uuid::Uuid::new_v4().simple()));
    write_new(&temp, png).map_err(|e| format!("Could not write the picture ({e})."))?;
    doc.output = Some(Fingerprint::of(png, &temp)); // NTFS keeps mtime across rename
    let part = t.store.join(format!("shot.json.r{}.part", doc.rev));
    if let Err(e) = shot_doc::to_disk(&doc, Some(&temp))
        .and_then(|bytes| write_new(&part, &bytes).map_err(|e| e.to_string()))
    {
        let _ = std::fs::remove_file(&temp);
        return Err(format!("Could not note the edit ({e})."));
    }
    Ok((Staged { temp, part }, doc))
}

/// (2) The picture swap is the commit point — the step that fails in practice.
/// (3) The document follows; a crash in between is rolled forward by `recover`.
pub fn publish(t: &Target, s: Staged) -> Result<(), String> {
    if let Err(err) = crate::muxer::replace_file(&s.temp, t.picture) {
        let _ = std::fs::remove_file(&s.temp);
        let _ = std::fs::remove_file(&s.part);
        return Err(err); // old picture + old shot.json: consistent
    }
    if let Err(err) = std::fs::rename(&s.part, t.store.join("shot.json")) {
        log::warn!("committed; document left staged for recovery: {err}");
    }
    Ok(())
}

/// At open (under the lock) and at startup.
pub fn recover(t: &Target) {
    let Ok(dir) = std::fs::read_dir(t.store) else { return };
    for path in dir.flatten().map(|e| e.path()) {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if !(name.starts_with("shot.json.r") && name.ends_with(".part")) { continue; }
        let published = matches!(shot_doc::read(&path), shot_doc::Opened::Ready(ref d)
            if d.output.as_ref().is_some_and(|fp| fp.matches(t.picture)));
        if published {
            let _ = std::fs::rename(&path, t.store.join("shot.json")); // roll forward
        } else {
            if let Some(temp) = shot_doc::pending_of(&path) { let _ = std::fs::remove_file(temp); }
            let _ = std::fs::remove_file(&path); // roll back
        }
    }
}

/// Body: b"CBS2" | u32 LE header length | header JSON | RGBA from the worker.
#[tauri::command]
pub async fn commit_screenshot(
    app: tauri::AppHandle,
    request: tauri::ipc::Request<'_>,
) -> Result<crate::commands::ShotOpen, String> {
    let tauri::ipc::InvokeBody::Raw(body) = request.body() else {
        return Err("commit_screenshot takes a binary body".into());
    };
    if body.len() < 8 || &body[..4] != b"CBS2" { return Err("not a commit frame".into()); }
    let size = u32::from_le_bytes(body[4..8].try_into().unwrap()) as usize;
    let header: crate::commands::CommitHeader =
        serde_json::from_slice(body.get(8..8 + size).ok_or("short frame")?).map_err(|e| e.to_string())?;
    let body = body.clone(); // the request borrows it; ~10 ms at 4K
    let worker = app.clone();
    let opened = tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<crate::state::AppState>();
        let _clip = state.clip_locks.lock(&header.id);
        // Under the lock: re-read clip.path, recover, check rev == baseRev,
        // size the crop from the PNG header, require len == w*h*4, RGBA->RGB,
        // stamp_kept, encode_png, stage + publish, still_from, set_picture.
        crate::commands::commit_composite(&state, header, &body[8 + size..])
    })
    .await
    .map_err(|e| e.to_string())??;
    crate::commands::announce_change(&app); // made pub(crate)
    Ok(opened)
}
```

### Sketch 3: `src/editor/raster.ts` and `render.ts` — the one rasteriser and its export frame
```ts
// raster.ts — pure integer code: runs in vitest (node), the worker and the preview.
export interface Area { x: number; y: number; width: number; height: number }

/** Three boxes with the variance of one of half-width r (what sigmaOf assumes). */
export const boxRadii = (r: number) =>
  Math.max(1, Math.floor((Math.sqrt(1 + (4 * r * (r + 1)) / 3) - 1) / 2 + 0.5));

/** Line-for-line port of `blur_rows` (shot.rs:633-661), strided over RGBA. */
function pass(src: Uint8ClampedArray, dst: Uint8ClampedArray, lines: number, length: number,
              lineStep: number, step: number, r: number) {
  for (let line = 0; line < lines; line++) {
    const base = line * lineStep;
    for (let c = 0; c < 3; c++) {
      let first = 0, last = Math.min(r, length - 1), sum = 0;
      for (let i = 0; i <= last; i++) sum += src[base + i * step + c];
      for (let i = 0; i < length; i++) {
        dst[base + i * step + c] = Math.floor(sum / (last - first + 1));
        const next = i + 1;
        if (next === length) break;
        if (next > r) { sum -= src[base + first * step + c]; first++; }
        if (last + 1 < length && next + r > last) { last++; sum += src[base + last * step + c]; }
      }
    }
  }
}

/** Blur `rect` (patch coords, may overhang) of an opaque RGBA patch in place. */
export function blur3(px: Uint8ClampedArray, pw: number, ph: number, rect: Area, r: number, round: boolean) {
  const ri = boxRadii(r);
  const a = px.slice(), b = px.slice();
  for (let k = 0; k < 3; k++) {
    pass(a, b, ph, pw, pw * 4, 4, ri); // across
    pass(b, a, pw, ph, 4, pw * 4, ri); // down
  }
  for (let y = Math.max(0, -rect.y); y < rect.height && rect.y + y < ph; y++)
    for (let x = Math.max(0, -rect.x); x < rect.width && rect.x + x < pw; x++) {
      // Oval from the *requested* rect, 4×4 samples, same f64 maths as shot::oval_coverage.
      const cov = round ? ovalCoverage(x, y, rect.width, rect.height) : 16;
      const at = ((rect.y + y) * pw + rect.x + x) * 4;
      for (let c = 0; c < 3; c++) px[at + c] = (a[at + c] * cov + px[at + c] * (16 - cov) + 8) >> 4;
    }
}

// render.ts — preview: scale = display/original; export: scale = 1.
// import type { Mark } from "@/lib/generated/Mark"; import { paint } from "./paint";
// interface Sources { original: ImageBitmap; under?: { bitmap: ImageBitmap; x: number; y: number } }
export function renderScene(ctx: OffscreenCanvasRenderingContext2D, src: Sources,
                            marks: readonly Mark[], view: { region: Area; scale: number }, from = 0) {
  const { region, scale } = view;
  const picture = () => ctx.setTransform(scale, 0, 0, scale, -region.x * scale, -region.y * scale);
  if (from === 0) {
    picture();
    ctx.imageSmoothingQuality = "high";
    ctx.drawImage(src.original, 0, 0);
    if (src.under) ctx.drawImage(src.under.bitmap, src.under.x, src.under.y); // clean mark area
  }
  let run: Mark[] = [];
  const flush = () => { if (run.length) { picture(); paint(ctx, run); run = []; } };
  for (let i = from; i < marks.length; i++) {
    const mark = marks[i];
    if (mark.tool !== "blur") { run.push(mark); continue; }
    flush();
    const r = Math.max(1, Math.floor(mark.size * scale)); // preview never stronger than file
    const m = 3 * boxRadii(r);
    const rect = {
      x: Math.round((Math.min(mark.x1, mark.x2) - region.x) * scale),
      y: Math.round((Math.min(mark.y1, mark.y2) - region.y) * scale),
      width: Math.round(Math.abs(mark.x2 - mark.x1) * scale),
      height: Math.round(Math.abs(mark.y2 - mark.y1) * scale),
    };
    const x0 = Math.max(0, rect.x - m), y0 = Math.max(0, rect.y - m);
    const x1 = Math.min(ctx.canvas.width, rect.x + rect.width + m);
    const y1 = Math.min(ctx.canvas.height, rect.y + rect.height + m);
    if (rect.width < 2 || rect.height < 2 || x1 <= x0 || y1 <= y0) continue;
    const patch = ctx.getImageData(x0, y0, x1 - x0, y1 - y0);
    blur3(patch.data, x1 - x0, y1 - y0, { ...rect, x: rect.x - x0, y: rect.y - y0 }, r, mark.round);
    ctx.putImageData(patch, x0, y0);
  }
  flush();
}

// renderer.worker.ts — the frame commit_screenshot reads; posted with [frame.buffer].
export function exportFrame(src: Sources, marks: readonly Mark[], region: Area, crop: Area,
                            header: { id: string; baseRev: number; crop: Area | null; marks: readonly Mark[] }) {
  const ctx = new OffscreenCanvas(region.width, region.height).getContext("2d", { willReadFrequently: true })!;
  renderScene(ctx, src, marks, { region, scale: 1 }); // region = crop ∪ blur reach (commands.rs:567-584 semantics)
  const rgba = ctx.getImageData(crop.x - region.x, crop.y - region.y, crop.width, crop.height).data;
  const json = new TextEncoder().encode(JSON.stringify({ ...header, width: crop.width, height: crop.height }));
  const frame = new Uint8Array(8 + json.length + rgba.length);
  frame.set([0x43, 0x42, 0x53, 0x32]); // "CBS2"
  new DataView(frame.buffer).setUint32(4, json.length, true);
  frame.set(json, 8);
  frame.set(rgba, 8 + json.length);
  return frame; // main thread: invoke<ShotOpen>("commit_screenshot", frame)
}
```

## 5. Open questions

1. **Shadow lifetime.** How many releases keep writing the v1 `marks` shadow, which keeps 0.8.x able to edit?
2. **Legacy blurs.** Re-redact screenshots made with the 1-pass blur through an opt-in card (proposed) or automatically? Should the release notes warn that files already shared stay weak?
3. **Radius floor.** Enforce a minimum blur radius (for example h/4 of the rectangle), or rely on the Redact tool and an honest hint (ShotViewer.tsx:1107)?
4. **Watermark setting.** `stamp_kept` runs whenever `under.png` exists, even when the watermark is switched off (commands.rs:585-587). Should commits honour the setting?
5. **Dependencies.** Are `ts-rs` (dev), `jpeg-encoder` (runtime), vitest and Playwright acceptable under the supply-chain policy?
6. **Headless rendering.** Is a hidden WebView acceptable for future capture-time auto-redaction? If not, revisit B; the scene format permits it.
7. **Budgets and caps.** Which reference machine do the perf budgets assume? Is 8K a supported editing size? The proposed caps are a 16384 px side and 134 MP.
8. **External edits.** Should the default be "use this file as the original" or "overwrite"?
