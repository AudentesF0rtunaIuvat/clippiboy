# ClippiBoy review: supply chain, CI/release, website, Stream Deck plugin, tooling

Everything was read-only. I changed nothing in the repo. Scratch work (copied lockfiles, a RustSec advisory-db clone, a small matcher script, SDK tarballs) is under <review-scratch>/.

**The working tree is dirty, but not from me.** Another reviewer's build left `dist/` and a modified `src-tauri/gen/schemas/linux-schema.json` (+252 lines). That diff is useful evidence for finding 13.

## 1. Build, release and distribution chain

1. **App release.** A maintainer pushes a `v*` tag. `release.yml` runs on windows-latest with floating actions (`checkout@v4`, `setup-node@v4`, `dtolnay/rust-toolchain@stable`, `swatinem/rust-cache@v2`):
   - `npm ci`, then the Stream Deck plugin is built (`npm ci` twice, then `tsc` and `streamdeck pack`).
   - `tauri-apps/tauri-action@v0` runs `tauri build` (with `beforeBuildCommand` = `npm run build` = `tsc --noEmit && vite build`).
   - It signs the NSIS `-setup.exe` with the minisign key `TAURI_SIGNING_PRIVATE_KEY` and creates a **published, non-draft** release with `-setup.exe`, `.sig` and `latest.json`.
   - `gh release upload` then attaches the `.streamDeckPlugin`.
   - There is no Authenticode signature, no checksums and no provenance anywhere.
2. **Website.** When `Release` completes, `pages.yml` (via `workflow_run`) runs `bake-release.mjs`. It reads the GitHub releases API, writes `site/release.json` and patches `index.html`, then deploys `site/` to GitHub Pages.
   - The `server/site-proxy` Worker serves the Pages site at clippiboy.com.
   - `/get/<ref>` reads `release.json`, streams the setup.exe from GitHub under the name `ClippiBoy_<v>_x64-setup.<ref>.exe`, and counts the download in D1.
3. **Install.** The user runs the unsigned per-user NSIS installer, which installs to `%LOCALAPPDATA%\ClippiBoy`. `hooks.nsh` records the installer's file name for the download-source statistic.
4. **Updates.** The app polls `releases/latest/download/latest.json`, checks the minisign signature against the pubkey in `tauri.conf.json` (key id 3E0633A2DA7AD3CE), and runs the new setup.exe in passive mode.
5. **ffmpeg.** A `ffmpeg-*` tag runs `ffmpeg.yml`:
   - `fetch-ffmpeg.mjs` downloads gyan.dev's essentials ZIP from the `GyanD/codexffmpeg` GitHub mirror and checks it against a pinned SHA-256 taken from gyan.dev's `.sha256`.
   - It extracts `ffmpeg.exe` and `ffprobe.exe`, re-zips them, and publishes them as release `ffmpeg-9.0.1` with `--latest=false`.
   - On first start the app downloads that ZIP and checks **both exe SHA-256s pinned in `tools.rs:34-43`** before moving them into place.
   - The pinning holds end to end: an upstream or asset swap is rejected. The one gap is finding 21.
6. **Stream Deck.** Users double-click the `.streamDeckPlugin`. The plugin reads `%APPDATA%\ClippiBoy\streamdeck.json` (port and token) and polls `http://127.0.0.1:<port>/v1/status` with an `X-ClippiBoy-Token` header.
7. **Server.** The Workers are deployed by hand (`wrangler deploy`). There is no CI for them.

## 2. Findings, most severe first

### High

