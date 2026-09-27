# Archived agent-worktree work

Uncommitted work from two stale agent worktrees, saved 2026-09-27 before the
worktrees were removed. Nothing here is on `main`. Both patches were verified
with `git apply --check` against their base commit `4ed6b2c`.

| Worktree | Contents |
|---|---|
| `agent-a33dc9226abec49aa` | ~1,900 changed lines in storage, palace and the load harness, plus a new `crates/surreal-memory/src/storage/sessions.rs` (a read/write session-pool prototype, the idea behind change c5 in `surrealdb-3x-connection-model`). Much of the storage part predates and overlaps the merged c1–c3 work. |
| `agent-a8503823b2f30fd6f` | MCP handler/transport changes plus a new `tests/mcp_protocol_versions.rs`. |

Restore:

```bash
git worktree add ../restore 4ed6b2c
cd ../restore
git apply <path>/<name>-uncommitted-on-4ed6b2c.patch
tar -xzf <path>/<name>-untracked-on-4ed6b2c.tar.gz
```
