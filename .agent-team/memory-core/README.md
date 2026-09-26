# memory-core

Seven project roles support the embeddable Rust library and the durable REST/MCP service. These are installed definitions, not running agents or a scheduler. Start with one implementer and one independent reviewer; add specialists only for distinct work. Use at most four concurrent agents including the lead.

| Role | Responsibility |
| --- | --- |
| memory-lead | Architecture, task ownership, integration and coordination |
| memory-product | Product requirements, API/operator usability, consumer handoffs |
| memory-runtime | Durable ledger, receipt/replay, SSE, executor and service lifecycle |
| memory-storage | Library API, persisted models, SurrealDB storage and migrations |
| memory-retrieval | Embeddings, tokenization, relevance, model readiness and palace |
| memory-transports | REST/MCP/A2A, coercion, transport trust boundaries and contracts |
| memory-verifier | Fresh-context independent review and completed-boundary evidence |

Read [routing](routing.md), [tools](tool-policy.md), [handoffs](handoffs.md), [model policy](model-policy.md) and [review protocol](review-protocol.md). Canonical roles are in [team.json](team.json); [state.json](state.json) is the creator coordination ledger. Existing KBD/OpenSpec state remains authoritative. Use the local creator CLI for updates; do not manually edit state projections.

From this project root:

```text
node .agent-team/memory-core/launch.mjs codex
node .agent-team/memory-core/launch.mjs claude
node .agent-team/memory-core/launch.mjs kimi
node .agent-team/memory-core/launch.mjs opencode
node .agent-team/memory-core/launch.mjs minimax
```

Ask the native harness to use `memory-lead` and the relevant specialist. Native selection/spawning capabilities differ; definitions do not start Claude experimental teams or force seven agents to run. MiniMax has no verified `--agent` selector: use its native custom-agent UI/tool selection after launch, or provide the role definition as context. Kimi defaults to `kimi-code/k3`; an explicit native model argument overrides it. OpenCode inherits its configured provider/model. MiniMax uses this project's `.minimax` data directory; configure that isolated profile through native login/settings. No global credentials are copied. The launcher uses the Node installation owning mcode when available to avoid native SQLite ABI mismatch.

Native definitions:

- Codex: `.codex/agents/memory-*.toml`
- Claude Code: `.claude/agents/memory-*.md`
- Kimi Code: `.kimi-code/agents/memory-*.md`
- OpenCode: `.opencode/agents/memory-*.md`
- MiniMax CLI: `.minimax/agents/memory-*/agent.md`, under the launcher's `MINIMAX_DATA_DIR`

Project skills live in `.agents/skills`. Every role has explicit skill file paths; read those directly if a harness does not discover this directory. Do not assume frontmatter skill IDs install or execute skills. MCP configuration is provided in `.mcp.json` and `.codex/config.toml` as `compass-surreal-memory-server`. Other harnesses can use the exact stdio definition in [tool policy](tool-policy.md) or the local Compass CLI; no global registration is required.

Refresh the graph after relevant source changes:

```text
compass update . --force --store sqlite --inference-level low --no-cluster --no-viz --timing
node .compass/verify.mjs
```

The graph is generated and ignored; it must be rebuilt in a fresh checkout. Recorded absolute MCP paths are for this workspace and must be updated when relocated. Consult `.compass/scope.json` and `.compass/verification.json` for extraction scope, counts and limitations. Current graph includes inline tests and inactive feature branches; it is not a runtime trace.

See [verification.json](verification.json) for the exact checks performed and native support limits. No application/DB execution or successful model inference is implied by this installation.
