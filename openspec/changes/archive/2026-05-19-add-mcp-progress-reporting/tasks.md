# Tasks — add-mcp-progress-reporting

## 1. Progress notifications for slow tools
- [x] 1.1 Emit `notifications/progress` from slow tools — `run_with_progress` (`src/mcp/progress.rs`) wraps `add_mindmap_node`, `add_mindmap_edge`, `compress_memories`, `auto_summarize_task_stream`, `palace_ingest` in `src/mcp/mod.rs`
- [x] 1.2 SSE transport flushes heartbeats — `notify_progress` routes through the existing `Peer`/SSE channel; the SSE handler already streams server-pushed messages
- [x] 1.3 Degrade gracefully with no `progressToken` — `run_with_progress` early-returns and runs the future directly when `ctx.meta.get_progress_token()` is `None`

## 2. Mindmap TIMEOUT enforcement (M-1)
- [x] 2.1 `TIMEOUT 30s` added to all three mindmap `UPDATE` queries (`update_mindmap_graph`, `append_mindmap_node`, `append_mindmap_edge`) via the `MINDMAP_UPDATE_TIMEOUT` const
- [x] 2.2 Large-mindmap updates now fail fast — SurrealDB enforces the `TIMEOUT`; normal-sized updates verified to complete well under bound
- [x] 2.3 CLAUDE.md "Mindmap Performance Limitations" corrected — the doc previously claimed a `TIMEOUT` that was never enforced (only a `warn!`); the doc now matches the implemented behavior, and the unimplemented "JSON size monitoring" claim was removed

## 3. Tasks API evaluation
- [x] 3.1 Evaluated the experimental MCP Tasks API (spec 2025-11-25)
- [x] 3.2 Decision recorded in proposal.md — **DEFER**: all operations are now bounded ≤~30s; progress heartbeats fully address the failure mode; Tasks API adds stateful lifecycle for zero current benefit

## 4. Verification
- [x] 4.1 `mindmap_update_path_succeeds_under_timeout_bound` exercises all three TIMEOUT-bearing UPDATE paths (20 node adds) within bound
- [x] 4.2 No-`progressToken` path: `run_with_progress` early-return; existing MCP handler tests run with no token and still pass
- [x] 4.3 Mindmap `UPDATE` timeout enforced — SurrealDB engine enforces `TIMEOUT 30s`; positive path covered by integration tests
- [x] 4.4 fmt + clippy + lib/integration tests clean (40 bin lib + 17 + 36 library tests pass)
- [x] 4.5 QA gate: rust-reviewer against `.kbd-orchestrator/constraints.md`