**1. [High][supply-chain/security] Installer and app exe are not Authenticode-signed, and the first install has no integrity check (confirmed)**
- `src-tauri/tauri.conf.json:63-70`: the `bundle.windows` block has only an `"nsis"` section. There is no `certificateThumbprint`, `signCommand` or `timestampUrl`.
- The site publishes no checksums (`site/index.html:35,46` link to `clippiboy.com/get/site`).
- `scripts/make-probe.mjs:71` tells testers: "If Windows warns that the publisher is unknown: 'More info' and then 'Run anyway'."
- Scenario: minisign only protects updates. A fresh install trusts whatever asset currently sits on the latest release. Anyone or anything holding `contents: write` can replace `-setup.exe` (the compromised-action path in finding 3, a leaked PAT, a careless maintainer). Every new download through clippiboy.com/get would then serve the replacement, and nothing verifies it.
- Users also see SmartScreen warnings, and on Windows 11 with Smart App Control on, unsigned, low-reputation binaries are blocked outright.
- Fix:
  - Sign with Azure Trusted Signing through tauri's `bundle.windows.signCommand` (for example `trusted-signing-cli -e <endpoint> -a <account> -c <profile> %1`), or another OV/EV certificate.
  - Enable GitHub "immutable releases".
  - Publish `SHA256SUMS` and `actions/attest-build-provenance` attestations, and show the hash on the site.
  - Stop telling users to click through SmartScreen.

**2. [High][bug/release] A `workflow_dispatch` "trial run" publishes a real signed release tagged `main` (confirmed against tauri-action v0 source)**
- `release.yml:15-16`: `# For a trial run without a tag.` / `workflow_dispatch:`.
- `:60` `tagName: ${{ github.ref_name }}`, `:68` `releaseDraft: false`, `:71` `includeUpdaterJson: true`.
- tauri-action `src/index.ts:178` reads `if (tagName && !releaseId) { ... createRelease(...) }`, and `create-release.ts` calls `createRelease({ tag_name, target_commitish: commitish || context.sha, draft })`.
- Scenario: on a dispatch from main, `ref_name` is `"main"`. tauri-action creates git tag `main` and publishes a non-draft release "ClippiBoy main", signed with the production key and carrying a `latest.json`. GitHub marks new published releases as latest by default.
  - The updater endpoint `releases/latest/download/latest.json` then serves the trial build.
  - If the version was already bumped, which is the documented order (README:752 "bump the version … first"), every installation is offered an untagged build.
  - A tag named `main` also makes `git checkout main` ambiguous.
  - The comment at `:74-75` ("a workflow_dispatch trial run has no release to attach anything to") is wrong.
- Fix: `tagName: ${{ startsWith(github.ref, 'refs/tags/') && github.ref_name || '' }}` (an empty tagName makes tauri-action build only), and upload trial artifacts with `actions/upload-artifact`.

**3. [High][supply-chain] Third-party actions on mutable refs run alongside the updater signing key and a `contents: write` token (confirmed)**
- `release.yml:33` `dtolnay/rust-toolchain@stable` (a branch), `:36` `swatinem/rust-cache@v2`, `:54` `tauri-apps/tauri-action@v0` (currently 84b9d35 = v0.6.2, while v1.0.0 exists).
- `:57-58` the signing key and password are passed in env to tauri-action. `actions/checkout@v4` (`:26`, ffmpeg.yml `:21`) keeps the default `persist-credentials: true`.
- `npm ci` (`:40`, `:50-51`) runs dependency install scripts in the same job.
- Scenario: a retagged or compromised action (the tj-actions/changed-files incident pattern) or a malicious lifecycle script could:
  - read `TAURI_SIGNING_PRIVATE_KEY` and sign a malicious update for every user;
  - or use the token left in `.git/config` to replace release assets (see finding 1).
- Fix:
  - Pin every action to a full SHA and add Dependabot for `github-actions`.
  - Set `persist-credentials: false`.
  - Run `npm ci --ignore-scripts` where possible.
  - Move signing into its own job or step, or an environment (finding 4).

### Medium

