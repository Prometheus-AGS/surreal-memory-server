---
type: SessionRecord
id: karpathy-session-fa1c1b3f3e325c5d
title: Karpathy session fa1c1b3f3e32
tags:
- karpathy
- session-learning
sources:
- session:a529c600-97a5-4762-bd06-2049a9ec4fbd
timestamp: 2026-10-03T18:57:02.433037+00:00
created_at: 2026-10-03T18:57:02.433037+00:00
updated_at: 2026-10-03T18:57:02.433037+00:00
revision: 0
---

## Delta

Everything on this machine now runs SurrealDB 3.3.0 (the latest release), the memory server runs current `main` (`cc40136`), and the 20 operations stuck since #30 have all committed.

**Final state:**

| Check | Result |
|---|---|
| Database server | `surrealdb-3.3.0` from `~/.prometheus/bin/surreal`, and its plist now points there |
| Homebrew `surreal` | Uninstalled, `surrealdb/tap` removed; `which -a surreal` shows only the 3.3.0 binary |
| Memory server | `cc40136`, identical binary in `/usr/local/bin` and `~/.local/bin`; `/health` ok, `/ready` true for every capability |
| Network binding | `127.0.0.1:23001` only, with `API_HOST=127.0.0.1` in the running job |
| Stuck operations | All 20 committed within about 3 minutes of deploy; nothing non-terminal remains |
| Smoke test | A new `add_memory` through `/api/v2/operations` reached `committed` |
| Errors since deploy | 0 paused operations, ledger timeouts, connection replacements, namespace errors or request-id mismatches |
| Leftover 3.2.4 references | None in runtime files, docs or plists; only an archived historical patch |

**What I merged:**
- **[prometheus-skill-system#119](https://github.com/Prometheus-AGS/prometheus-skill-system/pull/119):** this was the real cause of the 3.2.4 plist. The installer now prefers `~/.prometheus/bin/surreal` and refuses to render a plist for any version other than 3.3.0. The memory-server template now includes `API_HOST=127.0.0.1`.
- **[#37](https://github.com/Prometheus-AGS/surreal-memory-server/pull/37):** the ledger now replaces a session that has lost its namespace, closing #35. #36 stays open with the load-test findings.
- **[#38](https://github.com/Prometheus-AGS/surreal-memory-server/pull/38):** consistency fixes:
  - `.env.example` no longer sets `SURREAL_MODE` twice and documents every variable the server reads.
  - The website docs have corrected defaults and a local deploy guide.
  - `build.sh` now includes `palace`, and `mcp.sh` no longer points at another clone.

**Things you should know:**
- **Two short outages.** The database was down about 45 s and the memory server about 50 s while they reloaded. Both times launchd's first bootstrap failed because the old process hadn't exited yet; the retry worked. MCP clients lost their connection during the restart, and you may need to reconnect `surreal-memory` in `/mcp`.
- **Homebrew removed some unrelated libraries.** The first `brew uninstall` attempt failed on the untrusted tap, but Homebrew's automatic cleanup still removed orphaned libraries, at least libsndfile, libudfread and flac. Nothing installed depends on them (`brew missing` is clean). If you used the `flac` command line tool directly, reinstall it with `brew install flac`.
- **I briefly broke, then restored, three other plists.** I misused `--exclude` (it takes names without `ai.prometheus.`), so pk-cherry, forge-mcp and prometheus-nudge were re-rendered pointing at a temporary checkout. I re-rendered them from their real source before anything reloaded them, and I added the lesson to `docs/lessons.md`.
- **I fast-forwarded the skill-pack install checkout.** `worktrees/dist-ship-script-lib`, which your services are installed from, was a clean `main` five commits behind. Bringing it up to date also pulled in two already-merged delivery-cadence fixes.
- **Backups**, all from before the changes, in case you need to roll back:
  - plists and binaries saved as `*.bak-pre-consistency-20261003-132620`
  - a full database export at `~/.prometheus/backups/surrealdb-memory-mcp-20261003-132620.surql` (permissions set to 600)

**Not touched:**
- The regenerated openspec/`.agents` files, the `.kbd-orchestrator` progress change and your local `.claude/settings.local.json` in this repo. They are still uncommitted.
- Your skill-pack checkout on `codex/delivery-cadence-recovery`.

Still to do are #34 (executor cancel-safety), #32 (retry/reject) and #33 (stats), each as its own deploy.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: a529c600-97a5-4762-bd06-2049a9ec4fbd
- Captured: 2026-10-03T18:57:00.205551Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/commands/opsx/apply.md
- .claude/commands/opsx/archive.md
- .claude/commands/opsx/bulk-archive.md
- .claude/commands/opsx/continue.md
- .claude/commands/opsx/explore.md
- .claude/commands/opsx/ff.md
- .claude/commands/opsx/new.md
- .claude/commands/opsx/onboard.md
- .claude/commands/opsx/sync.md
- .claude/commands/opsx/verify.md
- .claude/settings.local.json
- .claude/skills/openspec-apply-change/SKILL.md
- .claude/skills/openspec-archive-change/SKILL.md
- .claude/skills/openspec-bulk-archive-change/SKILL.md
- .claude/skills/openspec-continue-change/SKILL.md
- .claude/skills/openspec-explore/SKILL.md
- .claude/skills/openspec-ff-change/SKILL.md
- .claude/skills/openspec-new-change/SKILL.md
- .claude/skills/openspec-onboard/SKILL.md
- .claude/skills/openspec-sync-specs/SKILL.md
- .claude/skills/openspec-verify-change/SKILL.md
- .kbd-orchestrator/phases/surrealdb-connection-architecture/progress.json
- .opencode/commands/opsx-apply.md
- .opencode/commands/opsx-archive.md
- .opencode/commands/opsx-bulk-archive.md
- .opencode/commands/opsx-continue.md
- .opencode/commands/opsx-explore.md
- .opencode/commands/opsx-ff.md
- .opencode/commands/opsx-new.md
- .opencode/commands/opsx-onboard.md
- .opencode/commands/opsx-sync.md
- .opencode/commands/opsx-verify.md
- .opencode/skills/openspec-apply-change/SKILL.md
- .opencode/skills/openspec-archive-change/SKILL.md
- .opencode/skills/openspec-bulk-archive-change/SKILL.md
- .opencode/skills/openspec-continue-change/SKILL.md
- .opencode/skills/openspec-explore/SKILL.md
- .opencode/skills/openspec-ff-change/SKILL.md
- .opencode/skills/openspec-new-change/SKILL.md
- .opencode/skills/openspec-onboard/SKILL.md
