# Model policy

Hard reasoning is assigned to lead, product, runtime, storage, retrieval and verifier; bounded transport implementation defaults to medium and should escalate for difficult protocol/security work. Tiers are task classifications, not measured quality scores.

| Harness | Native binding | Evidence and limits |
| --- | --- | --- |
| Codex | hard `gpt-6-astra`/high; medium `gpt-5.6-terra`/high | Installed host catalog advertises these IDs/efforts; no inference smoke test |
| Claude Code | hard `opus`/high; medium `sonnet`/high | Documented native aliases; account entitlement unverified |
| Kimi Code | launcher defaults to `kimi-code/k3` | Role frontmatter model selection unsupported; native secondary pool/settings apply |
| OpenCode | inherit active provider/model | Do not fabricate a provider/model ID; inspect native configuration before invocation |
| MiniMax CLI | inherit isolated profile's provider/model | mcode is available; gateway alias MiniMax-M3 is not automatically a native ID; credentials/inference unverified |

Use agent-team-models before task invocation to discover actual availability and apply any capability/budget requirement. No prices or unsupported capabilities are inferred. Common manifest policy stays tier-only so Codex IDs do not leak into other provider namespaces; concrete supported IDs are native overrides. An unavailable requested model needs an explicit native selection, not silent substitution. Optional gateway discovery and review receipts state their own freshness and limits.

Prefer a fresh different-family reviewer when dispatchable. A same-family reviewer is independent in context but a weaker cross-model check. Definitions and CLI discovery do not establish successful role execution or model quality.

During setup the configured gateway listed MiniMax-M3, gpt-5.4, gpt-5.5, k3, k3-256k and kimi-for-coding. agent-team-models selected MiniMax-M3 for review with unknown pricing; its single 60-second inference attempt timed out. See research/model-discovery.json, research/review-model-selection.json and reviews/gateway-attempt.json. The completed native review is fresh-context same-family fallback, not a successful cross-model review.