**4. [Medium][security] Signing-key governance (confirmed)**
- The secrets are repo-level (`release.yml:7-9`) with no `environment:` protection.
- Anyone who can push a tag can point a `v*` tag at a commit carrying a modified `release.yml` and print the key. There is no review gate and no tag ruleset.
- README:759-765: "the key lives under `%USERPROFILE%\.clippiboy\updater.key` with its password beside it in `updater.password`". A plaintext password sitting next to the encrypted key defeats the encryption.
- There is a single key with no rotation or backup plan, and README:750 notes it cannot be recovered if lost.
- Fix:
  - Create a `release` environment with required reviewers and a deployment rule limited to `v*` tags, and move both secrets there.
  - Add a tag ruleset.
  - Store the password in a password manager, not beside the key.
  - Keep an offline backup of the key.

**5. [Medium][legal] GPLv3 ffmpeg is redistributed without license text or a source offer (likely)**
- `ffmpeg.yml:38-39` zips only the two exes: `Compress-Archive -Path dist-ffmpeg/ffmpeg.exe, dist-ffmpeg/ffprobe.exe`. gyan.dev's LICENSE and README are dropped.
- The release notes at `:52` say only "(gyan.dev essentials build $version)".
- gyan.dev essentials builds are configured `--enable-gpl --enable-version3` (libx264/libx265).
- `make-probe.mjs --full` (`:113-117`) ships the same exes with no license either.
- Distributing GPLv3 object code requires the license and Corresponding Source, or a written offer, or equivalent access with clear directions (GPLv3 §6).
- Running ffmpeg as a separate process keeps the proprietary app itself out of scope, but the redistribution obligations still apply.
- Fix: include `LICENSE.txt` and gyan's README in the ZIP and the release; link to or mirror the exact source and build scripts for that version; mention it in-app and on the site. Alternatively switch to an LGPL build if the GPL encoders are not needed.

**6. [Medium][legal] No third-party notices anywhere (confirmed)**
- A search of `src/`, `src-tauri/src`, `src-tauri/windows` and `site/` found no license or notice file or screen. The only LICENSE is the proprietary one (`LICENSE:23-25` says third-party parts "stay under their own licenses").
- The installer statically links about 740 crates (MIT, Apache-2.0, BSD, ISC; Apache-2.0 also needs its NOTICE files). React, zustand, the bundled SQLite and ring are in there too. The plugin ships `@elgato/streamdeck`, `ws` and `zod` in `node_modules`. The site serves Inter (OFL).
- MIT and BSD require the copyright notice in binary distributions.
- Fix: generate `THIRD-PARTY-NOTICES` with `cargo about` plus `license-checker` or `npm-license-crawler` in CI, bundle it as a resource, and link it from Settings → About and from the site.

**7. [Medium][dx/quality] No CI quality gate at all (confirmed)**
- `.github/` contains only `ffmpeg.yml`, `pages.yml` and `release.yml`. There is no PR or push workflow, no `dependabot.yml`, no `rustfmt`/`clippy`/`deny` config and no `rust-toolchain.toml`.
- `grep '#[test]' src-tauri/src` counts 271 tests that never run in CI. `server/src/*.test.ts` and `server`'s `tsc --noEmit` (`server/package.json:9-10`) never run either.
- A release is the first time code is compiled in CI, so a broken commit is found at tag time or by users.
- Fix: the `ci.yml` in section 3.

**8. [Medium][security] `server/scripts/e2e.mjs` puts accounts with publicly known bearer tokens into production (confirmed by reading; unverified whether any exist right now)**
- `:4-6`: `node scripts/e2e.mjs --seed | npx wrangler d1 execute clippiboy-friends --remote ...`.
- `:16-21` inserts sessions for `hash("tok-a-test-only")` and `hash("tok-b-test-only")`. `:11` defaults to the production workers.dev URL.
- Scenario: between the seed and the manual `DELETE`, or forever if cleanup is forgotten, anyone reading the public repo can authenticate as these accounts. With them they can use share-link upload to R2 (up to 50 MB a clip, publicly linkable), friend requests, presence and so on.
- Fix: run e2e against `wrangler dev --local` or a staging D1; or generate random tokens per run and expire them; and put the cleanup in a `finally` block.

