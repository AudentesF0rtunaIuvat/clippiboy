# Reproductions and experiments

Small programs the reviewers and verifiers wrote while checking findings. They were run in a Linux container during
the review; absolute paths were replaced with `<review-scratch>`, so adjust paths before running. Most Rust files
are standalone (`rustc file.rs`); the Python ones need numpy and Pillow; the server probes expect `wrangler dev` on
port 8799 with the seed from `server/scripts/e2e.mjs`.

## ST-1 deadlock: a MutexGuard temporary lives for the whole call

- [`tlt/t.rs`](tlt/t.rs)

## ST-15 tint.rs panics on a non-ASCII accent colour

- [`t/a.rs`](t/a.rs)

## LS-20 folder_name reserved-name gaps

- [`ls20/t.rs`](ls20/t.rs)

## AU-1 leftovers assignment: launcher tap also covers the game

- [`au1/main.rs`](au1/main.rs)
- [`au1sim/Cargo.toml`](au1sim/Cargo.toml)
- [`au1sim/src/lib.rs`](au1sim/src/lib.rs)
- [`au1sim/src/model.rs`](au1sim/src/model.rs)
- [`au1sim/src/audio/mod.rs`](au1sim/src/audio/mod.rs)

## AU-44 trim plan edge cases

- [`au44/t.rs`](au44/t.rs)

## ED-1 blur: box vs Gaussian contrast, deconvolution attack, 3-pass radii

- [`sim/blur.py`](sim/blur.py)
- [`sim/blur2.py`](sim/blur2.py)
- [`sim/deconv.py`](sim/deconv.py)
- [`sim/deconv3.py`](sim/deconv3.py)
- [`sim/disc.py`](sim/disc.py)
- [`sim/vis2.py`](sim/vis2.py)
- [`editor/blurcheck.py`](editor/blurcheck.py)
- [`editor/blursim.py`](editor/blursim.py)
- [`judge/radii.py`](judge/radii.py)
- [`judge2/radii_cmp.py`](judge2/radii_cmp.py)

## Watermark rectangle parity, Rust f32 vs TS f64

- [`sim/wm.py`](sim/wm.py)
- [`work/parity.py`](work/parity.py)

## Editor geometry extracted for testing (from the editor judge)

- [`judge/nt/geom.ts`](judge/nt/geom.ts)
- [`judge/nt/geom.test.ts`](judge/nt/geom.test.ts)

## Server: local wrangler-dev probes (suspension, relay budget, invisible probe, ranges, quotas)

- [`probe1.mjs`](probe1.mjs)
- [`probe2.mjs`](probe2.mjs)
- [`sv5x/probe.mjs`](sv5x/probe.mjs)

## Server: MP4 box checker CPU benchmark and edge cases (SV-29 and related)

- [`mp4bench.ts`](mp4bench.ts)
- [`bench/mp4bench.mjs`](bench/mp4bench.mjs)
- [`sv29/t.ts`](sv29/t.ts)

## FC-4 gallery: uncached toLocaleDateString benchmark

- [`bench.mjs`](bench.mjs)
- [`bench/b.mjs`](bench/b.mjs)

## FC-23 TypeScript strictness experiment (noUncheckedIndexedAccess, 51 errors)

- [`tsv/tsconfig.base.json`](tsv/tsconfig.base.json)
- [`tsv/tsconfig.nuia.json`](tsv/tsconfig.nuia.json)
- [`tsv/nuia.out`](tsv/nuia.out)
- [`tsv/tsconfig.nir.json`](tsv/tsconfig.nir.json)
- [`tsv/nir.out`](tsv/nir.out)
- [`tsv/tsconfig.vite.json`](tsv/tsconfig.vite.json)

## Gap hunt (unfinished): persisted-format compatibility harness across releases 0.7.5-0.8.3

- [`compat/Cargo.toml`](compat/Cargo.toml)
- [`compat/src/main.rs`](compat/src/main.rs)
- [`compat/src/v075.rs`](compat/src/v075.rs)
- [`compat/src/v076.rs`](compat/src/v076.rs)
- [`compat/src/v077.rs`](compat/src/v077.rs)
- [`compat/src/v078.rs`](compat/src/v078.rs)
- [`compat/src/v080.rs`](compat/src/v080.rs)
- [`compat/src/v081.rs`](compat/src/v081.rs)
- [`compat/src/v082.rs`](compat/src/v082.rs)
- [`compat/src/v083.rs`](compat/src/v083.rs)
