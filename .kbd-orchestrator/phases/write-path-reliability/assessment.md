# KBD Assessment — Phase: write-path-reliability

- **Project**: surreal-memory-server
- **Phase goal**: Make the surreal-memory MCP write path reliable and observable
  end-to-end, so clients stop abandoning the tool and falling back to the
  filesystem. Covers the create/write path, MemPalace, and Mindmap subsystems.
- **Assessed**: 2026-05-17
- **Change backend**: OpenSpec (`openspec/` present, `project.json` confirms)
- **Triggering incident**: `authentic-digital-twin-content` substrate-extraction
  session — `create_task_stream` and `create_entity` each hung 4 minutes, then
  the MCP client timed out. The client logged the failure and built a
  filesystem-only fallback (`STATE.md`, Karpathy flat-file pattern).

---

## 0. Headline Verdict

**LARGE GAP.** This is not polish. A memory server whose two primary write
operations both hang for 4 minutes and force a client to abandon the tool is
failing at its core function. The prior `task-stream-reliability` phase fixed
*correctness* bugs in TaskStream; it did **not** fix *availability* of the write
path. The incident proves the write path is currently unusable under at least
one common configuration (`EMBEDDING_PROVIDER=local`, cold model cache).

> **Sycophancy correction applied.** The initial draft verdict called this a
> "small gap / fundamentally sound / light-touch phase" and dismissed the
> `create_task_stream` hang as "the same root cause bleeding over." That draft
> was corrected: `create_task_stream` does **no embedding** (verified at
> `surreal.rs:1830`), so its identical hang is the single most diagnostic fact
> in the log — it *rules out* the embedding model as the sole cause and points
> at a shared, upstream defect. Understating that is the S-03 pattern
> (gap minimization contradicting the report's own findings).

---

## 1. Phase Goals

| # | Goal | Rationale |
|---|------|-----------|
| G-1 | A write tool call either completes or fails with a clear error in **bounded time** (target ≤ 30s default). No silent multi-minute hangs. | The incident: 4-minute hang, no result, no error. |
| G-2 | Embedding model load (Candle local + Palace FastEmbed) must not block the first write open-endedly. Cold-cache load is observable and bounded. | First-use lazy load is the leading suspect for the embedding-dependent path. |
| G-3 | The write path is observable: progress is visible to the MCP client so it does not time out while real work is in flight. | MCP spec 2025-06-18 progress notifications / 2025-11-25 Tasks API. |
| G-4 | MemPalace and Mindmap write paths share the same bounded-time + observability guarantees. | Both flagged as recently error-prone. |
| G-5 | Retry/reconnect logic cannot compound into a multi-minute silent stall. | Nested retry amplification (see F-3). |

---

## 2. Evidence Gathered

### Codebase inspection
- `crates/surreal-memory/src/embeddings/candle.rs` — lazy Candle/BERT loader.
- `crates/surreal-memory/src/storage/surreal.rs` — `create_entity` (1269),
  `create_task_stream` (1830), `create_record` (1011), `retry_operation`,
  `reconnect`, `connect_with_retry`, `ensure_embedding_indexes` (573).
- `src/mcp/handlers.rs` — MCP tool handlers; `src/mcp/http.rs` — HTTP/SSE transport.
- `crates/surreal-memory/src/palace/context.rs`, `embedding.rs` — Palace init.
- `crates/surreal-memory/src/storage/migrations/mod.rs` — schema v1–v19.
- `docker-compose.yaml` — running config: `EMBEDDING_PROVIDER=local`,
  `LOCAL_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5`, HF cache bind-mounted.

### Runtime observation
- Current container (restarted this session) is **healthy**; migrations v18/v19
  applied; embedding service reports "ready (384 dimensions)" but note the log
  also says **"will verify on first use"** — the model is *not* actually loaded
  at startup.

### Web research (Tavily)
- **MCP Tasks API** (spec 2025-11-25, experimental): "submit now, poll later"
  for operations > 30s. `tools/call` + `task` → `tasks/get` / `tasks/result`.
- **MCP progress notifications** (spec 2025-06-18, stable): `notifications/progress`;
  clients with `resetTimeoutOnProgress: true` reset their timeout on each heartbeat.
- **Async handleId pattern**: return a tracking ID in < 1s; run slow work in a
  background task; client polls. Recommended for any tool with unpredictable latency.
- **Karpathy LLM Wiki pattern**: plain-markdown knowledge base an LLM maintains;
  the client's filesystem fallback (`STATE.md`) is exactly this. Community
  consensus: the *robust* version adds typed entities, hybrid (BM25+vector+graph)
  search, auto-ingest hooks, and quality scoring on top of flat files — i.e.
  precisely what surreal-memory already provides *if the write path works*.

---

## 3. Findings (Gap Report)

### C-1 — CRITICAL — First write blocks open-ended on cold-cache model load

The embedding-dependent write path (`create_entity` → `embed_entity` →
`CandleEmbeddings::embed` → `ensure_loaded`) loads the BERT model **lazily on
the first embed call** (`candle.rs:67` `get_or_try_init`). On a cold cache,
`ensure_loaded` calls `download_model` (`candle.rs:156`), which does **three
sequential HuggingFace Hub downloads** (`config.json`, `tokenizer.json`,
`model.safetensors`) over the network with **no timeout, no progress reporting,
and no bound** (`candle.rs:169–179`). The first `create_entity` call therefore
blocks on an unbounded network download while the MCP client sees nothing. This
is the leading cause of the `create_entity` arm of the incident.

- **Severity**: CRITICAL — unbounded block on the primary write path.
- **Files**: `candle.rs:67–133`, `156–184`; `surreal.rs:1283`.

### C-2 — CRITICAL — `create_task_stream` hangs despite doing NO embedding

`create_task_stream` (`surreal.rs:1830`) sets timestamps and calls
`create_record` — it never touches the embedding service. Yet it hung the
**identical 4 minutes**. This proves a **shared, non-embedding root cause** on
the write path. The most probable shared cause: connection acquisition / retry
amplification (see F-3), or a write transaction stalling against SurrealDB while
the embedding model load (C-1, running concurrently for another call) saturates
CPU/IO on the same single-threaded-ish path. Either way, the write path has a
shared failure mode that the embedding fix alone will **not** resolve.

- **Severity**: CRITICAL — proves the gap is structural, not model-specific.
- **Files**: `surreal.rs:1830–1839`, `1011–1049`.

### H-1 — HIGH — No MCP progress / Tasks support: every slow call is a silent hang

The MCP layer (`src/mcp/handlers.rs`, `src/mcp/http.rs`) returns tool results
synchronously. There is no `notifications/progress` emission and no Tasks-API
"submit now, poll later" path. Any operation exceeding the client's timeout
(here ~4 min) appears as a dead server. Per current MCP spec, slow tools
(model load, Palace ingest, large mindmap update, `auto_summarize`,
`compress_memories`) should emit progress heartbeats or be task-augmented.

- **Severity**: HIGH — turns every slow-but-working call into a perceived crash.
- **Files**: `src/mcp/handlers.rs`, `src/mcp/http.rs:92–148`.

### H-2 — HIGH — Nested retry amplification produces multi-minute silent stalls

`create_record` → `retry_operation` retries up to `max_operation_retries` (3).
On a retriable error it calls `reconnect()` → `connect_with_retry`, which loops
up to `max_connect_retries` (**10**, per `docker-compose.yaml`) with backoff up
to 5s each. Worst case: 3 × (10 × ~5s connect + per-attempt op time) ≈ **150s+**
of blocking, all silent, before any error surfaces. The `TIMEOUT 30s` on the SQL
(`surreal.rs:1030`) bounds the *query* but not this *outer* amplification.

- **Severity**: HIGH — a defect, not "defensive depth"; directly explains a
  multi-minute hang with no result.
- **Files**: `surreal.rs` `retry_operation`, `reconnect`, `connect_with_retry`.

### H-3 — HIGH — MemPalace first call blocks on FastEmbed model init, unbounded

`PalaceContext::from_storage` (`palace/context.rs:58`) calls
`FastEmbedService::new().await` → `mempalace_core::embedder::FastEmbedder::new_default()`
(`palace/embedding.rs:21`), which downloads the all-MiniLM-L6-v2 FastEmbed model
(~25–80 MB) on first use. Same pattern as C-1: unbounded, unobserved network
load on the first `palace_*` call. CLAUDE.md documents the ~300ms warm path but
not the cold-download path.

- **Severity**: HIGH — same hang class as C-1, second subsystem.
- **Files**: `palace/context.rs:40–78`, `palace/embedding.rs:18–25`.

### H-4 — HIGH — Embedding load happens on the async runtime without `spawn_blocking`

`ensure_loaded` performs CPU-heavy, synchronous work — `Tokenizer::from_file`,
`std::fs::File::open` + `serde_json::from_reader`, `VarBuilder::from_pth` /
`from_mmaped_safetensors`, `BertModel::load` (`candle.rs:88–121`) — directly
inside an `async` block on a tokio worker thread, with **no `spawn_blocking`**.
The subsequent forward pass in `embed_internal` is likewise sync CPU work under
an `async` mutex (`candle.rs:286–382`). This blocks a runtime worker thread for
the entire model load + inference, starving other tasks (including, plausibly,
the concurrent `create_task_stream` of C-2).

- **Severity**: HIGH — runtime starvation; couples unrelated calls together.
- **Files**: `candle.rs:67–133`, `270–389`.

### M-1 — MEDIUM — Mindmap large-graph mitigation is a warning, not a fix

CLAUDE.md claims a "30-second query timeout" mitigation for mindmaps > 500
nodes. Inspection shows `add_mindmap_node` / `add_mindmap_edge`
(`surreal.rs:2703`, `2733`) only emit a `tracing::warn!` — there is **no
`TIMEOUT` clause** on the mindmap `UPDATE` path observed (the `TIMEOUT 30s`
exists on `create_record`, a different code path). The whole node/edge array is
still rewritten on every add (`append_mindmap_node`). For > 500 nodes this is a
real latency cliff with no enforced bound. The documented mitigation is
**partially missing in code**.

- **Severity**: MEDIUM — degraded, not dead; but doc/code drift is itself a risk.
- **Files**: `surreal.rs:2703–2745`, `append_mindmap_node`; CLAUDE.md "Mindmap
  Performance Limitations".

### M-2 — MEDIUM — No startup health gate / warmup for embedding providers

The server reports "Embedding service ready" at startup (`candle.rs:39`) when in
fact nothing is loaded ("will verify on first use"). There is no optional
warmup embed and no `/health` differentiation between "process up" and "write
path actually ready." A client cannot tell a working server from one about to
hang for 4 minutes. The incident log's own recommendation ("verify health
before attempting writes") is currently impossible to satisfy.

- **Severity**: MEDIUM — observability gap; enables the bad client experience.
- **Files**: `candle.rs:34–50`; `src/mcp/http.rs` `/health`.

### L-1 — LOW — Sequential HF downloads + no shared download lock

`download_model` fetches three files sequentially (`candle.rs:169–179`); they
could be concurrent. If two embed calls race before `OnceCell` resolves,
`get_or_try_init` serializes them correctly, but the *losing* caller still waits
the full cold-load duration with no signal.

- **Severity**: LOW — efficiency / latency polish.
- **Files**: `candle.rs:156–184`.

---

## 4. Severity Roll-up

| Severity | Count | Findings |
|----------|-------|----------|
| CRITICAL | 2 | C-1, C-2 |
| HIGH     | 4 | H-1, H-2, H-3, H-4 |
| MEDIUM   | 2 | M-1, M-2 |
| LOW      | 1 | L-1 |

**Root-cause cluster**: C-1 + H-3 + H-4 are one defect family — *unbounded,
unobserved, runtime-blocking model loads*. C-2 + H-2 are a second family —
*shared write-path stalling with no bounded failure*. H-1 + M-2 are the third —
*the client cannot see or survive either of the above*.

---

## 5. Recommended Direction for Plan Phase

Three ordered change-clusters (Plan phase decides exact OpenSpec change IDs):

1. **Bound and observe model loading** (C-1, H-3, H-4, L-1, M-2): move model
   load + inference off the async runtime via `spawn_blocking`; add a bounded
   timeout + clear error to the HF download; add an explicit, optional
   startup warmup; make `/health` reflect true write-path readiness.
2. **Eliminate write-path silent stalls** (C-2, H-2): cap total
   retry+reconnect wall-clock at a single bounded budget; ensure every write
   tool returns either a result or a typed error within that budget; instrument
   the connection-acquisition path.
3. **Make slow tools non-hanging at the MCP layer** (H-1, M-1): adopt MCP
   `notifications/progress` heartbeats for slow tools (model load, Palace
   ingest, mindmap update, `auto_summarize`, `compress_memories`); enforce the
   documented mindmap `TIMEOUT`; evaluate the experimental Tasks API for the
   genuinely long operations.

**Karpathy-method note**: the client's filesystem fallback *is* the Karpathy
LLM Wiki pattern. surreal-memory's value proposition is the robust superset
(typed entities, hybrid BM25+HNSW+graph, auto-ingest) the research says a mature
wiki needs. That value is only realized if the write path is reliable — closing
this phase is what makes the Karpathy fallback unnecessary rather than required.
A separate spec'd path for clean ingestion of an existing `docs/digital-twin-travis/`
flat-file substrate is reasonable as a follow-up, not part of this phase.

---

## 6. Out of Scope (note for Plan / follow-up)

- UAR consumer resync (still the standing `task-stream-reliability` follow-up).
- Pre-existing `tests/contract_alignment_test.rs:300` compile error.
- TaskStep REST endpoints; REST JWT/header scope extraction.
- Re-ingestion tooling for the `authentic-digital-twin-content` substrate docs.

---

## Phase Status: ASSESSMENT COMPLETE

Gap report only — no code, no change structures produced (KBD strict phase
ordering). Next step: `/kbd-plan write-path-reliability`.