**9. [Medium][quality] The Release workflow never checks that the tag matches the version (confirmed)**
- `release.yml:3-5` says "The tag has to match the `version` in `src-tauri/tauri.conf.json`", but nothing enforces it. `ffmpeg.yml:33-37` has exactly such a check.
- If `v0.9.0` is tagged without bumping `tauri.conf.json` (0.8.3), `latest.json` advertises 0.8.3, nobody is updated, and the release is mislabelled.
- Fix: the guard step in section 3, also comparing `package.json` and `Cargo.toml`.

### Low

**10. [Low][supply-chain] Dependency advisories (confirmed with RustSec DB and `npm audit --package-lock-only` on copies)**
- `rustls 0.23.43` (`Cargo.lock:4733`) has RUSTSEC-2026-0285, -0346, -0348 and -0349, fixed in ≥0.23.46. All are low impact for a client that has no brotli feature, but the fix is a free `cargo update -p rustls`.
- `zip 4.6.1` has RUSTSEC-2026-0367, -0368 and -0369, fixed in 9.0. These affect only `extract()`. `tools.rs:265-283` uses `by_name` with its own hashing, so the app is **not affected**; upgrade anyway.
- `rusqlite 0.32` bundles SQLite 3.46.0 (the latest rusqlite is 0.40). Later SQLite CVEs need attacker-supplied SQL. Low.
- `tiny_http 0.12.0` (last release 2022) is low-maintenance but bound to loopback only. Acceptable.
- tauri 2.11.5 (latest 2.12.4), tauri-plugin-updater 2.10.1 (latest 2.14.0), iroh 1.2.0 (latest 1.3.0).
- The npm advisories are all dev or build-time only:
  - root: `source-map-js` (high, DoS);
  - streamdeck: `brace-expansion` and `fast-uri`;
  - server: `wrangler 4.141` → `miniflare` → `undici 7.29` and `sharp`.
  - The plugin's runtime lockfile is clean.
- Unmaintained transitive crates (`paste`, `proc-macro-error`, `unic-*`) come through tauri; `glib 0.18` (unsound) is Linux-only.

**11. [Low][quality] Duplicate crates and build bloat (confirmed)**
- `Cargo.toml:65` `windows = 0.58` is used only by clippiboy, while tauri, wry and tao use 0.61.3 and netwatch uses 0.62.2: three `windows` crates.
- `png = "0.17"` (`:47`) sits next to 0.18 from image and tray-icon. `dirs = "5"` (`:39`) sits next to 6 from tauri and 4 from auto-launch. 58 crates are duplicated in total.
- Good news: there is a single `reqwest` (0.13.4) and no aws-lc.
- `crate-type = ["staticlib", "cdylib", "rlib"]` (`:10`) is the mobile template default. With `lto = true` and `codegen-units = 1`, a Windows-only app pays extra link time for it.
- Fix: move to `windows`/`windows-core` 0.61, `png` 0.18 and `dirs` 6, and set `crate-type = ["rlib"]` (or drop `[lib]` crate-type).

**12. [Low][dx] Toolchain is not reproducible, and the rust-version is wrong (confirmed)**
- `Cargo.toml:6` says `rust-version = "1.77"`, but `telemetry.rs:11` uses `std::sync::LazyLock` (needs 1.80) and iroh 1.2.0 declares an MSRV of 1.91.
- CI uses floating `@stable` and `windows-latest`. tauri-action does not pass `--locked`.
- Fix: add a `rust-toolchain.toml` with a pinned version, set `rust-version = "1.91"`, and pass `args: -- --locked` (verify that tauri forwards it to cargo).

