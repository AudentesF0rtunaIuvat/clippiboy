# ClippiBoy screenshot editor: feature, UX and architecture review

This was a read-only review. I changed no repo files and wrote one scratch script. Every claim about how the editor behaves today cites file:line. Anything marked "proposed" does not exist yet.

---

## 0. Executive summary

**What's already good:**
- The edit model is an object model: `Shape[]` is kept as JSON beside the file. Every save rebuilds the picture from the untouched original, so crop and marks can each be taken back on their own.
- The watermark is re-stamped after every edit.
- Arrows, boxes, pen and text are painted by one routine (`paint`) on Chromium canvas for both the preview and the file, so those marks cannot drift.
- Marks can already be selected, moved, resized and restyled after placing.

**Biggest UX bugs:**
1. The header ✕ closes the viewer without asking about unsaved marks.
2. Save errors are silent. `lastError` is only shown on the Settings page.
3. Marks drawn while a save is running are silently overwritten.
4. Once the thickness slider or a colour field has focus, Delete, Ctrl+Z and Escape stop working. Drawing on the picture does not take that focus back.
5. "Remove all marks" writes the file at once, with no question and no undo.
6. You cannot see your marks while cropping, even though the crop cuts through them.

**Biggest feature gaps against ShareX, Greenshot, Flameshot, Snagit, Snipping Tool, macOS Markup and CleanShot X:**
- No redo.
- No highlighter, plain line, numbered steps, spotlight, pixelate, magnifier or emoji.
- No tool hotkeys and no Shift to keep lines straight or boxes square.
- No copy or export button inside the editor.
- No choice of format, quality, scale or target size. 4K game PNGs can easily exceed Discord's 10 MB limit.
- No multi-select, z-order, duplicate or snapping.
- No OCR.
- No "after capture: copy or open editor" option.

**Architecture verdict:**
- The WebView/Rust split is right for vector marks and wrong for anything that reads pixels.
- Blur already drifts between preview and file:
  - The preview uses a CSS Gaussian; the file uses a single box pass.
  - The preview's oval is antialiased; the file's oval mask has a hard edge.
- Every future pixel tool (pixelate, magnifier, blur outside a spotlight, beautify shadow, eyedropper, JPEG/WebP export) would need two implementations that must be kept in step.
- **Short term:** contain the drift (3-pass box blur, feathered oval mask, parity tests), switch to binary IPC, and add a version to the sidecar file.
- **Medium term:** move the whole composite into one WebView renderer (an OffscreenCanvas in a Worker). Rust keeps owning the files, atomic writes, watermark stamping, the clipboard and thumbnails.

---

## 1. Inventory: what exists today

### 1.1 Entry point, modes and layout
- **How you get there:** the gallery opens `ShotViewer` for screenshot entries (`src/routes/Clips.tsx:655-663`).
- **Three exclusive modes:** `"view" | "crop" | "draw"` (`ShotViewer.tsx:165`). There is no separate Edit button; clicking any tool enters draw mode (`pickTool` 585-592), and clicking a crop preset enters crop mode (`pickPreset` 541-549).
- **Side column:** 380 px wide (939-941). From top to bottom: `ClipMeta` (name, description, game), then the "Annotate" section (open by default, 193), then the "Crop" section.
- **Footer:**
  - Favorite, Show in folder, and Delete (asks first).
  - Previous / next picture, disabled while editing (1246-1264).
  - A hint text (1233-1237).

### 1.2 Tools: `src/components/ShotAnnotate.tsx`
- **Tool list:** `select | arrow | box | ellipse | pen | text | blur` (14-24). The buttons are text-only, in a 4-column grid (`ShotViewer` 951-962).
- **Colours:**
  - 6 swatches (red, amber, green, violet, white, near-black) at 27-34.
  - Plus a custom mixer (`ui/ColorPicker.tsx`): saturation/value field, hue slider, hex and RGB fields.
  - No alpha channel, no eyedropper, no recent colours.
- **A second ink (`altColor`):**
  - Text always has a "rim" (148-151, 245-261).
  - A box or ellipse gets a separate border colour only when it is both filled and outlined.
- **Looks:** box and ellipse can be Outline, Filled or Both (89-93, `ShotViewer` 1014-1026).
- **Blur shape:** Rectangle or Ellipse (96-99, `ShotViewer` 1028-1041).
- **One size slider per tool** (`range`, 119-129). Sizes scale with picture width through `baseStroke` (`max(2, round(w/480))`, 114-116).
  - Strokes: 0.4× to 6× base.
  - Text height: 3× to 24×.
  - Blur radius: 1× to 12×.
- **Arrow** (212-233): a straight shaft with a filled triangular head, head length `max(size*4, 10)`, round caps. The round cap pokes out past the tip, blunting it on thick arrows. There are no line-only, double-headed, curved or tapered variants.
- **Box and ellipse** (190-210): no corner radius, no opacity. `globalAlpha` is always 1 (181, 269).
- **Pen** (235-243):
  - A polyline. Points closer than one displayed pixel are dropped (843-852).
  - No smoothing, no pressure.
  - No resize handles (`handlesOf` returns none, 457).
- **Text** (245-261):
  - Font: `600 {size}px Inter, "Segoe UI"` (284-286). Inter is not bundled (no `@font-face`), so in practice it is Segoe UI Semibold.
  - Lines break only where Enter was pressed. The rim is a stroke painted under the fill.
  - You type into an in-place textarea styled to match the canvas: same font, `WebkitTextStroke`, `paint-order` (989-1033).
  - Enter adds a line; Escape or Ctrl+Enter puts the text down (1001-1010). An emptied text removes the mark (874-880).
  - Double-click a text to edit it (926-942).
  - No background or label box, no alignment, no font choice, and no way to turn the rim off.
