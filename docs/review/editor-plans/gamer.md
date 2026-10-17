# Screenshot editor v2: a gamer-first plan

Baseline: ClippiBoy 0.8.3 (`417f434`). Any statement about how the code behaves today cites the file:line I read. Bug numbers (#n) refer to `reports/01-editor-bugs.md`.

## 0. Starting point and target experience

**Who this is for.** Someone playing borderless or fullscreen at 1440p or 4K, often with two monitors and sometimes HDR. They post to Discord (the console already hard-codes `DISCORD = [10, 25]`, Console.tsx:45), Reddit and X. Before posting they must hide party names, chat, friend codes and server IPs, and they will not Alt-Tab into the app in the middle of a match.

**The path today:**
- **Hotkey.** `take_screenshot_and_notify` (lib.rs:157-197) grabs the frame, stamps it and writes a PNG with `Compression::Fast` (shot.rs:267). It then spawns ffmpeg for the thumbnail *before* returning (state.rs:933, thumbs.rs:51-64), because the banner shows that thumbnail (lib.rs:181-187).
- **Clipboard.** Nothing is copied automatically. The only screenshot settings are the hotkey and the watermark switch (config.rs:49, 64).
- **Console.** It opens only over borderless games; exclusive fullscreen is refused (console.rs:164-172).
  - "Copy" on a screenshot puts the full PNG on the clipboard as CF_HDROP (Console.tsx:708-718).
  - The 10/25 MB shrink buttons are for video only: `!clip.screenshot &&` (Console.tsx:719).
  - The stage is a plain `<img>` (Console.tsx:1307-1312), so nothing can be edited there. "Open in the app" leaves the game (Console.tsx:955-958, commands.rs:834-848).
- **Editor.** It is mouse-only, with no tool keys (ShotViewer.tsx:374-419). Copy exists only in the right-click menu in view mode (ShotViewer.tsx:767-770), and it reports nothing back: `void api.copyClipImage` (clipMenu.tsx:170).

**Target flows (4K, mid-range PC):**

| Flow | What the user does | Budget |
|---|---|---|
| **A. Snap & paste** (also works in exclusive fullscreen) | Press the hotkey. A sound plays and the clipboard holds a file under 10 MB plus the picture. Paste in Discord. | File on disk ≤ 400 ms p95 after the first frame; clipboard ready ≤ +700 ms; no UI at all |
| **B. Redact in the console** | Console hotkey → `E` (edit newest shot) → `R` and drag over names → `Enter` (save, fit, copy) → `Esc` | Interactive ≤ 300 ms; Enter → clipboard ≤ 1 s p95; whole loop ≤ 10 s |
| **C. Template** | Per-game zones are redacted at capture time | Flow A's output is already redacted |
| **D. Full edit in the app** | Keyboard-first; Ctrl+Enter = save & copy | Drag at 60 fps |

## 1. Goals, non-goals, guarantees, success criteria

**Goals**
- Redaction that can be trusted (R1-R8 below), failing closed: when in doubt, nothing is written.
- A Discord-ready clipboard within about 1.1 s of the hotkey, with no UI. Size-fit for Discord, X and Reddit.
- Quick edit inside the console, keyboard-first, with no Alt-Tab.
- Per-game redaction templates applied at capture.
- Correct results on 4K/8K, multi-monitor and HDR.
- Fix the save-path defects from report 01 before adding new features.

**Non-goals for v2**
- **A region-select overlay at capture.** It would have to draw over exclusive-fullscreen games, which no window of ours can (console.rs:22-25). Cropping stays in the editor.
- **Report 02's single WebView renderer (option A).** Reasons:
  - Capture-time templates need headless redaction in Rust.
  - The process that writes the file should be the one enforcing the guarantee.
  - The console's WebView runs with `--disable-gpu` (overlay.rs:53-58).
  - Revisit after the M3 measurements.
- OCR auto-redaction, beautify, stickers, numbered steps.
- HDR output files (output stays 8-bit sRGB).
- Gamepad editing.

**What must be true of a "redacted" screenshot**
- **R1 Strength.**
  - *Redact* (solid fill): every covered pixel has exactly the fill colour.
  - *Blur*: the file's σ is at least the preview's σ, and at least `min(w,h)/4`. Test: a sinusoid whose period lies between 4 px and `2·min(w,h)` keeps ≤ 2 % of its contrast.
  - *Pixelate* is never offered as redaction, because block means can be inverted against a known font.
- **R2 Coverage.** The rectangle in the file contains the preview's rectangle.
  - Today all four edges use `Math.round` (ShotAnnotate.tsx:514-517). Floor the near edges and ceil the far ones instead.
  - Today the oval uses a pixel-centre test (shot.rs:199-208). Write every pixel the antialiased preview touches.
- **R3 Completeness.** Rust refuses to write if a redaction mark stored in `marks` has no identical step. Today marks are opaque to Rust (shot.rs:316-319) and the steps arrive separately (commands.rs:568-581).
- **R4 Derived copies follow the file.**
  - The thumbnail is rebuilt or deleted. Today a failed rebuild keeps the old one (commands.rs:641-643).
  - Fitted share copies are purged.
  - The console stage is cache-busted. Today it loads `fileUrl(clip.path)` with no `?v=` (Console.tsx:1309), unlike the main viewer (ShotViewer.tsx:449). With `console_in_capture` switched on (model.rs:487-491), a stale *unredacted* picture can show up on stream.
- **R5 Atomic and honest.** There is never a state where the UI says "saved" and the file says otherwise. Every failure produces a toast.
- **R6 No hidden data in shared files.** `write_png` writes no text chunks (shot.rs:251-279); pin that with a test. Sidecars never travel.
- **R7 Unredacted pixels stay home.** They exist only in `originals/<id>/{image,under}.png` (shot.rs:287-289, 365-367). They never leave the machine and can be purged.
- **R8 Copy or send of a shot with unsaved redactions saves first.** The stale file is never copied.

**Success criteria**
1. **Capture.** Hotkey → file ≤ 400 ms p95 at 3840×2160 (not counting the wait for WGC's first frame). The fitted copy adds ≤ 700 ms p95. No ffmpeg process runs on the screenshot path.
2. **Size.** "Copy for Discord" stays under the chosen limit for 100 % of inputs up to 7680×4320, in ≤ 1.5 s p95 at 4K. The lossless PNG is used whenever it already fits.
3. **Interaction.**
   - Dragging a mark on a 4K shot with 10 blurs: ≥ 60 fps (p95 frame time ≤ 16.7 ms) in the main window, ≥ 30 fps in the console.
   - Preview memory ≤ 120 MB extra. Today every layer run is a full-resolution canvas, roughly (N+1) × 33 MB (ShotAnnotate.tsx:566-586).
4. **Save.**
   - Enter → clipboard ≤ 1 s p95 at 4K, and the Rust peak memory is ≤ 4× the RGB frame.
   - A fault-injection suite (target locked, disk full, thumbnail fails, crash between steps) never leaves the sidecars ahead of the file. It never leaves an unredacted derived copy while the UI reports success.
5. **Parity.**
   - Vector marks: mean |Δ| between file and preview ≤ 1/255.
   - R1-R3 pass in CI for every value the slider can take (`range`, ShotAnnotate.tsx:119-129).

## 2. Milestones

### M0: Trustworthy save and redaction (7 d)

**Outcome:** redactions you can post, saves that cannot half-happen or land on the wrong picture, and errors you can see.

**Tasks**
- **`Shot::blur`** (shot.rs:139-210):
  - Three box passes with σ rounded up, and a margin of 3r.
  - Apply the R1 floor, and the coverage-safe oval from R2.
  - Signed coordinates. A rectangle wholly outside the picture is rejected instead of becoming a 1-px strip (#24, shot.rs:148-151).
  - New `Shot::fill` for Redact.
  - Fix the comment at 129-134.
- **`Step`** (commands.rs:400-419): signed x/y and a new `Redact` variant. Run `check_redactions` before anything is decoded (sketch 1).
- **ShotAnnotate.tsx:**
  - Add a `redact` tool: an entry in TOOLS (16-24); `paint` skips it like blur (263-266); the preview is a solid div.
  - `toSteps` (491-525) rounds outward, then applies the < 2 px slip rule.
  - Rewrite the hint at ShotViewer.tsx:1107 ("Redact hides for sure; Blur obscures").
- **`edit_screenshot`** (commands.rs:511-599):
  - Stage every write as a uuid-named `.part`. Swap the main PNG in first, then rename the sidecars (sketch 2).
  - Do the same for `keep_original` (606-620) and `write_under` (shot.rs:385-400).
  - Skip re-encoding `base.png` when the crop is unchanged (#27).
  - Sweep leftover `*.part` and `*.new.png` files at startup (#18).
- **Per-shot lock** in `AppState`, held by `write_screenshot`, `file_clip` (commands.rs:745-769) and `delete_clip` (676-705). The path is re-read under the lock (#3).
- **`write_screenshot` returns `{clip, edit}`.** It becomes `async` with `spawn_blocking` (#15) and calls `notify` on error, like `store` does (commands.rs:1317-1322).
- **ShotViewer:**
  - An epoch guard on the save continuation (#5).
  - Input frozen while busy (#6).
  - Tools and Save gated on a loaded `edit`, and load errors shown instead of `.catch(() => {})` (#4, 341-356, 956).
  - The header ✕ goes through the unsaved-marks check (749-751).
  - Close and Delete are disabled while busy.
  - Toasts for copy failures (#16, clipMenu.tsx:170).
- **Thumbnail failure:** delete the old thumbnail and `set_thumb(None)` (#10, minimal fix).
- **Input hardening:**
  - PNG caps in `read_png`/`read_rgba` (#7, shot.rs:468, 685): ≤ 16384 per side, ≤ 2²⁷ pixels, and `try_reserve_exact`.
  - `size_of_png` when there is no original yet (#11, commands.rs:457-461).
  - Reject screenshots received from friends that are not PNGs (share.rs:1187-1188).
- **Console.tsx:1309:** add `?v=${clip.sizeBytes}` (R4).
- **Timing logs** in `take_screenshot` and `edit_screenshot`, as the baseline for criteria 1 and 4.

**IPC:** `ShotStep` gains `redact` and signed coordinates (types.ts:525-527). `writeScreenshot` returns `ShotSaved` (ipc.ts:144-148, store.ts:598-609).

**Tests**
- **Rust:**
  - Blur MTF residual for R ∈ {4, 8, 16, 48, 96, 192}.
  - Property test: file σ ≥ preview σ for every R from 1 to 192.
  - Coverage against a reference rasteriser.
  - A mark/step mismatch is refused.
  - Commit fault injection through an injectable `replace` function.
  - A PNG bomb fixture (IHDR 60000×60000) is rejected before any allocation.
  - The output carries no tEXt, iTXt, zTXt or eXIf chunk.
- **TypeScript:** add vitest (there is no test runner today). Shared fixtures for outward rounding and for `markRect`.

**Risks**
- Old edits look stronger when re-saved. This is intended; say so in the release notes.
- The fail-closed check could trip on old marks. The probe reads only `tool`, `x1`…`y2` and `round`.
- serde_json needs `float_roundtrip`, so floor/ceil give the same answer in TS and Rust.

### M1: Hotkey to Discord in about a second (6 d)

**Outcome:** flow A works. Console screenshot tiles get the 10/25 MB buttons. The editor gets Copy (Ctrl+C) and Save & copy (Ctrl+Enter).

**Tasks**
- **New `shot_share.rs`:**
  - `fit()` (sketch 3): use the lossless PNG if it fits; otherwise a binary search on JPEG quality, then a ladder of downscales.
  - Output goes to `%APPDATA%/ClippiBoy/share/<id>/<stem>.jpg`, not next to the shot as video exports do (commands.rs:871).
  - Purged on every edit and on delete.
- **Dependency:** add `jpeg-encoder`. It serves the thumbnails as well.
- **clipboard.rs:**
  - `copy_share`: CF_HDROP, CF_DIB and a registered "PNG" format in one clipboard session. Today every call empties the clipboard first (121-123, 132-134).
  - `read_text` bounded by `GlobalSize` (#22).
- **New command:** `copy_screenshot_for_share(id, megabytes)`, run in `spawn_blocking`.
- **`take_screenshot`** (state.rs:890-964) returns `(Clip, Shot)`:
  - The thumbnail is made in-process with `thumbs::still_from(&Shot)`, an area downscale to fit 480×480. This fixes #10 and #23.
  - It drops ffmpeg from both capture and save (state.rs:933, commands.rs:641).
  - The after-capture action runs off the banner path.
- **Config:** `screenshot_after: { copy: Off | Picture | Fitted, megabytes }` with serde defaults (model.rs:458-461 explains why a missing default costs the whole config), plus the settings UI.
- **Console.tsx:719:** drop the `!clip.screenshot` guard. Add the same buttons to the console stage.
- **Feedback:** a distinct "copied" sound for exclusive fullscreen, where neither the console nor the banner draws. Count fitted copies in telemetry (telemetry.rs:198-200 counts shots only).

**Tests**
- `fit()` stays under the limit on synthetic noise, gradient and text frames at 1080p, 4K and 8K, against limits of 5/8/10/25 MB.
- Thumbnails of 16×4000 and 8000×16.
- Unit tests for the DROPFILES and PNG payload builders.

**Risks**
- Which format Discord pastes when HDROP and an image are both present: verify, and fall back to HDROP only.
- 4:2:0 chroma smears small red HUD text: use 4:4:4 at quality ≥ 85.

### M2: Keyboard-first (5 d)

**Outcome:** everything except the drawing itself works without the mouse.

**Tasks**
- **Move the key handler** (ShotViewer.tsx:374-419) into a pure `keyAction(state, event)` in `lib/shotKeys.ts`.
- **Keys:**
  - Tools: V, A, R, B, E, P, T, C.
  - Shift constrains (`onMove`, ShotAnnotate.tsx:764-853). Arrow keys nudge by 1 px, Shift+arrows by 10 px.
  - Escape works down a ladder: text field → selection → tool → the discard question.
  - Ctrl+Y and Ctrl+Shift+Z redo. Ctrl+S saves. Ctrl+0/1/=/- zoom; lockdown already swallows the browser's own handling of these (lockdown.ts:29-47). Enter applies a crop.
- **Focus fix.** Only text-entry fields swallow keys. Today any INPUT does, sliders included (ShotViewer.tsx:378). Pointerdown drops stray focus (ShotAnnotate.tsx:718).
- **Undo:**
  - A module-level gesture counter, and a token per restyle interaction (#17; ShotAnnotate.tsx:638, ShotViewer.tsx:682).
  - Handle `pointercancel` and lost capture, and ignore moves with `buttons === 0` (#12). Alt+Tab in the middle of a drag is normal behaviour for a gamer.
- **Remembered styles.** Store each tool's size as `size / baseStroke` in localStorage, wrapped in try/catch. Restyling the selected mark also sets the tool's default (ShotViewer.tsx:676-687).
- **Small fixes.** The blur hit margin becomes `slack` (#20, ShotAnnotate.tsx:372). The slider steps from its minimum (#28). "Remove all marks" asks first (ShotViewer.tsx:1095-1104).

**Tests:** vitest for `keyAction`, undo/redo and the constrain maths.

### M3: Edit without leaving the game (9 d)

**Outcome:** flow B works in the console.

**Tasks**
- **Split ShotViewer** (#26) into:
  - `useShotEdit(id)` (sketch 2): load, save, epoch, busy, undo;
  - `ShotStage`: image, under-piece, layers, watermark preview (ShotViewer.tsx:839-935);
  - `CropLayer.tsx`;
  - `lib/layers.ts`.
  Move the hooks above the early return (#19). Delete the dead `screenshotHasOriginal` (ipc.ts:150-151).
- **Preview at display resolution** (#9): canvases sized to `round(box.width·dpr)` with `setTransform`; the dragged mark lifted into the draft layer; a memoised side panel. This is mandatory because the console has no GPU (overlay.rs:57).
- **Binary layers** (#8): a raw-body `write_shot_layer` per layer; `write_screenshot` refers to the layers by index. PNG encoding moves into a Worker using OffscreenCanvas.
- **Console `QuickEdit`.** Replace the screenshot branch of `ClipStage` (Console.tsx:1307) with:
  - Redact, Blur, Arrow, Text, Crop (free and 16:9), and Undo, using the M2 keys;
  - `Enter` = save, fit, copy;
  - `E` on the console's first screen = edit the newest shot.
- **Draft survival.** The console hides and resets itself on any loss of focus (console.rs:90-103). Keep unsaved shapes per clip id in a module-level map and offer "Restore marks" when it reopens.

**Tests**
- vitest + jsdom for the race guards in the hook.
- A dev-only `?bench` page that drags a mark on a 4K fixture and records frame times, run on Windows for criterion 3.

**Risks**
- `backdrop-filter` may be too slow in software. Fallback: preview the blur through `ctx.filter` on a display-size canvas. A tainted canvas is fine for display; reading pixels back is blocked anyway by the CSP's `default-src 'self'` (tauri.conf.json:31).
- The console shares a browser process with the banner (overlay.rs:60-63), so the GPU cannot be switched on for it alone.

### M4: Per-game redaction templates and privacy hygiene (6 d)

**Outcome:** "Always redact these zones in <game>", and a way to flatten a shot's history.

**Tasks**
- **Sidecar v2** (shot.rs:311-320):
  - a `version` field;
  - `marks: Vec<serde_json::Value>`, still accepting the old string;
  - a `written {size, mtime}` stamp to detect external edits (#14);
  - `readOnly` when the version is newer than supported, instead of `unwrap_or_default` (335-340).
- **Templates.** A `RedactZone {kind, x, y, w, h in 0..1, ellipse, aspect}` list per game name (the same key as `current_game`, state.rs:909). The editor gets "Remember these redactions for <game>".
- **Capture with templates.**
  - Factor `render_edit(original, under, steps, crop)` out of commands.rs:539-587.
  - At capture, copy the stamped picture to `originals/<id>/image.png` first. Then write the clip-folder file as `render_edit` with the zones as steps, and write typed redaction marks to `shot.json`.
  - The user's folder therefore never holds the unredacted frame. A shot whose aspect differs from the template is skipped, with a banner saying so.
- **Flatten** (#13):
  - Per shot, "Make permanent" deletes image.png, base.png and shot.json, and saves the composite's clean corner as the new `under.png`.
  - Globally, a "discard screenshot originals" action beside the existing one for trims (commands.rs:1241-1249).

**Tests**
- v1 fixtures from 0.8.0, 0.8.1 and 0.8.3 still read.
- A future version opens read-only.
- A capture with zones gives a redacted file and an unredacted original, and its marks open in the TS editor.
- After flattening, the watermark can still be re-stamped.

### M5: Big screens: HDR, multi-monitor, 8K (7 d)

**Tasks**
- **HDR.** The frame pool is created as `B8G8R8A8UIntNormalized` (wgc.rs:191, 235), and `read_back` assumes 4 bytes per pixel (shot.rs:596-615). HDR highlights probably clip or wash out; verify on hardware.
  - When the display is in scRGB, grab `R16G16B16A16Float` and tone-map to sRGB using the SDR white level.
  - This is for screenshots only; recording stays as it is.
- **Monitor choice.** Today a screenshot follows the recording target (state.rs:892-906). Add "the screen the game is on", via `MonitorFromWindow(GetForegroundWindow())` and the existing `CreateForMonitor` (wgc.rs:144-146).
- **Watermark** (#21): bilinear upscaling in `scale` (watermark.rs:179-205).
- **8K:** a blur that copies only its patch, and PNG encoding at 8K under 1.5 s.

**Tests:** tone-mapping of synthetic FP16 values, watermark upscale smoothness, an 8K memory test.

**Risks:** HDR behaviour varies with driver, OS and game.

### Where each report-01 defect is handled

| Milestone | Defects |
|---|---|
| M0 | #1-#7, #10 (minimal), #11, #15 (shot commands), #16, #18, #24, #27 |
| M1 | #10 (full), #22, #23 |
| M2 | #12, #17, #20, #28 |
| M3 | #8, #9, #19, #25 (dead code), #26 |
| M4 | #13, #14 |
| M5 | #21 |

Deferred:
- **OCR auto-redaction.** `Windows.Media.Ocr` is weak on stylised game fonts, it may only ever *propose* marks, and it needs M4's typed marks first.
- **Option A renderer.** See the non-goals.

Total: about 40 developer-days.

## 3. Code sketches

### Sketch 1: the redaction core (shot.rs, commands.rs)

```rust
/// Half-width of each of three box passes with combined variance ≥ σ².
/// One pass of half-width r has variance r(r+1)/3, three have r(r+1).
/// Rounded up: the file may blur harder than the preview, never softer (R1).
pub fn three_pass_radius(sigma: f32) -> usize {
    let r = ((1.0 + 4.0 * sigma * sigma).sqrt() - 1.0) / 2.0;
    (r.ceil() as usize).max(1)
}

/// The preview's σ (`sigmaOf`, ShotAnnotate.tsx:1079), floored so that the
/// shape of a word does not survive either.
pub fn redaction_sigma(radius: u32, width: u32, height: u32) -> f32 {
    let r = radius.max(1) as f32;
    (r * (r + 1.0) / 3.0).sqrt().max(width.min(height) as f32 / 4.0)
}

impl Shot {
    /// A signed rectangle clipped to the picture; wholly outside is an error.
    pub fn inside(&self, x: i64, y: i64, w: u32, h: u32) -> Result<Rect, String> {
        let (x0, y0) = (x.max(0), y.max(0));
        let x1 = (x + w as i64).min(self.width as i64);
        let y1 = (y + h as i64).min(self.height as i64);
        if x1 <= x0 || y1 <= y0 {
            return Err("A redaction lies outside the picture.".into());
        }
        Ok(Rect { x: x0 as u32, y: y0 as u32, width: (x1 - x0) as u32, height: (y1 - y0) as u32 })
    }

    pub fn blur(&mut self, x: i64, y: i64, w: u32, h: u32, sigma: f32, ellipse: bool)
        -> Result<(), String>
    {
        let at = self.inside(x, y, w, h)?;
        let r = three_pass_radius(sigma);
        let margin = 3 * r; // three passes reach three radii
        let width = self.width as usize;
        let from_x = (at.x as usize).saturating_sub(margin);
        let from_y = (at.y as usize).saturating_sub(margin);
        let pw = (at.x as usize + at.width as usize + margin).min(width) - from_x;
        let ph = (at.y as usize + at.height as usize + margin).min(self.height as usize) - from_y;

        let mut a = vec![0u8; pw * ph * 3];
        for row in 0..ph {
            let src = ((from_y + row) * width + from_x) * 3;
            a[row * pw * 3..(row + 1) * pw * 3].copy_from_slice(&self.rgb[src..src + pw * 3]);
        }
        let mut b = vec![0u8; a.len()];
        for _ in 0..3 { blur_rows(&a, &mut b, pw, ph, r); std::mem::swap(&mut a, &mut b); }
        transpose(&a, &mut b, pw, ph);
        std::mem::swap(&mut a, &mut b);
        for _ in 0..3 { blur_rows(&a, &mut b, ph, pw, r); std::mem::swap(&mut a, &mut b); }
        transpose(&a, &mut b, ph, pw); // `b`: the blurred patch, pw × ph

        self.write_back(x, y, w, h, at, ellipse, |col, row| {
            let from = ((row - from_y) * pw + (col - from_x)) * 3;
            [b[from], b[from + 1], b[from + 2]]
        });
        Ok(())
    }

    /// Redact: every covered pixel becomes exactly `rgb`.
    pub fn fill(&mut self, x: i64, y: i64, w: u32, h: u32, ellipse: bool, rgb: [u8; 3])
        -> Result<(), String>
    {
        let at = self.inside(x, y, w, h)?;
        self.write_back(x, y, w, h, at, ellipse, |_, _| rgb);
        Ok(())
    }

    /// The oval of the rectangle as *asked*, grown by half a pixel so that every
    /// pixel the antialiased preview touches is written (R2).
    fn write_back(&mut self, x: i64, y: i64, w: u32, h: u32, at: Rect, ellipse: bool,
                  pixel: impl Fn(usize, usize) -> [u8; 3]) {
        let (cx, cy) = (x as f32 + w as f32 / 2.0, y as f32 + h as f32 / 2.0);
        let (rx, ry) = (w as f32 / 2.0 + 0.5, h as f32 / 2.0 + 0.5);
        for row in at.y as usize..(at.y + at.height) as usize {
            for col in at.x as usize..(at.x + at.width) as usize {
                if ellipse {
                    let (dx, dy) = ((col as f32 + 0.5 - cx) / rx, (row as f32 + 0.5 - cy) / ry);
                    if dx * dx + dy * dy > 1.0 { continue; }
                }
                let to = (row * self.width as usize + col) * 3;
                self.rgb[to..to + 3].copy_from_slice(&pixel(col, row));
            }
        }
    }
}
```

```rust
// commands.rs: Step gains signed coordinates and Redact; R3 is checked first.
#[derive(serde::Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Step {
    Blur { x: i64, y: i64, width: u32, height: u32, radius: u32, #[serde(default)] ellipse: bool },
    Redact { x: i64, y: i64, width: u32, height: u32, #[serde(default)] ellipse: bool, color: [u8; 3] },
    Layer { png: Vec<u8> },
}

/// Just enough of a stored mark to know where a redaction is.
#[derive(serde::Deserialize)]
struct Probe { tool: String, x1: f64, y1: f64, x2: f64, y2: f64, #[serde(default)] round: bool }

/// Outward, exactly as `toSteps`: floor near edges, ceil far ones, then the
/// < 2 px slip rule (ShotAnnotate.tsx:510). Needs serde_json `float_roundtrip`.
fn outward(p: &Probe) -> Option<(i64, i64, u32, u32)> {
    let (l, t) = (p.x1.min(p.x2).floor(), p.y1.min(p.y2).floor());
    let (w, h) = (p.x1.max(p.x2).ceil() - l, p.y1.max(p.y2).ceil() - t);
    (w >= 2.0 && h >= 2.0).then(|| (l as i64, t as i64, w as u32, h as u32))
}

/// R3: every stored redaction is a step that gets painted. Runs before a
/// single pixel is decoded; a mismatch writes nothing.
fn check_redactions(marks: &str, steps: &[Step]) -> Result<()> {
    let probes: Vec<Probe> = if marks.is_empty() { Vec::new() } else {
        serde_json::from_str(marks)
            .map_err(|_| "The marks could not be read. Nothing was written.".to_string())?
    };
    let mut wanted: Vec<_> = probes.iter()
        .filter(|p| p.tool == "blur" || p.tool == "redact")
        .filter_map(|p| outward(p).map(|r| (p.tool == "redact", r, p.round)))
        .collect();
    let mut got: Vec<_> = steps.iter().filter_map(|s| match s {
        Step::Blur { x, y, width, height, ellipse, .. } => Some((false, (*x, *y, *width, *height), *ellipse)),
        Step::Redact { x, y, width, height, ellipse, .. } => Some((true, (*x, *y, *width, *height), *ellipse)),
        Step::Layer { .. } => None,
    }).collect();
    wanted.sort_unstable();
    got.sort_unstable();
    if wanted != got {
        return Err("A redaction would have been left out of the picture. Nothing was written.".into());
    }
    Ok(())
}
```

### Sketch 2: one writer per shot, file first, continuation guarded

```rust
// state.rs: new fields `shots_busy: Mutex<HashSet<String>>`, `shots_idle: parking_lot::Condvar`
pub struct ShotLock<'a> { state: &'a AppState, id: String }

impl AppState {
    /// One writer per screenshot; filing and deleting wait their turn.
    pub fn lock_shot(&self, id: &str) -> Result<ShotLock<'_>, String> {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        let mut busy = self.shots_busy.lock();
        while busy.contains(id) {
            if self.shots_idle.wait_until(&mut busy, deadline).timed_out() {
                return Err("The screenshot is still being saved.".into());
            }
        }
        busy.insert(id.to_owned());
        Ok(ShotLock { state: self, id: id.to_owned() })
    }
}
impl Drop for ShotLock<'_> {
    fn drop(&mut self) {
        self.state.shots_busy.lock().remove(&self.id);
        self.state.shots_idle.notify_all();
    }
}

// commands.rs
fn part_of(target: &Path) -> PathBuf {
    let name = target.file_name().unwrap_or_default().to_string_lossy();
    target.with_file_name(format!(".{name}.{}.part", uuid::Uuid::new_v4().simple()))
}

/// Everything a save changes, staged beside its target. `None` removes it.
struct Commit { main: (PathBuf, PathBuf), side: Vec<(Option<PathBuf>, PathBuf)> }

impl Commit {
    fn abort(&self) {
        let _ = std::fs::remove_file(&self.main.0);
        for (part, _) in &self.side { if let Some(p) = part { let _ = std::fs::remove_file(p); } }
    }
    /// The picture first: that is the swap that fails in practice (a viewer
    /// holds it open). Only then the notes that describe it (R5).
    fn apply(self) -> Result<()> {
        if let Err(err) = crate::muxer::replace_file(&self.main.0, &self.main.1) {
            self.abort();
            return Err(err);
        }
        for (part, target) in &self.side {
            let done = match part {
                Some(p) => std::fs::rename(p, target),
                None => std::fs::remove_file(target).or_else(|e| {
                    if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) }
                }),
            };
            done.map_err(|e| format!("The picture is saved but its edit note is not ({e})."))?;
        }
        Ok(())
    }
}

#[derive(serde::Serialize)]
pub struct ShotSaved { clip: Clip, edit: ShotEdit }

#[tauri::command]
pub async fn write_screenshot(app: tauri::AppHandle, id: String, crop: Option<Rect>,
                              steps: Vec<Step>, marks: String) -> Result<ShotSaved> {
    let worker = app.clone();
    let saved = tauri::async_runtime::spawn_blocking(move || {
        let state = worker.state::<AppState>();
        check_redactions(&marks, &steps)?;           // R3, before any decode
        let _lock = state.lock_shot(&id)?;           // #3: no file_clip/delete in between
        edit_screenshot(&state, &id, crop, &steps, marks) // stages into a Commit, applies it,
                                                     // purges share/<id>, rebuilds the thumbnail
    }).await.map_err(|e| e.to_string())?;
    match saved {
        Ok(saved) => { announce_change(&app); Ok(saved) }
        Err(err) => { crate::notify(&app, "error", err.clone()); Err(err) } // #16
    }
}
```

```ts
// lib/useShotEdit.ts, extracted from ShotViewer.tsx:326-363 and 604-644
export function useShotEdit(id: string | undefined) {
  const [edit, setEdit] = useState<ShotEdit | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  /** Bumped per picture: a continuation that belongs to an older one is dropped. */
  const epoch = useRef(0);

  useEffect(() => {
    const mine = ++epoch.current;
    setEdit(null); setError(null); setBusy(false);
    if (!id || !inTauri) return;
    api.screenshotEdit(id).then(
      (loaded) => { if (epoch.current === mine) setEdit(loaded); },
      (err) => { if (epoch.current === mine) setError(String(err)); }, // #4: shown, not swallowed
    );
  }, [id]);

  const save = useCallback(
    async (crop: ShotRect | null, steps: ShotStep[], marks: string) => {
      if (!id || !edit || busy) return null; // #4: never on a guessed ground
      const mine = epoch.current;
      setBusy(true); // the layers read this and stop taking input (#6)
      try {
        const saved = await api.writeScreenshot(id, crop, steps, marks);
        if (epoch.current !== mine) return null; // #5: another picture is shown now
        setEdit(saved.edit);
        useEngine.getState().replaceClip(saved.clip); // new store action
        return saved;
      } catch {
        return null; // the core has already raised the toast
      } finally {
        if (epoch.current === mine) setBusy(false);
      }
    },
    [id, edit, busy],
  );

  return { edit, ready: edit !== null, error, busy, save };
}
```

### Sketch 3: fit for Discord, one clipboard session, after-capture copy

```rust
// shot_share.rs (new)
pub fn dir(id: &str) -> PathBuf { crate::config::data_dir().join("share").join(crate::muxer::sanitize(id)) }
/// Fitted copies show the picture as it was; after an edit they would still
/// carry what was just redacted away (R4).
pub fn forget(id: &str) { let _ = std::fs::remove_dir_all(dir(id)); }

pub struct Fitted { pub path: PathBuf, pub bytes: u64, pub quality: Option<u8>, pub picture: Option<Shot> }

pub fn fit(clip: &Clip, shot: &Shot, max_bytes: u64) -> Result<Fitted, String> {
    let source = Path::new(&clip.path);
    if std::fs::metadata(source).map(|m| m.len() <= max_bytes).unwrap_or(false) {
        // Lossless when it already fits, which most 1080p shots do.
        let bytes = std::fs::metadata(source).map(|m| m.len()).unwrap_or(0);
        return Ok(Fitted { path: source.into(), bytes, quality: None, picture: None });
    }
    let stem = source.file_stem().map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "screenshot".into());
    std::fs::create_dir_all(dir(&clip.id)).map_err(|e| e.to_string())?;
    for scale in [1.0f32, 0.75, 0.5, 0.375] {
        let scaled = (scale < 1.0).then(|| shot.resized(
            (shot.width as f32 * scale) as u32, (shot.height as f32 * scale) as u32)); // area average
        let src = scaled.as_ref().unwrap_or(shot);
        let (mut low, mut high, mut best) = (55u8, 92u8, None);
        while low <= high { // highest quality under the limit
            let quality = low + (high - low) / 2;
            let bytes = jpeg(src, quality)?;
            if bytes.len() as u64 <= max_bytes { best = Some((quality, bytes)); low = quality + 1; }
            else { high = quality - 1; } // quality ≥ 55: no underflow
        }
        if let Some((quality, bytes)) = best {
            let path = dir(&clip.id).join(format!("{stem}.jpg"));
            let part = path.with_extension("jpg.part");
            std::fs::write(&part, &bytes).and_then(|_| std::fs::rename(&part, &path))
                .map_err(|e| e.to_string())?;
            return Ok(Fitted { path, bytes: bytes.len() as u64, quality: Some(quality), picture: scaled });
        }
    }
    Err(format!("Even at 37 % of its size the picture is over {} MB.", max_bytes >> 20))
}

fn jpeg(shot: &Shot, quality: u8) -> Result<Vec<u8>, String> {
    let (Ok(w), Ok(h)) = (u16::try_from(shot.width), u16::try_from(shot.height)) else {
        return Err("The picture is too large for a JPEG.".into());
    };
    let mut out = Vec::with_capacity(shot.rgb.len() / 6);
    let mut encoder = jpeg_encoder::Encoder::new(&mut out, quality);
    if quality >= 85 { // keeps small red HUD text from smearing
        encoder.set_sampling_factor(jpeg_encoder::SamplingFactor::R_4_4_4);
    }
    encoder.encode(&shot.rgb, w, h, jpeg_encoder::ColorType::Rgb).map_err(|e| e.to_string())?;
    Ok(out)
}

// clipboard.rs, in `mod win`: file, picture and PNG in one session. Each
// `copy_*` there empties the clipboard first, so two calls kept only the last.
pub fn copy_share(file: &Path, dib: &[u8], png: Option<&[u8]>) -> Result<(), String> {
    let drop_list = drop_files(&[file]); // the DROPFILES block of copy_files, factored out
    let _clipboard = Clipboard::open()?;
    unsafe { EmptyClipboard() }.map_err(|err| format!("empty clipboard: {err}"))?;
    put(CF_HDROP, &drop_list)?; // Discord and Explorer attach the fitted file
    put(CF_DIB, dib)?;          // from the fitted pixels, so a re-encode stays small
    if let Some(png) = png {
        let format = unsafe { RegisterClipboardFormatW(windows::core::w!("PNG")) };
        if format != 0 { put(format, png)?; }
    }
    Ok(())
}

// lib.rs, take_screenshot_and_notify, once the clip is indexed and the banner is
// up (lib.rs:169-187). state.take_screenshot now returns (Clip, Shot).
let after = state.config_snapshot().screenshot_after;
if after.copy != ShotCopy::Off {
    let (app, clip) = (app.clone(), clip.clone());
    std::thread::spawn(move || {
        let done = match after.copy {
            ShotCopy::Picture => clipboard::copy_image(&shot.dib()),
            _ => shot_share::fit(&clip, &shot, after.megabytes << 20).and_then(|f| {
                let pixels = f.picture.as_ref().unwrap_or(&shot);
                let png = f.quality.is_none().then(|| std::fs::read(&f.path).ok()).flatten();
                clipboard::copy_share(&f.path, &pixels.dib(), png.as_deref())
            }),
        };
        match done {
            // In exclusive fullscreen the sound is the only signal there is.
            Ok(()) => sounds::play(&app, SoundKind::Copied),
            Err(err) => notify(&app, "error", format!("Not copied: {err}")),
        }
    });
}
```

```tsx
// Console.tsx:719: screenshots get the same two buttons
{DISCORD.map((mb) => (
  <Tool key={mb} label={`Fit to ${mb} MB and copy`} busy={busy === `discord-${clip.id}-${mb}`}
    onClick={() => run(`discord-${clip.id}-${mb}`, async () => {
      if (clip.screenshot) {
        const fitted = await api.copyScreenshotForShare(clip.id, mb);
        say(fitted.quality === null ? "Copied as PNG" : `${formatSize(fitted.bytes)} JPEG · copied`);
      } else {
        await api.exportForDiscord(clip.id, mb);
        say(`${mb} MB · copied`);
      }
    })}>
    <span className="text-[11px] font-bold">{mb}</span>
  </Tool>
))}
```

## 4. Open questions for the maintainer

1. **Blur floor.** Is σ ≥ `min(w,h)/4` right? Or should Redact be the only thing called redaction, with Blur purely cosmetic? Which tool does `B` pick by default?
2. **Size presets.** Keep `[10, 25]` (Console.tsx:45), or make it a setting (Nitro tiers) and add X and Reddit presets? Their current limits need verifying.
3. **Where fitted copies live.** In the APPDATA share cache (proposed), or beside the shot as video exports do (commands.rs:871)?
4. **Templates.** Write the redacted file into the clip folder at capture time (proposed), or only propose the marks in the editor?
5. **Originals.** Keep them forever as today, or auto-flatten after N days or on first share?
6. **Watermark when the setting is off.** Today an edit re-stamps it whenever `under.png` exists (commands.rs:585-587). Should it honour the setting?
7. **HDR.** Tone-map to SDR only, or also keep an HDR master such as `.jxr`?
8. **Console rendering.** Accept CPU-only rendering for the quick editor, or split the console off the banner's GPU-less browser process (overlay.rs:53-63)?
9. **Clipboard.** HDROP, DIB and PNG together, or HDROP alone? It depends on what Discord pastes when it is offered both.
10. **Option A.** Revisit the single WebView renderer after the M3 measurements, or rule it out for v2?