**13. [Low][quality] Committed generated files drift (confirmed)**
- `src-tauri/gen/schemas/*` is tracked, and a build in this checkout regenerated `linux-schema.json` with +252 lines (the deep-link permissions). The committed copy is stale.
- The Tauri template ignores `/gen/schemas`. Either ignore it, or regenerate it in CI and fail on a diff.

**14. [Low][security] Script-injection pattern with `github.ref_name` (confirmed pattern; only tag pushers can trigger it)**
- `ffmpeg.yml:33,49-50`: `$version = "${{ github.ref_name }}" -replace ...` inside a pwsh double-quoted string.
- `release.yml:81`: `gh release upload ${{ github.ref_name }} ...`.
- Git ref names may contain `$ ( ) ; \` "`, so a tag such as `ffmpeg-9$(calc)` executes.
- Fix: `env: TAG: ${{ github.ref_name }}`, then use `"$env:TAG"`.

**15. [Low][bug] `bake-release.mjs` uses string replacements, which expand `$` patterns (confirmed)**
- `:97` `` `$1${escape(release.plugin.url)}"` `` and `:102-106` interpolate the tag/URL into `String.replace` replacement strings. A tag or URL containing `$&`, `` $` `` or `$'` would splice page content.
- `escape()` does not cover this. Use function replacers: `html.replace(re, (_, a, b) => a + value + b)`.

**16. [Low][quality] The public download count includes updater traffic (confirmed)**
- `bake-release.mjs:61-63` says "The `.sig` and `latest.json` beside it are fetched by the updater, not by people, and are left out."
- With Tauri v2 and `createUpdaterArtifacts: true`, the NSIS updater downloads the `-setup.exe` itself. Every auto-update of every install therefore counts as a "download".
- The `/get` proxy (`site-proxy/index.ts:61`) also fetches from GitHub for bots it does not count in D1.
- Fix: show the D1 `downloads` sum instead, or subtract updater fetches.

**17. [Low][bug] Stream Deck plugin never rediscovers the app after a connection failure (confirmed)**
- `streamdeck/src/client.ts:212-216` returns `null` on any fetch error and `#poll` (`:183-185`) calls `forget()` only on HTTP 403. `#connect` (`:228-230`) returns the cached connection forever.
- Scenario: the user changes the control port in ClippiBoy (`control.rs:108` `restart`). The keys show "—" until the user clicks "Save and check" in the property inspector or Stream Deck restarts.
- The cached port is also kept when the app has exited, so a different local process (on a multi-user machine) that later binds that port receives the token twice a second.
- Fix: call `forget()` after N consecutive network errors, or when the handshake file's mtime changes, and drop the cache once the handshake disappears.

**18. [Low][quality] Stream Deck plugin housekeeping (confirmed)**
- Overlapping polls: `setInterval(…, 500)` (`client.ts:116`) with a 2 s timeout (`:88`) and no in-flight guard, so up to 4 requests can be outstanding against a stalled app.
- `setTitle`/`setState` are sent for every key every 500 ms even when nothing changed (`keys.ts`, `actions/*.ts`). Diff against the last drawn values.
- `manifest.json:24-27` `"Nodejs": {"Version": "20", "Debug": "enabled"}`:
  - Node 20 has been end-of-life since April 2026. Version "24" requires raising `Software.MinimumVersion`.
  - `Debug: enabled` only applies in Stream Deck developer mode (`--inspect=127.0.0.1:{PORT}`), but should be removed from releases.
- `@elgato/streamdeck` is at ^2.1.2 while 3.0.2 is available.
- The manual token is stored in plain global settings. SDK v3 "Secrets" (Stream Deck 6.9+) exists for this.
- The plugin README (`:23`) omits `/v1/recording`.
- The inner `package.json` says version 0.4.0 while the manifest says 0.6.5.0.
- Checked and fine:
  - Property inspector (`ui/settings.html`): no XSS. It uses `textContent` and `createElement` throughout.
  - `install.ps1`: dev-only, no admin, writes to `%APPDATA%` only. It uses `npm install` rather than `npm ci`.