- **Blur** (263-266, 1045-1081): the preview is a DOM `backdrop-filter: blur(σ)` patch. The file blur is done in Rust (see 1.5).
- **Select, move, resize and restyle existing marks: exists.**
  - The Select tool does a hit test (`hit` 345-397). The test is generous, and a hollow frame only answers on its line, so arrows inside frames stay pickable.
  - Moves are kept inside the picture (785-803).
  - Handles: arrow endpoints, 4 corners for box/ellipse/blur, one scale handle for text (447-470, 961-985).
  - A newly finished mark stays selected (866).
  - The panel edits the selected mark's style (`ShotViewer` 651-687). This does not update the tool's default for the next mark.
  - Remove button and Delete/Backspace (1043-1057, 389-393).
- **Not there:** z-order commands, multi-select, duplicate, copy/paste of marks, snapping, arrow-key nudge, rotating or flipping marks, Shift to constrain.

### 1.3 Undo, zoom/pan and keyboard
- **Undo**, in `ShotViewer` 256-307:
  - Snapshot-based. A drag, or one slider pull, counts as one step via a `gesture` key (281-294).
  - Ctrl+Z (394-404) and an Undo button (1070-1077).
  - Redo is deliberately missing (401-402).
  - History is cleared on every save (`resetShapes` 267-272, 634).
- **Zoom and pan:**
  - The wheel zooms around the pointer, from 1× to 8× of the "fit" size (471-493, 771-779).
  - The middle button pans in any mode; the left button pans only in view mode (496-517).
  - In view mode, double-click toggles zoom (813-821). A % badge resets the view (921-934).
  - The % shown is relative to fit, not to actual pixels.
  - There are no keyboard zoom keys, no "1:1" view, and no Space+drag.
  - Horizontal scroll (`deltaX`) is ignored.
- **All keys** (374-419):
  - Escape: leave crop, then leave draw mode (asks if anything is unsaved), then close the viewer.
  - Delete/Backspace: remove the selected mark.
  - Ctrl+Z: undo.
  - ←/→: previous/next picture, view mode only.
  - Tools have no hotkeys, and there is no Ctrl+S or Ctrl+C.
  - Note: `lib/lockdown.ts:29-47` blocks the browser's own Ctrl+S, Ctrl+=, Ctrl+- and Ctrl+0 with preventDefault only, so the editor is free to use them.

### 1.4 Crop: `ShotViewer.tsx` 78-127 and 1280-1456
- **Ratio presets:** Free, Original, 16:9, 9:16, 4:3, 3:2, 1:1, 21:9, 4:5 (78-88). Clicking a preset starts the crop already shaped to it (541-549).
- **Selection:** draw it, move it, or resize it with **4 corner handles only** (1396-1405). There are no edge handles.
- **Guides:** a rule-of-thirds grid (1434-1439), the selection size in px (1139-1143), and a minimum edge of 16 px (63, 1381-1392).
- **Undo crop** puts the whole picture back and keeps the marks (1163-1178).
- **Not there:** typing exact numbers, pixel-size presets, rotate/straighten, scaling on export.
- **Not visible during crop:** marks. The crop ground is the untouched original (439-441), and the marks layer is only mounted in draw mode (880-892).

### 1.5 The edit model: what's stored, rasterised and sent

**In the UI:**
- `Shape[]` (ShotAnnotate 43-75) holds floats in picture pixels.
- While drawing, coordinates are relative to the *cropped* picture. They are shifted into the original's coordinates on save and back on load (`shift` / `readMarks`, ShotViewer 1464-1485, 608).

