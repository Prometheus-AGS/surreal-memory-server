# Project connections and handoffs

| Peer | Observed connection | Memory team ↔ peer roles |
| --- | --- | --- |
| universal-agent-runtime | Cargo.toml points to `vendor/git/surreal-memory-server/crates/surreal-memory`; UAR memory facade/context consumers depend on the Rust library | storage/runtime ↔ uar-state, uar-runtime; transport trust ↔ uar-trust-tools; verifier ↔ uar-verifier |
| the-boss | `src/main/services/prometheus/workspaceMcp.ts` configures the external `surreal-memory` MCP service; Boss retains its separate application store | product/transports ↔ boss-product, boss-runtime, boss-security; operator/API usability ↔ boss-ux; verifier ↔ boss-verifier |
| prometheus-skill-system | README identifies it as canonical home for the memory-operation API description | product/runtime provide draft contract changes and evidence to its assigned maintainers; no invented team IDs |
| prometheus-skills-mini / prometheus-skill-pack | Skills/tooling clients and team tooling can use memory; creator's current optional adapter uses v1 memory REST | product/transports ↔ existing skill owners; v1 similarity deduplication is not v2 operation receipt idempotency |
| librefang consumer | Existing `resync-librefang-consumer` OpenSpec change records a pending library consumer sync | product/storage preserve that change; verify current checkout/pin before issuing a handoff, no completion claim |
| mempalace-rs | Optional palace library dependency supplies a storage trait implemented by `palace/adapter.rs` | retrieval/storage own the adapter; upstream implementation is outside this graph |
| Compass | Development analysis tool only | lead owns graph scope; it is not the memory service database |

This is a contract map, not a unified cross-repository call graph. Neighbor repositories were read as evidence only and are not indexed here. Boss's UAR integration roadmap must not be mistaken for an observed direct memory-library edge. Read each peer's current agent manifest before addressing a role; role names do not provide a live communication transport.

Each handoff includes: originating task, intended outcome, source Git revision, graph generation/freshness, exact API/trait/schema changes, compatibility/feature matrix, acceptance scenarios, actual evidence, unresolved risks, assigned receiving role and authorization scope. Include v2 operation ID/hash and last SSE sequence for receipt investigations; redact personal memory and credentials. Use local agent-team-handoff artifacts for switching owner/harness. Product drafts stakeholder updates; sending them requires an explicit user instruction.

Older root notes describe a direct UAR git dependency and an unresolved RwLock defect. Current source shows vendoring and ArcSwap. Preserve historical notes but inspect source before acting. Do not update a neighboring Cargo pin/vendor tree just because a note says to do so; prepare the exact proposed consumer change for its own authorized task.