**19. [Low][security] Website has no security headers (confirmed)**
- `server/site-proxy/index.ts:29-43` passes GitHub Pages' headers through and only rewrites `Location`. There is no CSP, `X-Content-Type-Options`, `Referrer-Policy` or `frame-ancestors`.
- The risk is low:
  - `site/main.js` is XSS-clean: `textContent`, `replaceChildren`, and `?ref=` validated against `/^[a-z0-9-]{1,32}$/` (`:385-386`).
  - There are no external scripts or fonts (Inter is self-hosted) and no `target=_blank`.
- Fix: in the proxy, set:
  - `Content-Security-Policy: default-src 'self'; connect-src 'self' https://api.github.com; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'`
  - `X-Content-Type-Options: nosniff` and `Referrer-Policy: strict-origin-when-cross-origin`
  - HSTS at Cloudflare.

**20. [Low][privacy/hygiene] Personal identifier in a public hostname, and a second public origin (confirmed)**
- `server/README.md:7,23` and `e2e.mjs:11` contain `clippiboy-friends.fabian081964.workers.dev` (first name plus number). `server/wrangler.jsonc:12` keeps `"workers_dev": true`.
- That origin bypasses any zone-level WAF or rate-limit rules applied to `api.clippiboy.com`.
- Fix: plan to set `workers_dev: false` once old builds are gone, and scrub the name from docs.

**21. [Low][supply-chain] An unverified ffmpeg runs until the pinned copy is in place (confirmed)**
- `src-tauri/src/muxer.rs:39-49` falls back to `_ => PathBuf::from(name)` when `TOOL_DIR` is unset or the file is missing.
- Rust's `Command` on Windows searches the exe's own directory (`%LOCALAPPDATA%\ClippiBoy`), then System32, then PATH. Any ffmpeg found that way runs without a hash check while the download is pending ("The buffer already runs while that happens", `site/index.html:191-192`).
- Same-user trust only, but it breaks the "tested build only" promise. Fix: refuse to fall back, or hash-check the fallback.

**22. [Low][security] `tools/report` binary planting and contents (likely)**
- `tools/report/src/main.rs:98-101` calls `Command::new("powershell")` by bare name. Rust resolves the application directory first, so a `powershell.exe` planted next to `clippiboy-report.exe` in Downloads (where an exe "sent around", per README:40-41, usually lands) runs instead.
- Fix: use `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`.
- What the report does, verified: it is read-only and writes only `clippiboy-report.txt` to the desktop. It contains:
  - COMPUTERNAME, USERNAME, the full `config.json` and full logs;
  - GPU and OS details, Run/Startup entries, the ffmpeg directory tree, and 30 days of Application event-log hits.
- `redact_token` (`:213-243`) blanks only `"token"` string values. That is enough today, because the friends session lives in `friends.json`, which the tool does not read.
- Consider masking profile paths, and add a "review before sharing" notice.
- It uploads nothing.

**23. [Low][dx] The rust-cache is probably useless for releases (likely)**
- Caches saved by a run on `refs/tags/vX` are visible only to that ref and to the default branch. No workflow builds Rust on main, so every release likely starts cold (the comment at `release.yml:35` expects reuse).
- A main-branch CI job (section 3) would warm it.

**24. [Low][quality] Non-draft release creation is a design misconception (confirmed)**
- `release.yml:66-67` says "Publishing as a draft would be safer, but the updater only sees finished releases."
- Create the release as a draft, upload the installer, `.sig`, `latest.json` and plugin, then run `gh release edit "$env:TAG" --draft=false` as the last step. This removes the window where "latest" has no `latest.json` or no plugin.

**25. [Low][legal/dx] Miscellaneous**
- LICENSE:16-18 forbids copying or modifying the source, while README:700-712 tells readers to clone and build it. Clarify that a personal build is allowed.
- `render-sounds.cjs:7` uses an unpinned `npm i --no-save node-web-audio-api`, a native module with install scripts.
- The root `.gitignore` lacks `.env` and `site/release.json`.
- `Cargo.toml` lacks `publish = false`.
- README (`:715`) says "Node 20+", which is end-of-life.

