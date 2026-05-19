# write-path Spec Delta — Add MCP Progress Reporting

## ADDED Requirements

### Requirement: Slow tool calls SHALL emit MCP progress notifications

A tool handler whose operation may take longer than a few seconds SHALL emit
`notifications/progress` heartbeats while work is in flight, so a client that
supports `resetTimeoutOnProgress` does not time out on a working server.

#### Scenario: A slow tool reports progress
- **GIVEN** a client supplies a `progressToken` for a slow tool call
- **WHEN** the operation runs longer than the heartbeat interval
- **THEN** the server emits at least one `notifications/progress` before the
  final result

#### Scenario: Missing progress token degrades gracefully
- **GIVEN** a client supplies no `progressToken`
- **WHEN** a slow tool is invoked
- **THEN** the operation still completes correctly without emitting progress

### Requirement: Mindmap updates SHALL enforce a bounded query timeout

The mindmap `UPDATE` path SHALL enforce a query `TIMEOUT` so that updates to
large mindmaps fail fast with a clear error rather than hanging. The implemented
behavior SHALL match the documentation.

#### Scenario: Large mindmap update fails fast
- **GIVEN** a mindmap large enough that an update exceeds the timeout
- **WHEN** a node or edge is added
- **THEN** the operation returns a clear error within the timeout bound

#### Scenario: Documentation matches implementation
- **WHEN** the mindmap timeout behavior is reviewed against CLAUDE.md
- **THEN** the documented timeout matches the enforced timeout in code
