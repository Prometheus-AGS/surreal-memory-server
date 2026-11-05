---
type: research-report
title: SurrealDB Agent Memory vs surreal-memory-server — feature and gap analysis
query: Research the agent memory feature SurrealDB provides in its cloud offering and compare it with this repository's features
date: 2026-11-05
verification_status: partial
confidence: 0.75
sources_count: 22
---

# SurrealDB Agent Memory vs surreal-memory-server

**Date:** 2026-11-05 · **Repo state read:** branch `chore/surrealdb-4.0` (base `7385227`, v1.10.2 → 1.11.0 in progress) · **Status:** partially verified (see [Method and limits](#method-and-limits))

## 1. Summary

SurrealDB shipped **SurrealDB Agent Memory** on 2026-10-14. It is a *managed memory and knowledge service* that runs as an application tier in front of SurrealDB (Cloud, or Enterprise self-hosted), and it was positioned as part of the SurrealDB 4 launch on 2026-11-03. It is a different kind of product from this repository: Agent Memory is an opinionated, LLM-driven pipeline (extract → reconcile → store → recall) behind an authenticated API, while `surreal-memory-server` is an **embeddable Rust library plus an unauthenticated MCP/REST server** that stores what callers hand it and offers graph, task and palace tooling around it.

The largest gaps, in priority order:

| # | Gap | Severity |
|---|---|---|
| G1 | No authentication or authorization on the REST/MCP server; scope filters are caller-supplied | **High** |
| G2 | No extraction: conversations are stored as raw rows, with no entities, attributes or instructions | **High** |
| G3 | No reconciliation: only a 0.92 similarity de-duplication, with no supersession, conflict or uncertainty handling | **High** |
| G4 | No bi-/tri-temporal model on memories (`valid_until` only) | Medium |
| G5 | No provenance object and no retrieval/decision traces | Medium |
| G6 | Hybrid search's BM25 leg has no full-text index defined; no graph signal in memory recall | Medium (verify) |
| G7 | No per-category decay or default expiry, no purge/erasure guarantee, no subtree forget (an hourly TTL worker does enforce `valid_until`) | Medium |
| G8 | No reflect/profile/chat surfaces; no SDKs, framework adapters or UI | Low–Medium |

Where this repository is **ahead of or different from** Agent Memory: an in-process embeddable library (Agent Memory has none), fully self-hosted operation with no token metering, local embeddings, an exposed graph-traversal toolset, TaskStreams, mindmaps, Memory Palace, and a durable operation ledger. Section 5 covers these.

## 2. What SurrealDB Agent Memory is

*All claims in this section come from SurrealDB's own documentation and blog (vendor-authored; see [sources](#7-sources)). Where the vendor makes a performance claim, it is marked as self-reported.*

**Product shape.** "A memory and knowledge layer for AI agents. One SurrealDB substrate, provenance-first, trace-aware, and tri-temporal." It runs as an application tier in front of SurrealDB; each write is one ACID transaction across records, relations and embeddings [S2, S4]. The SurrealDB 4 post calls it "a separate, managed service built on the same engine" [S3]. The earlier codename was **Spectron** (limited preview, waitlist); current docs still carry `SPECTRON_*` environment variables and an `X-Spectron-On-Behalf-Of` header [S6, S13].

**Isolation unit.** A **Context** has its own SurrealDB namespace and database. Contexts, API keys, conversations, entities and the knowledge graph are managed in **SurrealDB Studio** [S2].

**Integration surfaces.**
- REST: `/api/v1/{context_id}/...` with Bearer API keys and optional `Idempotency-Key` [S10].
- MCP at `/mcp`, seven tools: `remember`, `recall`, `context`, `reflect`, `forget`, `upload`, `inspect` [S9].
- SDKs: Python and TypeScript (`@surrealdb/memory`); docs also list Go, Swift and Kotlin pages. CLI `agent-memory`.
- Adapters: LangChain, Vercel AI SDK, OpenAI Agents, Pydantic AI, n8n, Mastra, a Claude Code hook, and a `claude-agent-memory` package [S2, S17, S18].
- Install guides for OpenCode, Zed, Antigravity and other coding assistants [S2].

**Memory model.** Eight "pillars"; six categories [S5]:
- *Episodic*: the raw session/turn record.
- Five extracted categories: *identity*, *knowledge*, *context*, *instructions*, *uncertainty*.
- Two streams: *authoritative* (curated documents, higher trust) and *experiential* (conversation-derived). Conflicts between them are surfaced, not averaged [S14].
- Facts are entity–attribute–relation records, each carrying a `source` object (kind, ref, span, confidence) [S15].

**Scoping and access.** A scope **tree** (`org/acme/user/alice`) with grants `memory:read`, `memory:write`, `memory:forget` over a node or subtree (`org/acme/*`); reads take a `lens` that can only narrow a grant; keys are bound to one Context and one principal and can only narrow it; delegated identity (`X-Spectron-On-Behalf-Of`, depth 1). "Per-verb grants, deny by default" [S6, S7, S9].

**Lifecycle.**
- **Decay** per category per daily pass: identity 1.0 with no decay; knowledge starts 0.8, ×0.995/day; context starts 0.5, ×0.95/day, and a context fact expires (`valid_until`) after about 7 days [S8].
- **Supersession:** a correction closes the previous value with `valid_until`, and history stays queryable [S11].
- **Forget** is soft by default; `purge: true` erases content and fact history; `POST /scopes/forget` hard-deletes a whole subtree. Traces keep a record that a forget happened, without the content [S16].
- Background **elaborate**, **consolidate** and **fsck** passes [S10].

**Temporal model.** Tri-temporal: *valid time*, *known time* and *system time*. An `asOf` read returns what memory *believed* at an instant, separate from what was true then [S12].

**Retrieval.**
- Seven fused signals; for the vector and BM25 lists the docs show reciprocal-rank fusion with k=60 over a pool of 256 candidates. Default `k`=10, capped at 50 [S19].
- Response tiers `direct`, `cache`, `hybrid`, `escalated`; document-section expansion; near-duplicate suppression.
- `context` returns a pre-formatted LLM block; `chat` composes recall and synthesis with citations; `profile` aggregates `selfFacts`, static, dynamic, preferences and instructions; `reflect` synthesizes insights and can persist them [S10, S20, S21, S22].
- Every retrieval, reconciliation and chat response writes a **trace** (retrieval/decision/response) as graph nodes [S15].

**Models.** LLM stages (extraction, synthesis, elaboration/consolidation; reconciliation makes no model call) accept `google`, `openai`, `anthropic`, `cerebras` or `openrouter`. Embeddings are **fixed to `gemini-embedding-2` at 3072 dimensions**; no other value is accepted [S13, S23].

**Availability and price.** Managed on SurrealDB Cloud: *Sandbox* free (3M tokens, one-off), *Lite* (price not captured), *Plus* $1,100/month (40M tokens/month), *Enterprise* custom (bring-your-own-model, single tenant, **self-hosted and air-gapped**). Tokens are metered on every LLM and embedding call, including background work [S6]. There is **no supported in-process library**: "Rust, Python, and TypeScript agents call the HTTP API or SDK against a deployed instance" [S24].

**Benchmarks (self-reported).** 77.2% on BEAM at 1M tokens with the official judge prompt; 57.1% when counted strictly (every rubric item met in full) [S3, S25]. Judge model naming differs between pages (see [contradictions](#6-contradictions-and-unknowns)).

**Adjacent SurrealDB 4 context.** The engine itself now offers vector, graph, document and full-text retrieval in one query and one transaction, and vendor figures of 7.4× faster vector index builds and filtered vector search "from 27 ms to 1.1 ms" (self-reported) [S3, S26]. This matters to us directly: we just moved to SDK/server 4.0.0.

## 3. What surreal-memory-server provides today

*From code read on this branch; static reading, not runtime-verified. File references are to this repository.*

| Area | What exists | Where |
|---|---|---|
| Scoped memory | `MemoryScope` Global/Agent/User/Session (+ Task per CLAUDE.md); `MemoryType` Episodic/Semantic/Procedural (+ Associative per CLAUDE.md); `importance`, `access_count`, `last_accessed_at`, `valid_until`, `version`, `categories`, free-form `metadata` | `crates/surreal-memory/src/memory.rs` |
| Conversation intake | `add_memories_from_conversation` stores each message as an **Episodic** memory tagged `conversation`/role; no extraction | `storage/surreal.rs:2926` |
| De-duplication | Cosine ≥ 0.92 merges near-duplicates at write | `storage/surreal.rs:2072, 2409` |
| Retrieval | `search_memories`, `hybrid_search_memories` (HNSW + a `content @@ $query` leg, weighted RRF-style merge), KNN with scope pushdown, query-embedding cache | `storage/surreal.rs:1424, 2705` |
| Audit/history | `get_memory_history` (audit log), `version` | `storage/mod.rs:93` |
| Expiry | Hourly TTL worker calls `expire_stale_memories` to soft-delete memories past `valid_until` (`TTL_INTERVAL_SECS`) | `src/workers/ttl.rs`, `storage/mod.rs:141` |
| Compression | `compress_memories`, `auto_summarize_task_stream` (re-embed summaries) | `storage/mod.rs:122` |
| Knowledge graph | Entities, relations, observations; `semantic_search`, `find_path`, `expand_neighbors`, `get_related`, `get_entity_history`, `get_graph_at_time` | `src/mcp/mod.rs` |
| Task state | TaskStreams, TaskSteps, `get_context_for_task` (token-budgeted) | `task_stream.rs`, `task_step.rs` |
| Mindmaps | Create/export, persona and ideation generators | `mindmap.rs` |
| Memory Palace (opt-in) | Drawers (384-d), wing/room/hall taxonomy, `palace_hybrid_search` (RRF across memory, entity, drawers) | `palace/` |
| Durable operations | Operation ledger with events, retry/reject, supervised embedding executor, operation-ID/hash replay | `src/operations.rs`, `src/executor.rs` |
| Transports | MCP (stdio/HTTP), REST, A2A (`/a2a/tasks/send`, `/agent.json`), Claude Code hook | `src/mcp`, `src/api`, `src/hook.rs` |
| Embeddings | OpenAI, Cohere, local (candle, supervised executor), FastEmbed | `embeddings/` |
| Library | Embeddable crate behind the `MemoryStorage` trait; consumed by UAR | `crates/surreal-memory` |
| Auth | **DB credentials only** (`SurrealAuthLevel`); the REST/MCP server has no authentication, so deployments bind `127.0.0.1` | `storage/auth.rs`, `CLAUDE.md` |

## 4. Gap analysis

Status key: **Gap** = absent; **Partial** = something exists but covers less; **Parity** = comparable; **Ahead** = we do more.

### 4.1 Feature matrix

| Capability | SurrealDB Agent Memory | surreal-memory-server | Status |
|---|---|---|---|
| Authentication | Bearer keys bound to one Context and principal | None on REST/MCP | **Gap (G1)** |
| Authorization | Per-verb grants over a scope tree, deny by default; narrowing-only lens; delegation | `user_id`/`agent_id`/`session_id` request filters | **Gap (G1)** |
| Tenant isolation | Context = own namespace + database | One namespace/db per deployment; test tenants only by naming | **Gap** |
| Structured extraction | LLM extraction of entities, attributes, relations, instructions, uncertainties; windowed (8,000-char windows with 256 overlap) | None; conversation → Episodic rows | **Gap (G2)** |
| Document ingestion | `upload` → chunk, extract, index; document-only query | `palace_ingest` (drawers) only | **Partial** |
| Memory categories | Identity / knowledge / context / instructions / uncertainty + episodic | Semantic / Episodic / Procedural / Associative; free-form `categories` | **Partial** |
| Reconciliation | Supersession with confidence floor; uncertainty records; authoritative vs experiential | 0.92 cosine merge; `version` counter | **Gap (G3)** |
| Temporal model | Valid / known / system time; `asOf` reads | `created_at`, `updated_at`, `valid_until`; `get_graph_at_time` | **Partial (G4)** |
| Provenance | `source` {kind, ref, span, confidence} on every fact | `metadata` JSON by convention | **Gap (G5)** |
| Traces | retrieval / decision / response traces as graph nodes | Operation-ledger events only | **Gap (G5)** |
| Hybrid retrieval | Seven signals fused (vector + BM25 + graph + more) | HNSW + `@@` leg; **no full-text index in migrations** | **Partial (G6)** |
| Graph traversal | Via retrieval signal | Dedicated tools: `find_path`, `expand_neighbors`, `get_related` | **Ahead** |
| Context assembly | `context` (formatted block), `chat`, `profile` | `get_context_for_task` (token-budgeted); UAR `context_builder` | **Partial (G8)** |
| Reflection | `reflect` (+ optional persist) | None | **Gap (G8)** |
| Consolidation | `elaborate`, `consolidate`, `fsck` background passes | `compress_memories`, `auto_summarize_task_stream` | **Partial** |
| Decay / expiry | Per-category daily decay; context facts expire at ~7 days | Hourly TTL worker expires memories past `valid_until`; no per-category decay or default expiry; static `importance`; recency scoring is consumer-side | **Partial (G7)** |
| Corrections | Supersede; old value kept for history | `update_memory` bumps `version` | **Partial** |
| Erasure | Soft forget, `purge: true`, subtree hard delete, audit retained | `delete_memory`, `delete_all_memories` (hard delete) | **Partial (G7)** |
| Idempotent writes | `Idempotency-Key` header | Operation-ID / hash replay in the v2 ledger | **Parity** |
| Entity aliasing | `same_as` resolves reads; nothing in the product writes it yet | Entity de-dup by name | Parity (both weak) |
| Embeddings | Fixed `gemini-embedding-2`, 3072-d, cloud | OpenAI, Cohere, local candle, FastEmbed; configurable dimension | **Ahead** |
| LLM stage choice | 5 providers per stage | No LLM stages in the service | n/a |
| SDKs / CLI | Python, TypeScript, Go, Swift, Kotlin; `agent-memory` CLI | REST and MCP only; Rust crate | **Gap (G8)** |
| Framework adapters | LangChain, Vercel AI, OpenAI Agents, Pydantic AI, Mastra, n8n, Claude hook | Claude Code hook; UAR | **Gap (G8)** |
| UI | SurrealDB Studio (conversations, entities, graph, traces) | None (docs site only) | **Gap** |
| Embeddable in-process | **None** | `surreal-memory` crate | **Ahead** |
| Self-hosted / air-gapped | Enterprise tier only | Default | **Ahead** |
| Cost model | Token-metered, from free Sandbox to $1,100/mo and up | Free; you pay your own embedding costs | **Ahead** |
| Published quality benchmark | BEAM 1M: 77.2% (57.1% strict), self-reported | None; load harness only | **Gap** |
| Task / workflow state | Not a feature | TaskStreams, TaskSteps | **Ahead** |
| Mindmaps, Palace | Not features | Present | **Ahead** |
| Agent-to-agent protocol | Shared Context memory | A2A endpoints | Different |

### 4.2 Gaps in detail

**G1: Authentication and authorization (High).** The server has no authentication; `CLAUDE.md` documents that loopback binding is the only control, and the container image defaults to `API_HOST=0.0.0.0`. Scope filters are chosen by the caller, so they are visibility filters rather than access control (the repo's own team notes say "identity filters are not authorization"). Agent Memory's model is the reference design: keys bound to a context and principal, grants over a path tree, narrowing-only lenses, deny by default. This is the one gap that is a *risk* rather than a missing feature, and it should come first for any non-loopback deployment.

**G2: No extraction (High).** Our service stores what it is given. Agent Memory's value comes from turning turns and documents into entities, attributes, relations, instructions and uncertainties. This is also the dependency for G3–G5 and most of G8. A design question precedes any work: does extraction belong *in* the service (needs an LLM stage, cost and latency on the write path) or in the consumer (UAR's `auto_capture`), with the service offering structured write endpoints? Agent Memory itself accepts pre-structured triples (`infer: "triples"`), which suggests the second path can be supported cheaply first.

**G3: No reconciliation (High).** We merge near-duplicates (cosine ≥ 0.92) and bump a `version`. We do not detect that "Alice is CTO" supersedes "Alice is CEO", close the old fact with an end date, keep both for history, or record an *uncertainty* when two sources disagree. Needs: structured facts first (G2), then valid-from/valid-until and supersession links. Note the repo's rule that v1 similarity de-duplication and v2 operation-ID replay stay distinct.

**G4: Temporal model (Medium).** We have `valid_until` and `created_at`/`updated_at`, plus `get_graph_at_time`. There is no `valid_from`, no "known time", and no way to ask what memory *believed* at an instant. A cheap first step is adding `valid_from` and `known_at` fields with migrations; the full `asOf` read comes with G3.

**G5: Provenance and traces (Medium).** Provenance is a convention in free-form `metadata`. Agent Memory has a typed `source` on every fact and graph-resident traces for each retrieval and decision. A typed provenance field (kind, ref, span, confidence) is a small schema change; traces are a larger design item. Our operation ledger already gives a durable events model that could be reused for retrieval traces.

**G6: Retrieval quality and index health (Medium; verify first).** `hybrid_search_memories` issues `content @@ $query` with `search::score(0)`, but `migrations/mod.rs:336` records that the full-text BM25 index was **omitted**, and `CLAUDE.md` still states both indexes must exist. I have not run this on 4.0; it may return nothing or an error, in which case "hybrid" is vector-only. SurrealDB 4 changes the full-text posting layout (index rebuilds are needed to correct term counts; per the `doc/UPGRADING.md` at the `v4.0.0` tag in surrealdb/surrealdb), so a proper index definition should be tested on 4.0 specifically. Separately, memory recall has no graph-traversal signal; the palace search does fuse sources with RRF and is the nearer model.

**G7: Lifecycle (Medium).** Expiry works: an hourly TTL worker (`src/workers/ttl.rs`) soft-deletes memories whose `valid_until` has passed. What is missing is anything that *sets* `valid_until` or lowers importance by category (no per-category decay curves or default expiry), a purge guarantee that removes history, and subtree forget. Right-to-erasure is the compliance-relevant piece: confirm whether `memory_history` rows survive `delete_memory` (not verified here).

**G8: Surfaces (Low–Medium).** No `reflect`, `profile` or `chat`; no SDKs, framework adapters or UI. The MCP gap is small: Agent Memory's seven tools map onto ours as `remember`→`add_memory`, `recall`→`hybrid_search_memories`, `context`→`get_context_for_task`, `forget`→`delete_memory`/`delete_all_memories`, `upload`→`palace_ingest`, `inspect`→`get_memory_history`/`get_entity`; `reflect` has no equivalent. A thin compatibility alias layer would let tools written for Agent Memory work against this server.

## 5. Where this repository is ahead or different

- **Embeddable core.** Agent Memory has no supported in-process mode [S24]; the `surreal-memory` crate is directly embeddable, which is why UAR can vendor it.
- **Local, offline, no metering.** Self-hosted by default, with local candle embeddings; Agent Memory's embeddings are a fixed cloud model and its self-hosted/air-gapped option is Enterprise-only.
- **Graph as a first-class API.** `find_path`, `expand_neighbors`, `get_related`, `get_graph_at_time`, `get_entity_history` are tools; Agent Memory exposes graph traversal as a retrieval signal and in Studio.
- **Task and workflow state.** TaskStreams/Steps, mindmaps, Memory Palace and A2A have no Agent Memory counterpart.
- **Operational durability.** The operation ledger with supervised executor, retry/reject and replay is more developed than anything Agent Memory documents on the write path (its public surface is an `Idempotency-Key` header).
- **Schema ownership.** The library controls its own migrations (v1–v16); Agent Memory's substrate is opaque behind a Context.

## 6. Contradictions and unknowns

1. **Availability stage.** One search summary described Spectron as limited-preview/waitlist; the 2026-10-14 launch and pricing page show a free self-serve Sandbox. *Resolved:* the waitlist information predates the launch (the product was renamed). Docs still contain `SPECTRON_*` remnants.
2. **"Embedded" naming.** The page titled "Embedded library quickstart" states there is no in-process runtime, while linking to "Embedded library" notes for Rust [S24]. *Treated as:* no in-process runtime; the Rust notes were not read.
3. **Plan naming.** Pricing lists *Plus* ($1,100/mo); the startup page says "Memory+ plan". Assumed the same plan.
4. **Benchmark judge.** The launch material names the judge as "Gemini 3.8 Flash" in one place and "Gemini 3.5 Flash Lite … 700 questions" in another [S3, S25]. *Unresolved.* All benchmark numbers are vendor-reported, and the vendor itself notes that headline scores depend on judge and model choices.
5. **Docs-vs-product drift.** Several settings are documented as "stored but not applied" (reconciliation and synthesis model hooks; only one embedding model accepted) [S23]; behaviour may change quickly after launch.
6. **Not examined:** the Lite plan price, the SDK pages beyond Python/TypeScript, the "seven signals" list in full, Studio, the Rust "Embedded library" notes, and any third-party hands-on review. No independent (non-SurrealDB) evaluation of Agent Memory was found.
7. **Our side, not run:** whether BM25 returns results on 4.0 with no index; whether history survives `delete_memory`; whether any REST route enforces authentication beyond what `CLAUDE.md` states.

## 7. Recommendations

Ordered by value and by dependency. This is a gap analysis, not a commitment to match a managed product.

1. **G1 first, independent of the rest.** Add API-key authentication with a context/principal binding, and default container binding to loopback or require a key when bound elsewhere. Keep the scope model as filters *within* an authorized principal's grant.
2. **Verify and fix G6 on 4.0.** Define a full-text index (SurrealDB 4 syntax) and add a test that proves the BM25 leg contributes; this also tells us whether current "hybrid" results are vector-only. Check the open 4.0 KNN issues (#7661, #7662) against our filtered KNN queries.
3. **Structured-write endpoints before LLM extraction (G2).** Accept pre-extracted entity/attribute/relation triples with provenance and let the consumer (UAR) do extraction; add an optional extraction stage later. This unlocks G3–G5 without putting an LLM on the write path.
4. **Schema work, in one migration batch** (the schema-sync rule applies): `valid_from`, `known_at`, a typed `source` object, a supersession link, and a lifecycle `memory_category`. Then implement supersession and uncertainty records.
5. **Lifecycle:** per-category default `valid_until` and importance decay (the TTL worker already enforces expiry), and a purge path with a documented erasure guarantee.
6. **MCP alias layer** for the seven Agent Memory tool names, and a small `reflect`/`profile` pair if extraction lands.
7. **Publish a retrieval-quality result** (BEAM or LoCoMo) with judge, model and date stated, or the comparison to Agent Memory's numbers remains one-sided.
8. **Position, don't chase.** The defensible differentiators are the embeddable library, offline/self-hosted operation, and the graph/task/palace toolset. Parity on the managed-service features is optional.

## Method and limits

- **Pipeline used:** the `deep-research` skill's stages 01 (plan), 02 (search), 03 (retrieve), 05 (credibility) and 09 (report), run manually. Stages 04, 06–08 and 10 (surreal-memory indexing, contradiction resolution, graph, `.research` package) were not run: the surreal-memory MCP server was disconnected during this session, and no packaged output was produced.
- **No adversarial review** of this report was run, so `verification_status` is `partial`.
- **Source quality (0–100, my assessment):** SurrealDB official docs and blog for *feature facts* 85; the same sources for *performance/benchmark claims* 40 (self-reported); employee LinkedIn posts 50 (corroboration only); a C# Corner article on generic agent memory was **discarded** (not about this product); search-engine summaries were used only to locate pages.
- **Single-vendor evidence.** Every statement about Agent Memory rests on SurrealDB's own pages, fetched 2026-11-05. None is independently confirmed.
- **Repository claims** are from static reading of the code on this branch plus `CLAUDE.md`; nothing here was executed against SurrealDB 4.0 for this report.

## Sources

Fetched 2026-11-05 unless noted. `.md` suffixes are SurrealDB's raw-markdown renderings of the same pages.

- S1 SurrealMCP launch (2025-08-23): https://surrealdb.com/blog/introducing-surrealmcp
- S2 Launch post, "Introducing SurrealDB Agent Memory" (2026-10-14): https://surrealdb.com/blog/introducing-surrealdb-agent-memory
- S3 "SurrealDB 4. The database and memory for every agent" (2026-11-03): https://surrealdb.com/blog/surrealdb-4-the-database-and-memory-for-every-agent
- S4 What is SurrealDB Agent Memory: https://surrealdb.com/docs/agent-memory/welcome/what-is-surrealdb-agent-memory.md
- S5 Memory categories: https://surrealdb.com/docs/agent-memory/mental-model/memory-categories.md
- S6 Pricing and comparison: https://surrealdb.com/pricing/spectron
- S7 Contexts and scope: https://surrealdb.com/docs/agent-memory/mental-model/contexts-and-scope.md
- S8 Memory lifecycle: https://surrealdb.com/docs/agent-memory/mental-model/memory-lifecycle.md
- S9 MCP tools: https://surrealdb.com/docs/agent-memory/reference/mcp-tools.md
- S10 REST API: https://surrealdb.com/docs/agent-memory/reference/rest-api.md
- S11 Reconciliation and supersession: https://surrealdb.com/docs/agent-memory/reasoning/reconciliation-and-supersession.md
- S12 Tri-temporal model: https://surrealdb.com/docs/agent-memory/architecture/tri-temporal-model.md
- S13 Surface, security and models: https://surrealdb.com/docs/agent-memory/architecture/surface-security-and-models.md
- S14 Authority hierarchy: https://surrealdb.com/docs/agent-memory/reasoning/authority-hierarchy.md
- S15 Accuracy promise (provenance, traces): https://surrealdb.com/docs/agent-memory/welcome/accuracy-promise.md
- S16 Forget: https://surrealdb.com/docs/agent-memory/operations/forget.md
- S17 `claude-agent-memory` (GitHub): https://github.com/surrealdb/claude-agent-memory
- S18 Mastra integration (2026-09-03): https://surrealdb.com/blog/announcing-the-mastra-integration-for-surrealdb-and-agent-memory
- S19 Hybrid search: https://surrealdb.com/docs/agent-memory/retrieve/hybrid-search.md
- S20 Recall: https://surrealdb.com/docs/agent-memory/retrieve/recall.md
- S21 Reflect: https://surrealdb.com/docs/agent-memory/operations/reflect.md
- S22 Profiles: https://surrealdb.com/docs/agent-memory/operations/profiles.md
- S23 Models per stage: https://surrealdb.com/docs/agent-memory/tuning/models-per-stage.md
- S24 Embedded library quickstart: https://surrealdb.com/docs/agent-memory/quickstarts/embedded.md
- S25 Launch-week page with benchmark figures: https://surrealdb.com/stop-choosing (benchmark post: https://surrealdb.com/blog/agent-memory-benchmarks-are-baked, located but not read in full)
- S26 SurrealDB 4 overview: https://surrealdb.com/4
- Docs index: https://surrealdb.com/docs/agent-memory