### Brief check of privacy.html against the code (another reviewer goes deeper)
- **Consistent:**
  - one ping a day with exactly the fields in `server/src/telemetry.ts:21-56`;
  - per-ref, per-day download counter (`site-proxy/index.ts:67-75`);
  - iroh starts lazily, only when a clip is sent (`share.rs:878-893`);
  - 400-day retention (`telemetry.ts:114-121`).
- **Watch:**
  - "Our server does not store your IP address with it" is true for the D1 rows, but the friends Worker has `"observability": {"enabled": true}` (`wrangler.jsonc:8`). Workers Logs keep request metadata for /ping; this is disclosed only in the Friends section (privacy.html:93).
  - Telemetry is on by default (`config.rs:66`) with a persistent random ID, justified as legitimate interest. Whether this needs consent under ePrivacy Art. 5(3) is a legal question (speculative).

### Things that are good
- The ffmpeg hash pinning, as described in section 1.
- Lockfiles are committed with integrity hashes, all from registry.npmjs.org and crates.io, and CI uses `npm ci`.
- `pages.yml` has least-privilege top-level permissions.
- `workflow_run` is safe here because `Release` cannot be triggered by PRs or forks.
- No secrets are committed (pattern grep was clean; `*.key` and `*.pem` are ignored).
- The control port binds to 127.0.0.1, requires a token with a constant-time compare, and rejects requests carrying browser headers (`Origin`/`Referer`/`Sec-Fetch-Site`, `control.rs:141-156`).
- `ffmpeg.yml` uses `--latest=false` correctly.

## 3. Recommended CI additions

**New `.github/workflows/ci.yml`:**
```yaml
name: CI
on:
  push: { branches: [main] }
  pull_request:
permissions: { contents: read }
concurrency: { group: ci-${{ github.ref }}, cancel-in-progress: true }
jobs:
  rust:
    runs-on: windows-latest
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262 # v4.4.0
        with: { persist-credentials: false }
      - run: rustup show   # honours rust-toolchain.toml once added
      - uses: Swatinem/rust-cache@6323deb102c322ba6fcbdcafc7e3dddab59af2b6 # v2.9.2
        with: { workspaces: "src-tauri\ntools/report" }
      - run: npm ci --ignore-scripts && npm run build   # tauri-build needs ../dist
      - working-directory: src-tauri
        run: |
          cargo fmt --check
          cargo clippy --locked --all-targets -- -D warnings
          cargo test --locked --lib
      - run: cargo build --locked --release --manifest-path tools/report/Cargo.toml
      - run: git diff --exit-code -- src-tauri/gen   # generated schemas in sync
  web:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
        with: { persist-credentials: false }
      - uses: actions/setup-node@49933ea5288caeca8642d1e84afbd3f7d6820020 # v4.4.0
        with: { node-version: 22, cache: npm }
      - run: npm ci && npm run build
      - working-directory: server
        run: npm ci && npm run check && npm test
      - working-directory: streamdeck
        run: npm ci && npm ci --prefix com.einfabo.clippiboy.sdPlugin && npm run build
      - run: npm audit --omit=dev --audit-level=high
  audit:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
        with: { persist-credentials: false }
      - uses: EmbarkStudios/cargo-deny-action@<sha>   # advisories + licenses + duplicate bans
        with: { manifest-path: src-tauri/Cargo.toml, command: check advisories licenses bans }
```

**`.github/dependabot.yml`:** add entries for `github-actions` (`/`), `cargo` (`/src-tauri`, `/tools/report`) and `npm` (`/`, `/server`, `/streamdeck`, `/streamdeck/com.einfabo.clippiboy.sdPlugin`), each on a weekly schedule.

