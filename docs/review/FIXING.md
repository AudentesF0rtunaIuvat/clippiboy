# Working through the findings

This folder is meant to be worked from directly, by you or by a coding agent. Every finding has a stable ID
(`ST-1`, `ED-1`, …) that appears in every file here. Line numbers refer to commit `417f434` (0.8.3); if the code
has moved since, search near the cited line.

## What is where

| Path | Use it for |
|---|---|
| [README.md](README.md) | Summary and the **Fix first** table: start here |
| `01-…` to `12-…` | One file per area. Each has verified issues with fix advice, then the "not independently re-verified" list, lower-severity issues, rejected claims, what is solid, and improvement ideas |
| [02-screenshot-editor-roadmap.md](02-screenshot-editor-roadmap.md) | Editor v2 plan; the full plans with code sketches are in [editor-plans/](editor-plans/) |
| [reports/](reports/) | The 11 original reviewer reports, unabridged: architecture overviews, reasoning, extra snippets (e.g. a ready `ci.yml` in `11-supply-chain.md`) |
| [data/verified/](data/verified/) | Per area, every finding **with the verifiers' evidence**: `votes[].evidence` quotes the exact lines read, `corrected_claim` says what is really true, `fix_assessment` judges the proposed fix |
| [data/extracted/](data/extracted/) | The findings as first extracted from the reports, with proposals and "found sound" notes |
| [data/duplicate-groups.json](data/duplicate-groups.json) | Which IDs were merged (the first ID in each group is the one kept) |
| [findings.json](findings.json) | Flat list of all 484 findings without evidence; handy for scripting or creating issues |
| [repro/](repro/README.md) | Reproductions written during verification (deadlock demo, blur inversion, server probes, benchmarks…) |

## Status meanings

- **confirmed / partial**: an independent verifier checked the code and agreed. For `partial`, follow
  `corrected_claim` rather than the original wording.
- **not independently re-verified**: the area reviewer quoted the code, but the second pass never got to it. Check it
  before changing anything (most medium findings are in this state).
- **unverifiable**: depends on Windows or WebView2 runtime behaviour; needs a test on a real machine.
- **refuted**: listed so nobody spends time on it.

## A good loop per finding

1. Read the entry in the area file, then its evidence in `data/verified/<area>.json`.
2. Reproduce it: use `repro/` where there is a script, or write the failing test first. The Rust unit tests run on
   Linux too (`cargo test --lib`); code behind `cfg(windows)` needs Windows.
3. Make the minimal fix, keeping the style of the surrounding code.
4. Run the checks:
   - `npm run build`
   - `cd server && npm run check && npm test`
   - `cd src-tauri && cargo test --lib && cargo clippy`
5. Mention the finding ID in the commit message.

Findings that touch the same code are best fixed together:

- Screenshot save safety: ED-2, ED-3, ED-4, ED-5, ED-6, EF-1, EF-6. See phase 0 of the editor roadmap.
- Redaction: ED-1, ED-11.
- Main-thread work and locks: ST-1, ST-2, LS-7, ST-12.
- File safety on trim and delete: AU-4, AU-5, LS-6.
- Server abuse: SV-1, SV-38, SV-2, SV-3, SV-12.
- Release pipeline: SC-1, SC-2, SC-3.

## Prompt for a Claude Code session

```
Read docs/review/FIXING.md and docs/review/README.md. Work through the "Fix first" table in order.
For each item: read its entry in the area file and its evidence in docs/review/data/verified/,
re-check the claim against the current code (especially items marked "not independently re-verified"),
write a failing test where the code allows it, make the minimal fix, run the checks listed in FIXING.md,
and commit with the finding ID in the message. Stop and ask before anything that changes stored data
formats, the server's database schema, or the release workflow.
```

## Listing findings quickly

```sh
# Every verified medium-or-higher finding, with ID, area and title
python3 -c "import json; [print(f['id'], f['final_severity'], f['slice'], f['title']) for f in json.load(open('docs/review/findings.json')) if f['status'] in ('confirmed','partial') and f['final_severity'] in ('critical','high','medium')]"

# Everything that touches one file
python3 -c "import json,sys; [print(f['id'], f['status'], f['title']) for f in json.load(open('docs/review/findings.json')) if any(sys.argv[1] in l for l in f['locations'])]" shot.rs
```