**On disk** (`%APPDATA%\ClippiBoy\originals\<id>\`, `edit::dir`, edit.rs:56-63):
- `shot.json` is `{crop: Rect|null, marks: string}`, where `marks` is the `Shape[]` JSON **encoded a second time as a string** (shot.rs:311-320).
  - The core never reads it (316-318).
  - **There is no version field.** Old marks are recognised by missing fields, e.g. `outline !== false` (ShotAnnotate 192-194).
  - It is written atomically: written to `.part`, then renamed (shot.rs:342-356).
  - A file that won't parse is silently read as "no edit" (`unwrap_or_default`, 335-340).
- `image.png`: the untouched original, copied once on the first edit (`keep_original`, commands.rs:606-620). It includes the baked-in watermark.
- `base.png`: the original with the watermark area laid back clean and the crop applied, but no marks. It exists only when there is a crop (commands.rs:589-595).
- `under.png` + `under.json`: the pixels that lay under the watermark at capture time, plus where they sit and the picture size (shot.rs:358-427; written at `state.rs:915-928`).

**Transport:**
- `toSteps` (ShotAnnotate 491-525) splits the marks into ordered `Step`s: a `layer` (a run of vector marks) or a `blur` (a rectangle).
- Each layer is drawn again on a fresh canvas **at the original's full resolution** (`layerBytes`, ShotViewer 1607-1624). It is PNG-encoded and sent as a JSON `number[]` (`Array.from(Uint8Array)`, 1623; types.ts:525-527; commands.rs:400-419).

**Rendering in the core** (`edit_screenshot`, commands.rs:511-599):
1. Decode the original.
2. Paste the clean watermark area back in.
3. Clamp the crop.
4. Build the cropped ground.
5. Apply the steps in order: `blend` uses straight alpha (shot.rs:104-121); `blur` is one sliding-window box pass per axis, reading a margin of `radius` around the rectangle (139-210), with a hard-edged oval mask (199-208).
6. Crop.
7. Re-stamp the watermark (`stamp_kept` 586).
8. Write `base.png` and `shot.json`.
9. Write the PNG (`Compression::Fast`, shot.rs:267) to a temp file and swap it in (`write_picture` 627-652).
10. Regenerate the thumbnail with ffmpeg (thumbs.rs:51-64) and update the stored size.

**What is rasterised where:**

| Part | Rendered by |
|---|---|
| Vector marks and text | WebView canvas (`paint`) |
| Blur | Preview: CSS `backdrop-filter`. File: Rust box blur |
| Watermark | Preview: an `<img>` at `markRect` (`lib/watermark.ts`, a hand copy of `watermark::rect_kept`). File: Rust area-average scaling at 0.65 opacity (watermark.rs:151-205) |

### 1.6 Watermark behaviour
- Stamped onto fresh screenshots when the setting is on (watermark.rs:132-141). What lay under it is kept.
- **Every edit lays the clean area back and stamps a fresh watermark**, whatever was cropped or painted (commands.rs:540-545, 585-587). It shrinks to the smallest legible size on small crops (`rect_kept` 94-125).
- The editor previews where it will land, live while you drag a crop (ShotViewer 519-530, 855-920).
- Re-stamping depends on `under.png` existing, not on the current setting. A user who has switched the watermark off still gets it re-stamped on older screenshots.
- A picture resized outside ClippiBoy is detected and edited as it is (shot.rs:402-420).
- **Parity check:** I compared Rust's f32 `rect_kept` against TS's f64 `markRect` for heights 1-4399 against 19 widths each. There were 0 mismatches (script: `<review-scratch>/wm.py`). So there is no drift today, but no test guards it.

### 1.7 Copy, save and export
- **Copy picture** (CF_DIB, read back from the edited file) and **Copy file** (CF_HDROP) exist only in the right-click menu. That menu is in the gallery and on the stage in view mode only (clipMenu.tsx:161-180, ShotViewer 767-770, commands.rs:659-668, clipboard.rs:131-135).
  - There is no success or failure feedback (`void api.copyClipImage(...)`).
  - Only one clipboard format is offered: no registered "PNG" format, and no HDROP alongside the picture.
- **Save** writes over the screenshot's own file. There is no save-as and no duplicate.
- "Export a copy…" and share links are for clips only (clipMenu.tsx:191-203). The output is always PNG.
- No drag-out: the stage image has `draggable={false}` (844).
- No setting for what happens after capture (config.rs has only the hotkey and the watermark switch).

### 1.8 Accessibility and input
- **What's there:** tool and crop buttons have `aria-pressed` (PanelButton 1583); sections have `aria-expanded` (1534); the text field has an `aria-label` (995); reduced motion is respected on close (207).
- **Viewer:** no `role="dialog"`, `aria-modal`, focus trap or focus restore (709-716).
- **Handles and the drawing surface:** plain divs. They cannot be focused and have no labels (917-943, 962-984).
- **Controls:**
  - The slider has `outline-none`, so there is no visible focus (Controls.tsx:91-124).
  - `Segmented` has no radiogroup or pressed semantics (Controls.tsx:47-89).
  - Swatch labels read out hex codes ("Colour #ef4444", ColorPicker.tsx).
- **Touch:** `touch-none` lets finger drawing work. There is no pressure, pinch-zoom or two-finger pan.
- **HiDPI is handled correctly.** Layout uses CSS-pixel sizes (`clientWidth`), and canvases are kept at the picture's own resolution.

### 1.9 Tests
- **Rust:** unit tests for the sliding blur against a naive version, the blur's margin, the crop clamp (shot.rs:709-823), and the watermark rectangles (watermark.rs:219-303).
- **TypeScript:** there is no test runner at all (package.json has no vitest or jest).

---

## 2. Gaps against best-in-class editors

Status: **E** = exists, **P** = partial, **–** = missing.

| Capability | ClippiBoy | Reference |
|---|---|---|
| Arrow / box / ellipse / pen / text | E | all |
| Plain line, double arrow, curved or tapered arrow | – | Snagit, CleanShot, ShareX |
| Rounded rect, speech bubble / callout, text background | – | Snagit, CleanShot, Greenshot |
| Numbered step counter | – | Snagit, CleanShot, ShareX, Flameshot |
| Highlighter (semi-transparent or multiply) | – (no opacity at all) | Snipping Tool, Markup, Greenshot |
| Spotlight (darken outside) | – | Snagit, CleanShot |
| Pixelate | – | ShareX, Greenshot, Flameshot |
| Gaussian blur | P (box blur in the file) | all |
| Solid redaction | P (Box + Filled + black, not labelled as redaction) | CleanShot, ShareX |
| Magnifier inset | – | Snagit, ShareX |
| Emoji / stickers | – | CleanShot, ShareX |
| Select, move, resize, restyle after placing | E (no pen resize) | all |
| Z-order, multi-select, duplicate, copy/paste marks | – | Snagit, ShareX, CleanShot |
| Snapping / alignment guides; Shift-constrain | – | all serious ones |
| Undo / redo | undo only | all |
| Keyboard-first (tool letters, Ctrl+S, Ctrl+C, nudge) | – | Flameshot, ShareX, CleanShot |
| Crop ratio presets | E | Snipping Tool, Markup |
| Crop edge handles, typed numbers, pixel-size presets (Twitter, IG, YT thumb) | – | Snagit, CleanShot |
| Rotate / flip; scale on export | – | Snagit, Greenshot, ShareX |
| Beautify (padding, background, radius, shadow) | – | CleanShot, ShareX |
| Format and quality choice (PNG/JPEG/WebP), size target | – (the clip side has one: ExportDialog.tsx:18-22) | ShareX, Snagit |
| Copy from inside the editor; "Save & copy"; copy right after capture | – (context menu only) | all |
| Drag-out of the file | – | CleanShot, Snipping Tool |
| OCR / copy text; searchable screenshots | – (Windows.Media.Ocr available) | Snipping Tool, ShareX, CleanShot |
| Auto-redaction | – | some, via OCR |
| Eyedropper / recent colours | – | Snagit, ShareX |
| Pen pressure, smoothing, pinch-zoom, touchpad pan | – | Snipping Tool, Markup |
| Edit any image (open or paste) | – (`edit_screenshot` refuses non-screenshots, commands.rs:520; drops are blocked by lockdown.ts:111-117) | all |
| Accessibility (dialog semantics, focus, keyboard drawing, list of marks) | weak | Snipping Tool is decent |

---

## 3. Architecture critique

### 3.1 What's sound
- **Non-destructive rebuild from the original** (commands.rs:484-599) is the right core. Crop and marks really are independent, and "Undo crop" keeps the marks (1174).
- **Vector marks and text render in one engine for both preview and file.** The preview canvases and `layerBytes` both call the same `paint`, so antialiasing, line caps, fonts and text metrics cannot drift. This was a good call: drawing text in Rust would mean shipping a font stack and would still differ from what the user typed.
- **The watermark can't be removed by editing**, and the core re-stamps it.
- **Writes are crash-safe:** `shot.json` goes through `.part` + rename (shot.rs:342-356), and the picture through temp + replace (commands.rs:627-638). Nothing is written until everything that can fail has run (commands.rs:555-565).

### 3.2 Problems

1. **Pixel operations are implemented twice and already drift.**
   - Blur: the preview is a CSS Gaussian with σ matched to the box's variance (ShotAnnotate 1069-1081); the file is a **single** box pass (shot.rs:123-127). Same variance, different kernel. A box kernel's frequency response has side lobes, so fine detail such as text strokes survives better in the file than in the preview. For redaction, that is the wrong way round.
   - The oval: the preview clips with an antialiased `border-radius:50%` (1062); Rust uses a hard in/out test (shot.rs:199-208).
   - Edges: the Rust comment (129-134) assumes `backdrop-filter` blurs the whole picture and clips afterwards. Chromium may instead clip the backdrop to the element and mirror at its edges. **Verify in WebView2.**
   - Every planned pixel tool would add another pair of implementations to keep in step, each previewed with whatever CSS can fake.
2. **Comments contradict the code.** ShotAnnotate 166-172 and 607-610 say "the export is this very canvas". In fact the export is drawn again (ShotViewer 1600-1606). Harmless, but it misleads anyone extending the code.
3. **Preview memory grows with blur count.** Each run of marks is its own full-resolution canvas (566-586), and each blur splits the run. A 4K canvas is about 33 MB, so 10 blurs interleaved with marks is about 11 canvases, roughly 360 MB. The export path then encodes one full-size PNG per layer.
4. **IPC payload is inefficient.** PNG bytes travel as a JSON `number[]` (1623), roughly 3-4× the bytes plus parse time. Tauri 2 accepts raw bodies (`tauri::ipc::Request` / `InvokeBody::Raw`) and can return raw bytes (`tauri::ipc::Response`).
5. **The core can't do anything without a save round trip.** Marks are an opaque, doubly-encoded string. So the core cannot validate marks, migrate them, render headlessly (presets or auto-redact at capture time), or export the *current* state. "Copy what I see" means saving first.
6. **No sidecar versioning, and failure is silent.**
   - `read_edit` turns any parse error into "no edit" (shot.rs:335-340).
   - If a newer version's file is opened by an older build, the editor would show `base.png` (cropped) as if it were uncropped and could wipe the edit on the next save.
   - The rules for reading old marks are scattered (`outline !== false`, etc.).
7. **Types are copied by hand across the boundary.** `ShotStep`, `ShotEdit` and `ShotRect` are written in both languages (types.ts:511-547, commands.rs:400-444). `watermark.ts` mirrors `rect_kept` with only a "change one, change both" comment (watermark.ts:6-8). Also, `api.screenshotHasOriginal` calls a Rust command that does not exist (ipc.ts:150-151); it is dead code.
8. **Blur coordinates are `u32`.** A single negative number fails the whole save at deserialization. The TS side works around this by clamping drags (ShotAnnotate 770-775); accepting signed values and clamping in Rust would be sturdier.
9. **No save lock.** The clip editor serialises its saves with `WORKING` (edit.rs:48-53); `edit_screenshot` has no lock. The UI's `busy` flag is the only guard.

### 3.3 Options

**A. One renderer in the WebView. Recommended for the medium term.**
- A pure module `renderScene(ctx, sources, scene, {scale, region})` draws everything:
  - vectors (the current `paint`);
  - blur via `ctx.filter = "blur(σ)"` inside a clip path (reading the whole composite, which matches today's Rust margin semantics);
  - pixelate via a downscale with `imageSmoothingEnabled=false`;
  - spotlight via `evenodd` fills; magnifier via a clipped `drawImage`;
  - beautify shadows via `shadowBlur`.
- **Preview:** the same function, drawn into a canvas at display resolution × devicePixelRatio. Cache a bitmap of everything below the mark being dragged and everything above it, so only the active mark is redrawn per pointer move. Redraw at higher resolution when zoomed.
- **Export:** the same function in an OffscreenCanvas Worker at full resolution.
  - Output is a raw RGBA buffer, or a PNG/JPEG/WebP blob, which gives format and quality choice for free (`convertToBlob({type, quality})`).
  - It travels over binary IPC to a `commit_screenshot(id, crop, scene_json, rgba|bytes)` command. Rust re-stamps the watermark with `stamp_kept`, keeping the guarantee, then writes atomically, regenerates the thumbnail and records the size.
- **One prerequisite:** an untainted copy of the original's pixels.
  - Drawing an asset-protocol `<img>` is fine for the preview, because a tainted canvas can still be drawn. Export needs pixel read-back.
  - Either add a binary command `shot_source(id) -> tauri::ipc::Response` and decode it with `createImageBitmap(new Blob([bytes]))`, or confirm the asset protocol sends CORS headers and add `connect-src` to the CSP. The current CSP is `default-src 'self'` (tauri.conf.json:31), so `fetch(asset:)` is blocked.
- **Result:** what you see is what you get by construction. "Copy current state" and format export need no save. Each new tool is written once.
- **Cost:** about 33 MB per canvas buffer, PNG encoding in the Worker, and `watermark.ts` still mirrors Rust (now pinned by a parity test).

**B. One renderer in Rust (tiny-skia + cosmic-text or swash + a bundled Inter):**
- Good for headless jobs (auto-redact at capture time).
- But text in the file would no longer match the DOM field the user typed into.
- Previews need a round trip per change, and the binary grows with the font stack.
- Not recommended as the main path for this app.

**C. Keep the hybrid but contain it. Do this now, regardless of A:**
- Rust blur: three box passes (Kovesi-style radii), which approximates a Gaussian at the same O(1) cost per pixel. Update `sigmaOf` to match.
- Rust oval: feather the mask by coverage.
- Add parity tests.
- Binary IPC for layers.
- Generate the shared types with ts-rs or tauri-specta.
- Versioned, typed sidecar.

**Recommendation:** do C now, which is cheap and removes today's drift. Move to A when the second pixel-reading tool or JPEG/WebP export lands. That is the point where duplicate implementations stop being affordable.

### 3.4 Can the model grow?
- **Today:** a new vector tool touches `Tool`/`TOOLS`, a `paint` case, `bounds`/`hit`/`handlesOf`, `range`/`sizeLabel`/`has*`, and `defaultStyles`. It needs no Rust change.
- **A new pixel tool** needs a new `Step` variant in TS and Rust, a Rust implementation, and a DOM preview. Under A, it is just a `renderScene` case.
- **Cross-mark state** such as automatic step numbering must be worked out **before** `toSteps` splits the marks. `paint` runs once per layer (566-586, 609-617), so a counter inside `paint` would restart after every blur.
- **Proposed sidecar v2:**
  ```json
  { "version": 2, "crop": {"x":0,"y":0,"width":1280,"height":720},
    "transform": {"rotate":0,"flipX":false},
    "marks": [ { "id":3, "tool":"arrow", "color":"#ef4444", "size":4,
                 "x1":10,"y1":20,"x2":300,"y2":200, "opacity":1, "head":"end" } ],
    "frame": null }
  ```
  - Rust: `#[serde(default)] version: u32` (missing means 1).
  - Rust: `marks: serde_json::Value` with a custom reader that also accepts the old *string*. Rust can then check it and refuse to edit when `version > SUPPORTED`, returning `readOnly: true` instead of `unwrap_or_default`.
  - TS: `readMarks` accepts a string or an array, then one `normalizeShape()` fills in defaults. That replaces the scattered `!== false` checks.
  - Migration happens lazily on the next save. No batch job is needed.

### 3.5 Testing strategy (proposed)
- **TypeScript:** add vitest. Move the pure geometry out of the components and test it:
  - `fitRatio`, `ratioOf`, `spanned` → a new `lib/crop.ts`;
  - `toSteps`, `bounds`, `hit`, `nearLine`, `onRim`, `handlesOf`;
  - `shift` / `readMarks`, `boxFor` / `clampView`;
  - the move clamp (no negative coordinates, ever).
- **Parity:** a Rust test writes or compares `fixtures/watermark_rects.json`, and vitest asserts `markRect` against the same file. If both blur implementations remain, add a shared expected-blur fixture.
- **Rendering goldens:** Playwright on Chromium renders fixture scenes through `paint` (later `renderScene`) and compares with pixelmatch, with tolerance.
- **Rust:** move a pure `render_edit(original, under, steps, crop) -> Shot` out of commands.rs:539-587. Test it on synthetic images:
  - blur before a layer vs after it;
  - crop applied after marks;
  - watermark position after a crop;
  - an external resize rejected by `under_rect`.
- **Sidecar fixtures** from 0.8.0, 0.8.1 and 0.8.3, run through `read_edit`, `under_rect` and the TS `readMarks`.

---

## 4. Prioritised roadmap (quick wins first)

### Tier 1: quick wins (S, about a day or less each)

1. **Guard every exit against losing unsaved marks.**
   - *Value:* stops silent loss of work.
   - *Sketch:* the header ✕ (ShotViewer 749-758) calls `close` directly. Route it through a `requestClose()` that does `if (editing && unsaved) setAskingDiscard(true) else close()`.
   - Optionally keep a draft of `shapes` per clip id in `sessionStorage` so a crash or update restart can offer "Restore unsaved marks?".
2. **Make save and copy failures (and copy success) visible.**
   - *Value:* today a failed save only resets the button. `lastError` is shown solely on the Settings page (settings/index.tsx:80-84), despite the comment at ShotViewer 640.
   - *Sketch:* in `write_screenshot` and `copy_clip_image`, call `crate::notify(&app, "error", …)` on `Err`, and `notify("ok","Picture copied")` on a successful copy. Alternatively add a client-side toast to `Toasts.tsx`.
3. **Freeze editing while saving.**
   - *Value:* marks drawn during the 0.5-2 s a 4K save takes are overwritten by `resetShapes(readMarks(loaded))` (634).
   - *Sketch:* pass `busy` to `AnnotateLayer` and `CropLayer` (`pointer-events-none`, plus a `BusyRing` overlay), and ignore keys while busy. Or compare `current.current` with the shapes that were sent and keep anything newer.
4. **Fix shortcuts going dead after touching a control.**
   - The global handler skips *every* INPUT, including `type=range` (378).
   - `preventDefault` on pointerdown (718) stops the drawing surface from taking focus, so the slider keeps focus indefinitely.
   - *Sketch:* skip only text-entry fields (`input:not([type=range],[type=checkbox],[type=button])`, `textarea`, `[contenteditable]`). In `onDown`, blur `document.activeElement` when it is not the caption field.
5. **Redo, plus fixing undo grouping.**
   - Add a `future` stack beside `history` (256). `editShapes` clears it; Ctrl+Y and Ctrl+Shift+Z replay it.
   - Restyle gestures are keyed `style-${id}-${keys}` (682). Two separate colour clicks on the same mark therefore merge into one undo step. Add a counter that advances on pointerup/blur of the control.
6. **Keyboard-first operation.**
   - V (select), A (arrow), L (line), R (box), E (ellipse), P (pen), T (text), B (blur), C (crop).
   - Ctrl+S: save, or apply the crop. Ctrl+C: copy the picture. Ctrl+0: fit; Ctrl+1: actual pixels; Ctrl+= / Ctrl+-: zoom (lockdown already cancels the browser's own handling).
   - Arrow keys nudge the selection by 1 px, Shift+arrows by 10 px, as a coalesced gesture `nudge-${id}`.
   - Escape deselects first, then switches to Select, and only then leaves the editor. Today it goes straight to the discard question (385).
   - Show the letter in each tool button's tooltip.
7. **Shift-constrain.** In `onMove` (764-853): arrows and lines snap to 15° steps; box, ellipse and blur become square or round; the crop snaps to 1:1 (`spanned`, 1323-1348).
8. **Remember styles across marks and pictures.**
   - When a mark is selected, `restyle` should also write `styles[chosen.tool]` (676-687). Today the next arrow is red again.
   - Persist styles in `localStorage` as `{color, altColor, sizeFactor: size/baseStroke(width), fill, outline, round}` and rebuild sizes per picture in `defaultStyles` (1488-1509). Wrap storage access in try/catch.
9. **Copy and Save & copy inside the editor, plus copy after capture.**
   - A "Copy" button in the footer, and "Save & copy" next to "Save marks" (`write()` then `api.copyClipImage`).
   - A setting "After a screenshot: copy it to the clipboard": in `take_screenshot_and_notify` (lib.rs:157-197) call `clipboard::copy_image(&shot.dib())` while the `Shot` is still in memory (state.rs:902-917).
10. **More robust clipboard.** In `copy_image` (clipboard.rs:131-135), put CF_DIB, a registered `"PNG"` format and, optionally, CF_HDROP inside the same clipboard session. Modern chat apps prefer PNG, and Explorer gets a file.
11. **Make blur closer to the preview and safer.**
    - `Shot::blur`: three box passes per axis, with `sigmaOf` changed to the 3-pass variance, and a feathered oval mask. Keep the existing tests and add one asserting the 3-pass result stays close to the CSS σ.
    - Add a **Redact** tool: an opaque filled box, black by default, with no opacity control. Change the blur hint (1107) so it doesn't promise unreadability.
12. **New vector tools that need no core change.** Each one is a `paint` case plus `bounds`/`hit`/`handles` plus panel controls, and all travel through today's layer PNG path:
    - **Line / double arrow:** `head?: "end"|"both"|"none"` on arrow. Also fix the tip: stop the shaft at the base of the head so the round cap doesn't poke past it.
    - **Highlighter:** `opacity?: number` → `ctx.globalAlpha`. A "Marker" preset is pen at 0.35 opacity with square caps at 4× thickness. The Rust `blend` already handles alpha.
    - **Numbered steps:** a `step` tool, drawn as a filled circle with a centred number. Store `n` at creation as max+1, with an optional "renumber". Don't compute it inside `paint` (see 3.4).
    - **Spotlight:** cover the whole canvas with alpha 0.5-0.7 black (`ctx.canvas.width/height`), cut the rect/ellipse hole with `evenodd`. One spotlight per picture.
    - **Rounded box:** `radius?` → `ctx.roundRect`.
    - **Text background / label:** `bg?` → a rounded rect behind each line, sized with `measureText`.
    - **Emoji stickers:** a text shape with `rim:false`. Chromium's canvas renders Segoe UI Emoji in colour. Add a small picker.
13. **Show the marks during crop.** Mount read-only `ShapeLayer`s and `BlurPatch`es in crop mode at the original's coordinates (`shift` by +origin). Today the crop ground is the original with no marks (439-441, 880-892).
14. **Small state fixes.**
    - "Save marks" should be disabled when nothing changed. Today the condition is only `shapes.length === 0` (1065), so it rebuilds a 4K file for nothing.
    - The "Edited" pill counts unsaved marks (532). It should reflect what is saved.
    - Stay in draw mode after saving (638). Optionally keep undo across the save.
    - "Remove all marks" (1095-1104) needs a confirmation (reuse `ConfirmDelete`), or make it undoable by keeping the previous `marks` string.
    - "Crop" (1150) and "Undo crop" (1174) silently save unsaved marks too. Say so in the label ("Crop & keep marks") or ask.
    - Touchpad users cannot pan in draw mode (needs a middle button, 502-503). Make the wheel without Ctrl pan and Ctrl or pinch zoom, and add Space+drag.
15. **Measure it.** Add a `Feature::ShotEditor = 6` bit (telemetry.rs:27-34; the server must map it too). Count successful `write_screenshot` calls, as the clip editor does at commands.rs:1020.
16. **Binary IPC for layers.** Send `Uint8Array` as a raw invoke body and read it in Rust through `tauri::ipc::Request`. Change the `ShotStep.layer.png` type.

### Tier 2: medium (M, a few days each)

17. **Export a copy and fit-for-Discord, for screenshots.**
    - *Value:* 4K game PNGs saved with `Compression::Fast` often exceed Discord's 10 MB limit.
    - *Sketch:* a `ShotExportDialog` modelled on `ExportDialog` (size presets 10/25/100 MB, the save dialog plugin):
      - format PNG/JPEG/WebP and a quality setting;
      - scale 100/75/50 %, 1440p, 1080p;
      - "fit under N MB" by binary search on quality, then on scale.
    - Before A: Rust with the `jpeg-encoder` and `fast_image_resize` crates (WebP would need libwebp), or encode in the WebView via `canvas.toBlob(type, q)`.
    - Add "Copy as file (fits Discord)": write a temp JPEG, then CF_HDROP, like the clip side's shrink-and-copy (commands.rs:850-888).
    - Optionally recompress the PNG losslessly (oxipng) in the background after saving.
18. **Unify crop and annotate into one session.**
    - One toolbar, with crop as a tool. History entries become `{shapes, crop}` snapshots.
    - One Save (Ctrl+S) and one Discard. This removes the implicit saves in item 14 and the "which save button?" confusion.
19. **A list of marks ("Layers" panel).**
    - A sidebar list ("Arrow", "Text 'GG'", "Blur").
    - Select, delete, hide, lock and reorder (z-order = order in `shapes`), plus multi-select.
    - This is also the accessible, keyboard-reachable way to get at marks.
20. **Multi-select and arrangement.**
    - Shift-click and drag a selection box with the Select tool; moving a group is one gesture.
    - Ctrl+D duplicates. Copy/paste marks within the editor (Ctrl+C/V when marks are selected; with none selected, Ctrl+C copies the picture).
    - Snap to picture edges, centre and other marks' bounds; Alt turns snapping off.
21. **Pixelate, and later a magnifier.**
    - Before A: a new `Step` kind (`ShotStep`, `commands.rs::Step`) and `Shot::pixelate`/`magnify`.
    - Preview with a small canvas that draws the ground `<img>` plus the layer canvases below it, downscaled with no smoothing. A tainted canvas is fine for display.
    - Under A, each is simply a `renderScene` case.
22. **Better crop.**
    - Edge handles; typed X/Y/W/H fields.
    - Pixel presets: 1920×1080, 1600×900 (X/Twitter), 1080×1080 and 1080×1350 (Instagram), 1280×720 (YouTube thumbnail). A size preset implies a ratio and scale-on-export.
    - A "swap orientation" control.
23. **Rotate 90° and flip.**
    - Add `transform` to the sidecar, applied after the crop; marks stay in the original's coordinates.
    - Preview with a CSS transform on the stage; map pointer positions through the inverse in `at()` (ShotAnnotate 691-699, ShotViewer 1308-1315).
24. **Pen, touch and touchpad.**
    - Use `event.pressure` and smoothing (e.g. perfect-freehand). Store `points` as `[x, y, p]`, which old readers ignore.
    - Pinch-zoom and two-finger pan through pointer events.
    - Larger hit margins for `pointerType === "touch"`.
25. **Accessibility pass.**
    - `role="dialog" aria-modal` with a focus trap and focus restore.
    - Visible focus on the slider; `Segmented` as `radiogroup` with `aria-checked`; named swatches ("Red").
    - Handles become focusable buttons with labels.
    - Keyboard creation: Enter with a tool active places a default-size mark at the view centre, then arrow keys move it.
    - An `aria-live` announcement such as "Arrow selected, 2 of 5".
    - `forced-colors` styles for the active tool state.
26. **Eyedropper.** Use `window.EyeDropper` if WebView2 exposes it (verify). Otherwise sample from an untainted copy of the image (binary IPC). Add a row of recent colours.
27. **Open the editor after capture.** An option, or a banner or console action, that brings the window up with the screenshot open in draw mode. Reuse the existing focus hand-off (Clips.tsx:286-310).
28. **"Make permanent" per screenshot.**
    - For privacy, delete `image.png`, `base.png` and `shot.json`.
    - So the watermark stays protected afterwards, keep the composite's own clean watermark area (crop it from `picture` before `stamp_kept`, commands.rs:585) as a new `under.png`.

### Tier 3: large (L)

29. **The single WebView renderer (option A in 3.3).** Migrate tool by tool. Keep `Shot::blur`/`blend` for one release and remove them after.
30. **OCR.**
    - Windows.Media.Ocr through the `windows` crate (add features `Media_Ocr`, `Graphics_Imaging`, `Storage_Streams`, `Foundation_Collections`).
    - Build a Bgra8 `SoftwareBitmap` from the `Shot`, create the engine with `OcrEngine::TryCreateFromUserProfileLanguages`, and run `RecognizeAsync`. The result has lines and words with bounding rectangles.
    - Ships as "Copy text" (M), then gallery search by stored OCR text: add it to the search text at Clips.tsx:199-205 (L).
31. **Auto-redaction.**
    - OCR words matched against patterns: email, IP, URL, Steam and Discord handles, the Windows user name, friend names from `friends.rs`.
    - Shown as *proposed* Redact or Blur marks that the user accepts, never applied silently.
32. **Beautify.**
    - A "frame" stage after the crop: padding, background (solid, gradient or a blurred copy of the picture), corner radius, shadow.
    - Practical only under A (CSS can preview it, but the file would need a second implementation).
    - Product question: does the watermark go on the picture or on the frame?
33. **Edit any image.** Paste from the clipboard or drop a file to create a screenshot entry. This needs an exception in lockdown.ts's drop blocking and an import command that sets `screenshot: true`.

---

## 5. UX bugs and rough edges

Ordered roughly by severity.

| # | Issue | Where |
|---|---|---|
| 1 | The header ✕ closes without the unsaved check that Escape does. This contradicts the comment at 184-188. | ShotViewer.tsx:749-758 vs 380-386, 580-583 |
| 2 | Save errors are silent. `lastError` only appears on the Settings page; nothing is emitted as a notice. Copy errors are swallowed too (`void`). | ShotViewer.tsx:639-641; store.ts:598-609; settings/index.tsx:80-84; clipMenu.tsx:170 |
| 3 | Marks and undo actions made during a save are overwritten when the saved state is loaded back. | ShotViewer.tsx:604-644, 880-892 (no busy flag passed) |
| 4 | After a slider or hex/RGB field has focus, Delete, Ctrl+Z and Escape are ignored. Drawing on the picture does not take focus away (pointerdown `preventDefault`). | ShotViewer.tsx:378; ShotAnnotate.tsx:713-718 |
| 5 | "Remove all marks" writes the file immediately; the marks are gone for good (history is reset on save). | ShotViewer.tsx:1095-1104, 634 |
| 6 | Marks are invisible while cropping, and "Crop" and "Undo crop" silently save unsaved marks. | ShotViewer.tsx:439-441, 880-893, 1150, 1174 |
| 7 | Escape with a mark selected goes straight to "discard all marks?" instead of deselecting first. | ShotViewer.tsx:379-386 |
| 8 | Restyling the selected mark doesn't become the tool default, so the next mark reverts. | ShotViewer.tsx:676-687 |
| 9 | Separate restyles of the same mark and property merge into one undo step. | ShotViewer.tsx:281-294, 682 |
| 10 | "Save marks" is enabled with nothing changed and rewrites the file. The "Edited" pill counts unsaved marks. | ShotViewer.tsx:1065, 532 |
| 11 | Saving exits draw mode and clears undo; the zoom also resets on every mode change and save. | ShotViewer.tsx:634-638, 372 |
| 12 | No pan in draw mode without a middle button (touchpads). The wheel only zooms and `deltaX` is ignored. | ShotViewer.tsx:496-517, 771-779 |
| 13 | Zoom % is relative to fit, not actual pixels; there is no 1:1 view. | ShotViewer.tsx:932 |
| 14 | Pen marks can't be resized. Text scales only by vertical distance (horizontal pulls do nothing). Clicking an existing caption with the Text tool creates a new one instead of editing. | ShotAnnotate.tsx:457, 818-828, 741-744 |
| 15 | The crop has corner handles only. Picking a tool while cropping silently drops the selection. | ShotViewer.tsx:1396-1405, 585-592 |
| 16 | Copy picture is only reachable by right-click in view mode; there is nothing in the editor. | ShotViewer.tsx:767-770 |
| 17 | If `screenshotEdit` fails, the error is swallowed. A later save then sends `crop: null` with marks positioned on the cropped picture, which drops the crop and puts marks in the wrong place. | ShotViewer.tsx:341-356, 1066 |
| 18 | Possible vertical jump when a caption is put down: the CSS line box uses line-height 1.25, the canvas uses `textBaseline="top"`. Verify in WebView2; fix by positioning both from `fontBoundingBoxAscent`. | ShotAnnotate.tsx:248, 1019-1020 |
| 19 | The thick-arrow round cap pokes past the head's tip. | ShotAnnotate.tsx:214-231 |
| 20 | Gallery tiles are `aspect-video object-cover`, so 1:1 and 9:16 crops look cut off. Screenshot tiles have no "edited" marker. | Clips.tsx:466, 483 |
| 21 | Watermark is re-stamped on edits even after the user has turned it off. This is a product decision to make explicitly; `under.png` would make it easy to honour the setting. | commands.rs:585-587; watermark.rs:144-149 |
| 22 | Accessibility: no dialog semantics or focus trap; handles can't be focused; slider focus is invisible; swatches announce hex codes. | ShotViewer.tsx:709-716, 962-984; Controls.tsx:109-120; ColorPicker.tsx |

Line-level issues for the other reviewer:
- **Hooks after an early return:** `ShotViewer.tsx:421` returns before `useEffect` (453) and `useRef` (462). This would crash if `clip` ever became undefined; Clips.tsx guards against that today.
- **Dead code:** `api.screenshotHasOriginal` (ipc.ts:150-151) has no Rust command.
- **Duplicate constant:** `MAX_ZOOM_STEP` (ShotViewer:60) repeats `MAX_ZOOM` (pictureBox.ts:78).
- **Stale comments:** ShotAnnotate 166-172 and 607-610 still say the export is the preview canvas.

---

## 6. Suggested order of work

1. Tier 1 items 1-4 (lost work and silent failures). About two days in total.
2. Items 5-11: redo, keyboard, Shift-constrain, remembered styles, copy flows, blur parity and the Redact tool. These give the most visible quality jump.
3. Item 12 (line, highlighter, steps, spotlight, emoji). These need no core change.
4. Item 17 (export / fit for Discord) and item 18 (one session for crop and marks).
5. Before adding pixelate, magnifier or beautify: sidecar v2 + binary IPC + vitest/parity tests (items 15-16 and 3.5), then the single WebView renderer (29).
6. OCR "Copy text" next. Search and auto-redaction once the single renderer exists.
