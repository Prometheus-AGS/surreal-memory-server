---
type: SessionRecord
id: karpathy-session-5bcc4e58915dc148
title: Karpathy session 5bcc4e58915d
tags:
- karpathy
- session-learning
sources:
- session:15118dbe-b1da-4071-8434-62d6e2c9af97
timestamp: 2026-08-26T06:58:29.937844+00:00
created_at: 2026-08-26T06:58:29.937844+00:00
updated_at: 2026-08-26T06:58:29.937844+00:00
revision: 0
---

## Delta

Two of four fixes done. Here's the significant finding so far:

**I found the actual root cause of issue #5, and it differs from every hypothesis in the issue.**

The issue guessed a hung lazy model *download*. The logs say otherwise — 24 watchdog kills across 23 generations, each landing **immediately after** the log line `Metal available, using GPU`:

```
08:21:47.652695  INFO Loading Candle embeddings model: BAAI/bge-small-en-v1.5
08:21:47.652791  INFO Metal available, using GPU
08:21:48.809166 ERROR operation processing paused operation_id=28c436e3...
```

`Self::get_device()` was called **directly in the async block** ([candle.rs:85](crates/surreal-memory/src/embeddings/candle.rs:85)), not on `spawn_blocking`. `Device::new_metal(0)` initializes the GPU stack through synchronous FFI, which blocks a tokio worker → starves the child's 250ms heartbeat → the parent's 30s watchdog declares it nonresponsive → **SIGKILL**.

That explains what the issue found inexplicable: "no panic, yet the process exits." It's not a panic and not the parent — it's the executor *child*, SIGKILLed by its own supervisor. And the reason the "Preparing lazy Candle embeddings" line always follows a failure is simply that a fresh child starts up.

Two fixes landed, both verified compiling:
1. **`cache_dir` wired into hf-hub** — `Api::new()` ignored it entirely, so `MODEL_CACHE_DIR` was dead config. Now uses `ApiBuilder::with_cache_dir`.
2. **Metal/CUDA init moved onto `spawn_blocking`** — the actual killer.

Plus a bounded download timeout (hf-hub 0.5 ships no connect/read timeout — verified in its source), configurable via `MODEL_DOWNLOAD_TIMEOUT_SECS`, default 600s.

I also caught my own bug mid-edit: I first wrapped an already-executing `try_join!` in `timeout`, which would never fire. Fixed to await lazily inside the timeout.

Remaining: default `EMBEDDING_WARMUP=true`, and the SIGTERM handler + panic hook for diagnosability.

Also worth noting — **PR #71 is now validated end-to-end**, not just by unit tests. Against your real ledger reconstructed from commit `7072370`, the fixed CLI reports `aliasConflicts: 1` where the old build reported `0`, and `--apply` refuses with the actionable message.

## Root Cause

No explicit root-cause section was captured; preserve this as a session record, not an inferred diagnosis.

## Corrective Actions

Review and promote only reusable findings.

## Session Metadata

- Harness: claude-code
- Session: 15118dbe-b1da-4071-8434-62d6e2c9af97
- Captured: 2026-08-26T06:58:28.547709Z
- Project: /Users/gqadonis/Projects/prometheus/surreal-memory-server

## Changed Paths

- .claude/settings.local.json
- .prometheus/knowledge/wiki/index.md
- .prometheus/knowledge/wiki/log.md
- .prometheus/knowledge/wiki/karpathy-session-01292e86f284184b.md
- .prometheus/knowledge/wiki/karpathy-session-10a9252284019f78.md
- .prometheus/knowledge/wiki/karpathy-session-34031cd6d43ba9c7.md
- .prometheus/knowledge/wiki/karpathy-session-3fc3c7eeb7d09395.md
- .prometheus/knowledge/wiki/karpathy-session-43bd4b6930d45ea9.md
- .prometheus/knowledge/wiki/karpathy-session-451238e4b7ffb324.md
- .prometheus/knowledge/wiki/karpathy-session-45aa5754db648111.md
- .prometheus/knowledge/wiki/karpathy-session-51f9c4e235dd261b.md
- .prometheus/knowledge/wiki/karpathy-session-585eaae338b196a4.md
- .prometheus/knowledge/wiki/karpathy-session-701d211d06b0ad54.md
- .prometheus/knowledge/wiki/karpathy-session-7507d85c683cd5c4.md
- .prometheus/knowledge/wiki/karpathy-session-7e95b31b80743f66.md
- .prometheus/knowledge/wiki/karpathy-session-8562a17c2277e920.md
- .prometheus/knowledge/wiki/karpathy-session-87bbdb2a738da798.md
- .prometheus/knowledge/wiki/karpathy-session-937ee31f03d082b3.md
- .prometheus/knowledge/wiki/karpathy-session-965133ca99862c8a.md
- .prometheus/knowledge/wiki/karpathy-session-a4581bc55ccc4c93.md
- .prometheus/knowledge/wiki/karpathy-session-a9af8f907daf546a.md
- .prometheus/knowledge/wiki/karpathy-session-cbc5d16e02f2bb51.md
- .prometheus/knowledge/wiki/karpathy-session-d0124063b8da3db5.md
- .prometheus/knowledge/wiki/karpathy-session-db7d80f2078fe176.md
- .prometheus/knowledge/wiki/karpathy-session-dc195d11ee752631.md
- .prometheus/knowledge/wiki/karpathy-session-e31032c14183b143.md
- .prometheus/project.json
