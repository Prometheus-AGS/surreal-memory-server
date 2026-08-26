---
id: embedding-executor
title: Embedding Executor
sidebar_position: 3
---

# Embedding Executor

Embedding runs in a **child process**, supervised over JSON on stdio.

## Why a separate process

Local embedding loads Candle plus a GPU backend. That stack can hang in FFI,
exhaust GPU memory, or abort in ways safe Rust cannot catch. In-process, any of
those takes down the server holding your agent's memory. Out of process, the
supervisor kills a replaceable child and spawns another.

The cost is a serialization boundary and a supervision protocol. That is the
trade the design accepts.

## Lifecycle

```mermaid
sequenceDiagram
  participant P as Supervisor
  participant C as Child process

  P->>C: spawn (embedding-executor)
  Note over C: exec, dynamic link,<br/>config, service construction<br/>(no output possible)
  C-->>P: Ready
  Note over P: startup budget satisfied;<br/>per-request watchdog now armed
  P->>C: request
  C-->>P: Progress "accepted"
  loop every 250ms
    C-->>P: Progress "working"
  end
  C-->>P: Completed / Failed
```

## Two budgets, not one

The `Ready` handshake exists because startup and serving have different
observability.

Before `Ready`, the child is **structurally incapable of emitting anything** —
its heartbeat interval does not exist until after it reads its first request.
Arming the per-request progress watchdog against that phase treats a healthy but
slow cold start as a hung process.

That failure was real. GPU initialization on one Apple Silicon host measured
**1.2s, 14s, 36s, and over 150s** on different boots, tracking system load. With
a single 30s watchdog covering startup, the supervisor SIGKILLed the child, the
call retried once, and the result was a ~63s failure (2 × 30s plus spawn and
kill overhead) with **no diagnostic** — because the child had never been able to
log anything.

| Variable | Default | Covers |
|---|---|---|
| `SURREAL_EXECUTOR_STARTUP_MS` | 300000 | spawn → `Ready` |
| `SURREAL_EXECUTOR_WATCHDOG_MS` | 30000 | per-request progress |

## Warmup

`EMBEDDING_WARMUP` defaults to **true**. Without it, the first user-facing
request pays the entire cold model load. Warmup moves that cost to startup, where
a failure only logs a warning and falls back to lazy loading.

## Model cache

hf-hub stores repositories under `<hf-home>/hub`. `MODEL_CACHE_DIR` names the
HuggingFace *home*, and the server appends `hub` when resolving it.

:::danger Cache path mismatch
Pointing hf-hub at the parent instead of `<parent>/hub` orphans an existing
cache. The symptom is a full re-download of the model weights on every executor
start, with no error — the download simply happens again. The resolved path is
logged on each load; confirm it ends in `hub`.
:::

Pre-populate with `./download-model.sh`, which writes the hub cache layout. A
flat directory of model files is **not** readable by hf-hub.