**`release.yml` hardening (key parts):**
```yaml
jobs:
  windows:
    runs-on: windows-latest
    environment: release            # secrets live here; required reviewers; tag rule v*
    permissions: { contents: write, id-token: write, attestations: write }
    steps:
      - uses: actions/checkout@11d5960a326750d5838078e36cf38b85af677262
        with: { persist-credentials: false }
      - name: Tag must match versions
        if: startsWith(github.ref, 'refs/tags/')
        shell: pwsh
        env: { TAG: "${{ github.ref_name }}" }
        run: |
          $t = (Get-Content src-tauri/tauri.conf.json -Raw | ConvertFrom-Json).version
          $p = (Get-Content package.json -Raw | ConvertFrom-Json).version
          $c = (Select-String -Path src-tauri/Cargo.toml -Pattern '^version = "(.+)"').Matches[0].Groups[1].Value
          if ("v$t" -ne $env:TAG -or $p -ne $t -or $c -ne $t) { throw "tag $env:TAG / tauri $t / npm $p / cargo $c" }
      # ... node/rust setup pinned by SHA, npm ci --ignore-scripts where possible ...
      - uses: tauri-apps/tauri-action@84b9d35b5fc46c1e45415bdb6144030364f7ebc5 # v0.6.2 (or v1.0.0 1deb371b…)
        env: { TAURI_SIGNING_PRIVATE_KEY: ..., TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ... }
        with:
          tagName: ${{ startsWith(github.ref, 'refs/tags/') && github.ref_name || '' }}
          releaseDraft: true
          includeUpdaterJson: true
          args: -- --locked
      - if: startsWith(github.ref, 'refs/tags/')
        shell: pwsh
        env: { GH_TOKEN: "${{ secrets.GITHUB_TOKEN }}", TAG: "${{ github.ref_name }}" }
        run: |
          $setup = Get-ChildItem src-tauri/target/release/bundle/nsis/*-setup.exe
          Get-FileHash $setup, streamdeck/dist/*.streamDeckPlugin -Algorithm SHA256 |
            ForEach-Object { "$($_.Hash.ToLower())  $(Split-Path $_.Path -Leaf)" } | Set-Content SHA256SUMS
          gh release upload "$env:TAG" streamdeck/dist/com.einfabo.clippiboy.streamDeckPlugin SHA256SUMS --clobber
      - uses: actions/attest-build-provenance@<sha>
        with: { subject-path: "src-tauri/target/release/bundle/nsis/*-setup.exe" }
      - if: startsWith(github.ref, 'refs/tags/')
        env: { GH_TOKEN: "${{ secrets.GITHUB_TOKEN }}", TAG: "${{ github.ref_name }}" }
        run: gh release edit "$env:TAG" --draft=false --latest
      - if: ${{ !startsWith(github.ref, 'refs/tags/') }}
        uses: actions/upload-artifact@<sha>
        with: { name: trial-build, path: src-tauri/target/release/bundle/nsis/ }
```

**`ffmpeg.yml` additions:**
- Use `env: TAG: ${{ github.ref_name }}` with `$env:TAG`, plus `persist-credentials: false` and pinned SHAs.
- Optionally fail when the extracted hashes are not already in `tools.rs`:
  ```pwsh
  $src = Get-Content src-tauri/src/tools.rs -Raw
  foreach ($e in 'ffmpeg.exe','ffprobe.exe') { $h=(Get-FileHash "dist-ffmpeg/$e" -A SHA256).Hash.ToLower(); if ($src -notmatch $h) { throw "$e $h not pinned in tools.rs" } }
  ```
- Add gyan's LICENSE/README and a `SOURCE.txt` pointing to the exact source into the ZIP, and attest it.

**Repo settings to change:** a tag ruleset for `v*` and `ffmpeg-*`, immutable releases on, the `release` environment with reviewers, and Authenticode signing through `bundle.windows.signCommand`.
