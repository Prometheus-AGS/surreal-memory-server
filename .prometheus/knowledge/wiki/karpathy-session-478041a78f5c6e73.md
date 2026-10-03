---
type: SessionRecord
id: karpathy-session-478041a78f5c6e73
title: Karpathy session 478041a78f5c
tags:
- karpathy
- session-learning
sources:
- session:a529c600-97a5-4762-bd06-2049a9ec4fbd
timestamp: 2026-10-03T18:17:57.623522+00:00
created_at: 2026-10-03T18:17:57.623522+00:00
updated_at: 2026-10-03T18:17:57.623522+00:00
revision: 0
---

## Delta

Yes, everything that is actually running is on SurrealDB 3.3.0, and 3.3.0 is the newest release. There's one problem: if the database restarts, launchd will start the old 3.2.4 binary.

| Component | Version | Status |
|---|---|---|
| Rust SDK (`Cargo.toml` pins `surrealdb` and `surrealdb-types`, and `Cargo.lock` agrees) | 3.3.0 | Latest on crates.io |
| Docker images (`docker-compose.yaml`, `docker-compose-local.yml`) | v3.3.0, pinned by digest | Current |
| Running database server (pid 957, `/version` reports `surrealdb-3.3.0`) | 3.3.0, from `~/.prometheus/bin/surreal-3.3.0` | Current |
| `surreal` first on your PATH (`~/.prometheus/bin/surreal`) | 3.3.0 | Current |
| Newest SurrealDB release on GitHub | v3.3.0 (published 2026-09-28) | There is no 3.3.x patch or 3.4 tag yet |

**Needs fixing:** the database's launch agent, `~/Library/LaunchAgents/ai.prometheus.surrealdb-native.plist`, still starts `/opt/homebrew/bin/surreal`, which is Homebrew's 3.2.4. The running 3.3.0 process was started on Sep 27 from a different plist. The file on disk is now identical to the `.bak-3.2.4` backup, so it looks like the 3.2.4 copy was restored over the 3.3.0 one at some point. After a reboot or crash, launchd would bring the database back up on 3.2.4, against data that 3.3.0 has already written. The SDK 3.2.4 behaviour behind #35 doesn't apply here (the memory server is built with SDK 3.3.0), but downgrading the server binary is risky in its own right.

The fix is to point that plist at `~/.prometheus/bin/surreal-3.3.0` (data path and arguments unchanged), then reload the agent, which restarts the database briefly. Separately, you could `brew uninstall surreal` or unlink it so 3.2.4 can't be picked up by accident. I haven't changed either of these. Do you want me to?

Two smaller points:
- The installed `surreal-memory-server` (built Sep 27) already uses SDK 3.3.0, but it predates PR #31, so the fix for stuck operations isn't deployed yet.
- Nothing in the #36 work is invalidated. The harness and tests ran against a 3.3.0 server with SDK 3.3.0, and #35 is about binaries built with SDK 3.2.4, which are no longer in use.

I've stopped the #35/#36 work as you asked. Those changes are still uncommitted on `fix/36-ledger-timeouts`, and nothing has been pushed.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: a529c600-97a5-4762-bd06-2049a9ec4fbd
- Captured: 2026-10-03T18:17:57.131967Z
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
