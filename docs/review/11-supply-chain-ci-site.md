# Supply chain, CI/release, website, Stream Deck plugin, tooling

**Scope:** .github/*, scripts/*, site/*, streamdeck/*, tools/report, package manifests and lockfiles.

**Findings:** 44 — 36 verified (2 medium or higher), 7 reported but not independently re-verified, 0 unverifiable/disputed, 1 rejected by verification.

Severity shown is the verified severity; where verification changed it, the reported one is in parentheses. See [README](README.md#how-findings-were-verified) for how verification worked.

## Verified issues (medium and above)

### SC-1 · Medium (reported High) · Installer and app exe are not Authenticode-signed, and the first install has no integrity check

- **Where:** `src-tauri/tauri.conf.json:63-70`, `site/index.html:35`, `site/index.html:46`, `scripts/make-probe.mjs:71`
- **Verification:** confirmed (reachability: confirmed; code: confirmed; impact: partly confirmed)

**What verification found:** Accurate: the installer and app exe are unsigned, and fresh downloads (including those proxied through clippiboy.com/get, server/site-proxy/index.ts:53-83) get no integrity check, because minisign protects only the updater. Two corrections. First, make-probe.mjs:71 is the README of the separate tester probe package, not installer guidance. Second, severity is overstated. The asset-swap scenario needs someone who already has contents: write, which is a compromise in its own right. In the SC-3 compromised-action path, the attacker runs inside the release job, so it would also reach a signing key wired into that job. Checksums published on the same release or site can be rewritten by the same attacker. So the gap is defense in depth plus distribution friction (SmartScreen warnings, Smart App Control blocks where SAC is on), not a vulnerability that can be exploited on its own.

**Original claim:** The `bundle.windows` block in src-tauri/tauri.conf.json:63-70 has only an "nsis" section: there is no `certificateThumbprint`, `signCommand` or `timestampUrl`, so the NSIS `-setup.exe` and the app exe carry no Authenticode signature. The release pipeline produces no checksums and no provenance, and the site publishes no checksums (site/index.html:35,46 just link to `clippiboy.com/get/site`). scripts/make-probe.mjs:71 tells testers: "If Windows warns that the publisher is unknown: 'More info' and then 'Run anyway'." Minisign only protects updates; a fresh install trusts whatever asset currently sits on the latest release.

**Scenario:** Anyone or anything holding `contents: write` (the compromised-action path in SC-3, a leaked PAT, a careless maintainer) can replace `-setup.exe` on the latest release. Every new download through clippiboy.com/get would then serve the replacement, and nothing verifies it. Users also see SmartScreen warnings, and on Windows 11 with Smart App Control on, unsigned low-reputation binaries are blocked outright.

**Fix:** Sign with Azure Trusted Signing through tauri's `bundle.windows.signCommand` (e.g. `trusted-signing-cli -e <endpoint> -a <account> -c <profile> %1`) or another OV/EV certificate; enable GitHub "immutable releases"; publish `SHA256SUMS` and `actions/attest-build-provenance` attestations and show the hash on the site; stop telling users to click through SmartScreen.

**Verifier on the fix:** Authenticode signing through bundle.windows.signCommand (Azure Trusted Signing, or SignPath/Certum for an individual outside the regions Trusted Signing serves) is the right fix for SmartScreen and SAC. It adds integrity only if the credential sits in a protected GitHub Environment with required reviewers and OIDC, apart from steps that run unpinned third-party actions. GitHub immutable releases block the post-publish asset swap most directly. SHA256SUMS on the same release add little, and attestations help only the few users who verify them. A cheap minimal addition: have the site-proxy work…

### SC-2 · Medium (reported High) · A workflow_dispatch "trial run" publishes a real signed release tagged `main`

- **Where:** `.github/workflows/release.yml:15-16`, `.github/workflows/release.yml:60`, `.github/workflows/release.yml:68`, `.github/workflows/release.yml:71`, `.github/workflows/release.yml:74-75`, `README.md:752`
- **Verification:** confirmed (code: confirmed; reachability: confirmed)

**Problem:** .github/workflows/release.yml:15-16 enables `workflow_dispatch:` under the comment `# For a trial run without a tag.`, while `:60` sets `tagName: ${{ github.ref_name }}`, `:68` `releaseDraft: false` and `:71` `includeUpdaterJson: true`. In tauri-action v0 (`src/index.ts:178`: `if (tagName && !releaseId) { ... createRelease(...) }`; `create-release.ts` calls `createRelease({ tag_name, target_commitish: commitish || context.sha, draft })`) a non-empty tagName always creates a release. On a dispatch from main, `github.ref_name` is "main", so tauri-action creates git tag `main` and publishes a non-draft release "ClippiBoy main", signed with the production updater key and carrying a `latest.json`. The comment at `:74-75` ("a workflow_dispatch trial run has no release to attach anything to") is wrong. Confirmed against tauri-action v0 source.

**Scenario:** GitHub marks new published releases as latest by default, so the updater endpoint `releases/latest/download/latest.json` then serves the trial build. If the version was already bumped, which is the documented order (README.md:752 "bump the version … first"), every installation is offered an untagged build. A tag named `main` also makes `git checkout main` ambiguous.

**Fix:** Use `tagName: ${{ startsWith(github.ref, 'refs/tags/') && github.ref_name || '' }}` (an empty tagName makes tauri-action build only), and upload trial artifacts with `actions/upload-artifact`.

**Verifier on the fix:** The proposed `tagName: ${{ startsWith(github.ref, 'refs/tags/') && github.ref_name || '' }}` evaluates to '' on a branch dispatch. With an empty tagName and no releaseId, tauri-action only builds, and adding actions/upload-artifact for the bundle dir keeps the trial useful. Simpler alternatives: drop workflow_dispatch, or guard the tauri-action step with the same `if` as the attach step and fix the misleading comment.

## Reported, not independently re-verified

The original reviewer checked these against the code, but the independent verification pass was stopped before reaching them. Treat as likely, confirm before fixing.

### SC-3 · High · Third-party actions on mutable refs run alongside the updater signing key and a contents: write token

- **Where:** `.github/workflows/release.yml:33`, `.github/workflows/release.yml:36`, `.github/workflows/release.yml:54`, `.github/workflows/release.yml:57-58`, `.github/workflows/release.yml:26`, `.github/workflows/ffmpeg.yml:21` (+2 more)
- **Verification:** not independently re-verified (not independently re-verified)

**Problem:** .github/workflows/release.yml uses third-party actions on mutable refs: `:33` `dtolnay/rust-toolchain@stable` (a branch), `:36` `swatinem/rust-cache@v2`, `:54` `tauri-apps/tauri-action@v0` (currently 84b9d35 = v0.6.2, while v1.0.0 exists). `:57-58` pass the signing key `TAURI_SIGNING_PRIVATE_KEY` and its password in env to tauri-action. `actions/checkout@v4` (release.yml:26, ffmpeg.yml:21) keeps the default `persist-credentials: true`, leaving the `contents: write` token in `.git/config`. `npm ci` (release.yml:40, :50-51) runs dependency install scripts in the same job.

**Scenario:** A retagged or compromised action (the tj-actions/changed-files incident pattern) or a malicious npm lifecycle script could read `TAURI_SIGNING_PRIVATE_KEY` and sign a malicious update for every user, or use the token left in `.git/config` to replace release assets (see SC-1).

**Fix:** Pin every action to a full SHA and add Dependabot for `github-actions`; set `persist-credentials: false`; run `npm ci --ignore-scripts` where possible; move signing into its own job or step, or an environment (SC-4).

- **SC-10 · Medium · Release workflow never checks that the tag matches the version** — `.github/workflows/release.yml:3-5`, `.github/workflows/ffmpeg.yml:33-37`  
  .github/workflows/release.yml:3-5 says "The tag has to match the `version` in `src-tauri/tauri.conf.json`", but no step enforces it, whereas ffmpeg.yml:33-37 has exactly such a check.  
  *Fix:* Add the "Tag must match versions" guard step from section 3 (SC-P-3), also comparing `package.json` and `Cargo.toml`.
- **SC-4 · Medium · Updater signing secrets are repo-level with no environment protection, review gate or tag ruleset** — `.github/workflows/release.yml:7-9`  
  The updater signing secrets used by .github/workflows/release.yml:7-9 are repo-level secrets with no `environment:` protection. There is no review gate and no tag ruleset (part of the report's "Signing-key governance" finding).  
  *Fix:* Create a `release` environment with required reviewers and a deployment rule limited to `v*` tags, move both secrets there, and add a tag ruleset.
- **SC-5 · Medium · Updater key password is stored in plaintext beside the key; single key with no rotation or backup** — `README.md:759-765`, `README.md:750`  
  README.md:759-765 documents that "the key lives under `%USERPROFILE%\.clippiboy\updater.key` with its password beside it in `updater.password`". A plaintext password sitting next to the encrypted key defeats the encryption. There is a single key with no rotation or backup plan, and README.md:750 notes it cannot be recovered if lost (part of the report's "Signing-key governance" finding).  
  *Fix:* Store the password in a password manager, not beside the key, and keep an offline backup of the key.
- **SC-6 · Medium · GPLv3 ffmpeg is redistributed without license text or a source offer** — `.github/workflows/ffmpeg.yml:38-39`, `.github/workflows/ffmpeg.yml:52`, `scripts/make-probe.mjs:113-117`  
  .github/workflows/ffmpeg.yml:38-39 zips only the two exes (`Compress-Archive -Path dist-ffmpeg/ffmpeg.exe, dist-ffmpeg/ffprobe.exe`), dropping gyan.dev's LICENSE and README; the release notes at `:52` say only "(gyan.dev essentials build $version)". gyan.dev essentials builds are configured `--enable-gpl --enable-version3` (libx264/libx265). `scripts/make-probe.mjs --full` (:113-117) ships the same exes with no license either. Distributing GPLv3 object code requires the license and Corresponding Source, or a written offer, or equivalent access with clear directions (GPLv3 §6). Running ffmpeg…  
  *Fix:* Include `LICENSE.txt` and gyan's README in the ZIP and the release; link to or mirror the exact source and build scripts for that version; mention it in-app and on the site. Alternatively switch to an LGPL build if the GPL encoders are not needed.
- **SC-7 · Medium · No third-party license notices anywhere** — `LICENSE:23-25`, `src`, `src-tauri/src` (+2 more)  
  A search of `src/`, `src-tauri/src`, `src-tauri/windows` and `site/` found no third-party license or notice file or screen. The only LICENSE is the proprietary one (LICENSE:23-25 says third-party parts "stay under their own licenses"). The installer statically links about 740 crates (MIT, Apache-2.0, BSD, ISC; Apache-2.0 also needs its NOTICE files), plus React, zustand, the bundled SQLite and ring. The Stream Deck plugin ships `@elgato/streamdeck`, `ws` and `zod` in `node_modules`. The site serves Inter (OFL). MIT and BSD require the copyright notice in binary distributions.  
  *Fix:* Generate `THIRD-PARTY-NOTICES` with `cargo about` plus `license-checker` or `npm-license-crawler` in CI, bundle it as a resource, and link it from Settings → About and from the site.
- **SC-8 · Medium · No CI quality gate at all** — `.github/workflows`, `server/package.json:9-10`  
  `.github/` contains only `ffmpeg.yml`, `pages.yml` and `release.yml`. There is no PR or push workflow, no `dependabot.yml`, no `rustfmt`/`clippy`/`deny` config and no `rust-toolchain.toml`. `grep '#[test]' src-tauri/src` counts 271 tests that never run in CI; `server/src/*.test.ts` and server's `tsc --noEmit` (server/package.json:9-10) never run either.  
  *Fix:* Add the `ci.yml` from section 3 of the report (see SC-P-1).

## Verified lower-severity issues

| ID | Severity | Title | Where | Fix |
|---|---|---|---|---|
| SC-11 | Low | rustls 0.23.43 has open RustSec advisories | src-tauri/Cargo.lock:4733 | `cargo update -p rustls` (a free fix). |
| SC-18 | Low | Outdated direct dependencies cause duplicate crates (windows, png, dirs) | src-tauri/Cargo.toml:65, src-tauri/Cargo.toml:47 | Move to `windows`/`windows-core` 0.61, `png` 0.18 and `dirs` 6. |
| SC-19 | Low | Mobile-template crate-type adds link time to a Windows-only app | src-tauri/Cargo.toml:10 | Set `crate-type = ["rlib"]` (or drop the `[lib]` crate-type). |
| SC-20 | Low | Declared rust-version 1.77 is wrong | src-tauri/Cargo.toml:6, src-tauri/src/telemetry.rs:11 | Set `rust-version = "1.91"`. |
| SC-21 | Low | Release toolchain is not reproducible (floating toolchain, no --locked) | .github/workflows/release.yml:33, .github/workflows/release.yml:54 | Add a `rust-toolchain.toml` with a pinned version and pass `args: -- --locked` to tauri-action (verify that tauri forwards it to cargo). |
| SC-25 | Low | Public download count includes updater traffic | .github/scripts/bake-release.mjs:61-63, server/site-proxy/index.ts:61 | Show the D1 `downloads` sum instead, or subtract updater fetches. |
| SC-26 | Low | Stream Deck plugin never rediscovers the app after a connection failure | streamdeck/src/client.ts:212-216, streamdeck/src/client.ts:183-185 | Call `forget()` after N consecutive network errors, or when the handshake file's mtime changes, and drop the cache once the handshake disappears. |
| SC-27 | Low | Stream Deck status polls overlap with no in-flight guard | streamdeck/src/client.ts:116, streamdeck/src/client.ts:88 | Add an in-flight guard so a new poll does not start while one is pending (implied by the report). |
| SC-28 | Low | Stream Deck keys are redrawn every 500 ms even when nothing changed | streamdeck/src/keys.ts, streamdeck/src/actions/*.ts | Diff against the last drawn values and only send changes. |
| SC-29 | Low | Stream Deck manifest targets end-of-life Node 20 with Debug enabled | streamdeck/com.einfabo.clippiboy.sdPlugin/manifest.json:24-27 | Move to Node "24" (requires raising `Software.MinimumVersion`) and remove `Debug: enabled` from releases. |
| SC-35 | Low | Website has no security headers | server/site-proxy/index.ts:29-43 | In the proxy, set `Content-Security-Policy: default-src 'self'; connect-src 'self' https://api.github.com; img-src 'self' data:; object-src 'none'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'`, `X-Conten… |
| SC-37 | Low | An unverified ffmpeg from the search path runs until the pinned copy is in place | src-tauri/src/muxer.rs:39-49, site/index.html:191-192 | Refuse to fall back, or hash-check the fallback. |
| SC-38 | Low | tools/report invokes powershell by bare name (binary planting) | tools/report/src/main.rs:98-101, tools/report/README.md:40-41 | Use `%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe`. |
| SC-40 | Low | Non-draft release creation rests on a misconception and leaves an incomplete-latest window | .github/workflows/release.yml:66-67 | Create the release as a draft, upload the installer, `.sig`, `latest.json` and plugin, then run `gh release edit "$env:TAG" --draft=false` as the last step. |
| SC-43 | Low | Root .gitignore lacks .env and site/release.json | .gitignore | Add `.env` and `site/release.json` to `.gitignore` (implied). |
| SC-12 | Info (reported Low) | zip 4.6.1 has RustSec advisories (app not affected) | src-tauri/Cargo.lock, src-tauri/src/tools.rs:265-283 | Upgrade zip (to 9.0) anyway. |
| SC-13 | Info (reported Low) | rusqlite 0.32 bundles an old SQLite 3.46.0 | src-tauri/Cargo.lock | None stated explicitly; the report points to the newer rusqlite 0.40. |
| SC-14 | Info | tiny_http 0.12.0 is low-maintenance (judged acceptable) | src-tauri/Cargo.lock | None proposed (acceptable). |
| SC-15 | Info (reported Low) | tauri, tauri-plugin-updater and iroh lag current releases | src-tauri/Cargo.lock | None stated explicitly; implied update to the listed latest versions. |
| SC-16 | Info (reported Low) | npm audit reports dev/build-time advisories in root, streamdeck and server lockfiles | package-lock.json, streamdeck/package-lock.json | None stated explicitly; section 3 adds `npm audit --omit=dev --audit-level=high` to CI. |
| SC-17 | Info (reported Low) | Unmaintained and unsound transitive crates via tauri | src-tauri/Cargo.lock | None stated explicitly (section 3 proposes cargo-deny advisories checks in CI). |
| SC-22 | Info (reported Low) | Committed generated schema files drift | src-tauri/gen/schemas/linux-schema.json | Either ignore `/gen/schemas`, or regenerate it in CI and fail on a diff. |
| SC-23 | Info (reported Low) | Script-injection pattern with github.ref_name in workflow run steps | .github/workflows/ffmpeg.yml:33, .github/workflows/ffmpeg.yml:49-50 | Use `env: TAG: ${{ github.ref_name }}`, then `"$env:TAG"` in the script. |
| SC-24 | Info (reported Low) | bake-release.mjs String.replace replacement strings expand `$` patterns | .github/scripts/bake-release.mjs:97, .github/scripts/bake-release.mjs:102-106 | Use function replacers: `html.replace(re, (_, a, b) => a + value + b)`. |
| SC-30 | Info (reported Low) | @elgato/streamdeck SDK is a major version behind | streamdeck/com.einfabo.clippiboy.sdPlugin/package.json:8 | Upgrade to @elgato/streamdeck 3.x (implied). |
| SC-32 | Info (reported Low) | Stream Deck plugin README omits /v1/recording | streamdeck/README.md:23 | Add `/v1/recording` to the README (implied). |
| SC-33 | Info (reported Low) | Stream Deck inner package.json version disagrees with manifest | streamdeck/com.einfabo.clippiboy.sdPlugin/package.json, streamdeck/com.einfabo.clippiboy.… | Align the versions (implied). |
| SC-34 | Info | streamdeck/install.ps1 uses npm install instead of npm ci | streamdeck/install.ps1 | Use `npm ci` (implied). |
| SC-36 | Info (reported Low) | Personal identifier in a public workers.dev hostname, which is also a second public origin | server/README.md:7, server/README.md:23 | Plan to set `workers_dev: false` once old builds are gone, and scrub the name from docs. |
| SC-39 | Info (reported Low) | rust-cache is probably useless for release builds | .github/workflows/release.yml:35 | A main-branch CI job (SC-P-1) would warm the cache. |
| SC-41 | Info (reported Low) | LICENSE forbids copying/modifying while README tells readers to clone and build | LICENSE:16-18, README.md:700-712 | Clarify that a personal build is allowed. |
| SC-42 | Info (reported Low) | render-sounds.cjs installs an unpinned native npm module | scripts/render-sounds.cjs:7 | None stated explicitly; implied pinning of the version. |
| SC-44 | Info (reported Low) | src-tauri/Cargo.toml lacks publish = false | src-tauri/Cargo.toml | Add `publish = false` (implied). |
| SC-45 | Info (reported Low) | README requires end-of-life Node 20+ | README.md:715 | Update the stated Node requirement (implied). |

## Claims rejected by verification

| ID | Claim | Why it was rejected |
|---|---|---|
| SC-31 | Manual control-port token stored in plain Stream Deck global settings | Storage is as described: settings.html:109 `<input id="token" type="text" ...>` and :249 `send({ event: "setGlobalSettings", context: uuid, payload: { port, token } })`; client.ts:208-210 reads it back only as a fallback after the handshake file. But Secrets cannot hold it. In @elgato/streamdeck 2.… |

## What is solid

- ffmpeg hash pinning holds end to end: fetch-ffmpeg.mjs checks gyan.dev's essentials ZIP (from the GyanD/codexffmpeg mirror) against a pinned SHA-256 taken from gyan.dev's .sha256, and on first start the app checks both ffmpeg.exe and ffprobe.exe SHA-256s pinned in src-tauri/src/tools.rs:34-43 before moving them into place, so an upstream or asset swap is rejected (the one gap is SC-37).
- The updater checks the minisign signature of latest.json/setup.exe against the pubkey in tauri.conf.json (key id 3E0633A2DA7AD3CE).
- Lockfiles are committed with integrity hashes, all sourced from registry.npmjs.org and crates.io, and CI uses `npm ci`.
- pages.yml has least-privilege top-level permissions.
- `workflow_run` in pages.yml is safe because `Release` cannot be triggered by PRs or forks.
- No secrets are committed (pattern grep was clean; `*.key` and `*.pem` are gitignored).
- The control port binds to 127.0.0.1, requires a token with a constant-time compare, and rejects requests carrying browser headers (`Origin`/`Referer`/`Sec-Fetch-Site`, src-tauri/src/control.rs:141-156).
- ffmpeg.yml uses `--latest=false` correctly for the ffmpeg release.
- There is a single `reqwest` (0.13.4) in the dependency tree and no aws-lc.
- The zip RUSTSEC-2026-0367/-0368/-0369 advisories do not affect the app: src-tauri/src/tools.rs:265-283 uses `by_name` with its own hashing rather than `extract()`.
- The Stream Deck plugin's runtime lockfile is clean under npm audit; all npm advisories are dev/build-time only.
- Stream Deck property inspector (streamdeck/com.einfabo.clippiboy.sdPlugin/ui/settings.html) has no XSS: it uses `textContent` and `createElement` throughout.
- streamdeck/install.ps1 is dev-only, needs no admin, and writes to `%APPDATA%` only.
- site/main.js is XSS-clean: it uses `textContent` and `replaceChildren`, and validates `?ref=` against `/^[a-z0-9-]{1,32}$/` (site/main.js:385-386); there are no external scripts or fonts (Inter is self-hosted) and no `target=_blank`.
- tools/report is read-only, writes only `clippiboy-report.txt` to the desktop and uploads nothing; `redact_token` (tools/report/src/main.rs:213-243) blanks `"token"` string values, which is enough today because the friends session lives in `friends.json`, which the tool does not read.
- privacy.html is consistent with the code on: one ping a day with exactly the fields in server/src/telemetry.ts:21-56; the per-ref, per-day download counter (server/site-proxy/index.ts:67-75); iroh starting lazily only when a clip is sent (src-tauri/src/share.rs:878-893); 400-day retention (server/src/telemetry.ts:114-121); and no IP stored in the D1 rows.

## Improvement opportunities and missing features

| ID | Idea | Why | Effort | Sketch |
|---|---|---|---|---|
| SC-P-1 | Add a CI quality-gate workflow (.github/workflows/ci.yml) | Catches broken commits before tag time, runs the 271 Rust tests and the server tests/type-check, checks generated schemas, audits advisories/licenses/duplicates, and warms the rust-cache for releases. | M | On push to main and pull_request, `permissions: { contents: read }`, concurrency `ci-${{ github.ref }}` with cancel-in-progress. Job `rust` (windows-latest): `actions/checkout@11d5960a326750d5838078e36cf38b85af677262` (v4.4.0) with `persist-credentials: false`; `rustup show` (honours rust-toolchain… |
| SC-P-2 | Add Dependabot configuration | Keeps pinned action SHAs and cargo/npm dependencies current, so advisories like SC-11/SC-16 are surfaced automatically. | S | `.github/dependabot.yml` with weekly entries for `github-actions` (`/`), `cargo` (`/src-tauri`, `/tools/report`) and `npm` (`/`, `/server`, `/streamdeck`, `/streamdeck/com.einfabo.clippiboy.sdPlugin`). |
| SC-P-3 | Harden release.yml (protected environment, version guard, draft-then-publish, checksums, provenance) | Protects the signing key, prevents mislabelled or trial releases, removes the incomplete-latest window, and gives users checksums and build provenance. | M | Job `windows`: `environment: release` (secrets live there; required reviewers; tag rule v*), `permissions: { contents: write, id-token: write, attestations: write }`; checkout pinned by SHA with `persist-credentials: false`; a "Tag must match versions" pwsh step (if tag ref, `env: TAG`) that reads… |
| SC-P-4 | Harden ffmpeg.yml and ship license/source info with the ffmpeg ZIP | Removes the ref_name injection pattern and credential persistence, and meets GPL redistribution obligations with attested artifacts. | S | Use `env: TAG: ${{ github.ref_name }}` with `$env:TAG`, `persist-credentials: false` and SHA-pinned actions; add gyan's LICENSE/README and a `SOURCE.txt` pointing to the exact source into the ZIP, and attest it. |
| SC-P-5 | Fail ffmpeg.yml when extracted hashes are not pinned in tools.rs | Guarantees the published ffmpeg ZIP matches the SHA-256s the app pins, so a release cannot ship binaries the app would reject. | S | pwsh step: `$src = Get-Content src-tauri/src/tools.rs -Raw; foreach ($e in 'ffmpeg.exe','ffprobe.exe') { $h=(Get-FileHash "dist-ffmpeg/$e" -A SHA256).Hash.ToLower(); if ($src -notmatch $h) { throw "$e $h not pinned in tools.rs" } }`. |
| SC-P-6 | Change repository settings for release integrity | Restricts who can trigger releases, prevents release-asset replacement, and gates access to the signing key. | M | Add a tag ruleset for `v*` and `ffmpeg-*`, turn on immutable releases, create the `release` environment with required reviewers, and add Authenticode signing through `bundle.windows.signCommand`. |
| SC-P-7 | Mask profile paths in the diagnostic report and add a review-before-sharing notice | The report contains COMPUTERNAME, USERNAME, full config.json, full logs, GPU/OS details, Run/Startup entries, the ffmpeg directory tree and 30 days of event-log hits; masking and a notice reduce acci… | S | In tools/report/src/main.rs, mask user profile paths in collected output and print a "review before sharing" notice (alongside the existing `redact_token` at :213-243). |
