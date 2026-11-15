# Spec Delta

## Purpose

Lets callers list, bulk-delete and consume every memory in a scope through
bounded, resumable pages, so no single request can ask the database or a
transport for an unbounded response.

## ADDED Requirements

### Requirement: Memory listing SHALL be paginated with a hard page size

A memory listing request SHALL return at most one page. A page SHALL contain
no more than the requested limit, the default limit applies when none is
given, and a limit of zero or above the documented maximum SHALL be rejected
as an invalid request rather than clamped.

#### Scenario: Default page size applies
- **GIVEN** a scope holding more memories than the default page size
- **WHEN** a caller lists memories without a limit
- **THEN** the page holds exactly the default number of memories
- **AND** the page reports that more memories remain

#### Scenario: Limit above the maximum is rejected
- **WHEN** a caller lists memories with a limit above the documented maximum
- **THEN** the request fails as an invalid request naming the maximum
- **AND** no memories are returned

#### Scenario: Zero limit is rejected
- **WHEN** a caller lists memories with a limit of zero
- **THEN** the request fails as an invalid request

### Requirement: A page SHALL state whether more memories remain

Every page SHALL say whether the listing is complete. When more memories
remain the page SHALL carry a cursor that fetches the next page; when none
remain it SHALL carry no cursor. A response SHALL NOT present a partial
listing as the complete set.

#### Scenario: Last page carries no cursor
- **GIVEN** a scope holding fewer memories than the page size
- **WHEN** a caller lists memories
- **THEN** the page reports that no memories remain
- **AND** the page carries no cursor

#### Scenario: Empty scope returns an empty complete page
- **WHEN** a caller lists memories for a scope with no memories
- **THEN** the page holds no memories and reports that none remain

### Requirement: Following cursors SHALL enumerate every memory exactly once

Pages SHALL be returned in one stable order. A caller that follows each
cursor until none is returned SHALL receive every memory that was in scope
for the whole iteration exactly once, with no memory repeated or skipped.

#### Scenario: Full enumeration matches the stored set
- **GIVEN** a scope holding several pages of synthetic memories
- **WHEN** a caller follows cursors until the listing is complete
- **THEN** the collected IDs equal the stored IDs for that scope
- **AND** no ID appears twice

#### Scenario: Enumeration is larger than one unbounded response
- **GIVEN** a synthetic scope whose total encoded size exceeds the page byte budget many times over
- **WHEN** a caller follows cursors until the listing is complete
- **THEN** every memory is returned and each page stays within the byte budget

### Requirement: Cursors SHALL be resumable and validated

A cursor SHALL be an opaque value that stays usable after an interruption,
with no server-side session. A cursor that is malformed SHALL be rejected as
an invalid request rather than treated as the start of the listing.

#### Scenario: Iteration resumes after an interruption
- **GIVEN** a caller that stopped after the first page of a listing
- **WHEN** the caller later lists again with that page's cursor
- **THEN** the listing continues from the next memory with none repeated

#### Scenario: Malformed cursor is rejected
- **WHEN** a caller lists memories with a cursor that is not a valid cursor
- **THEN** the request fails as an invalid request
- **AND** the first page is not returned in its place

### Requirement: Enumeration SHALL have explicit consistency under concurrent writes

Enumeration SHALL NOT be a snapshot. A memory created during an iteration
SHALL appear at most once and MAY be absent. A memory deleted or expired
during an iteration SHALL NOT appear on pages fetched after the change. A
memory present for the whole iteration SHALL still appear exactly once.

#### Scenario: Concurrent insert neither duplicates nor hides existing memories
- **GIVEN** an iteration that has fetched its first page
- **WHEN** new memories are added to the scope before the next page is fetched
- **THEN** every memory that existed before the iteration is returned exactly once
- **AND** each new memory is returned at most once

#### Scenario: Concurrent delete does not resurface
- **GIVEN** an iteration that has fetched its first page
- **WHEN** a memory not yet returned is deleted
- **THEN** later pages do not contain the deleted memory

### Requirement: Listed memories SHALL omit embeddings unless requested

A page SHALL NOT contain embedding vectors by default, and the vectors SHALL
NOT be read from the database for such a page. A caller MAY request
embeddings explicitly; that request SHALL be subject to a smaller maximum
page size than a listing without embeddings.

#### Scenario: Default page has no embeddings
- **GIVEN** stored memories that have embeddings
- **WHEN** a caller lists memories without requesting embeddings
- **THEN** no memory on the page carries an embedding

#### Scenario: Requested embeddings are returned
- **WHEN** a caller lists memories and requests embeddings within the smaller maximum
- **THEN** each memory on the page carries its stored embedding

#### Scenario: Embedding request above its maximum is rejected
- **WHEN** a caller requests embeddings with a limit above the embedding maximum
- **THEN** the request fails as an invalid request naming that maximum

### Requirement: A page SHALL stay within an encoded-size budget

The encoded size of one page SHALL stay within a documented byte budget. A
page SHALL be shortened to fit and its cursor SHALL point at the first memory
left out. A single memory larger than the budget SHALL still be returned
alone so enumeration always makes progress.

#### Scenario: Large memories shorten the page
- **GIVEN** memories whose content makes a full page exceed the byte budget
- **WHEN** a caller lists memories
- **THEN** the page holds fewer memories than the limit and fits the budget
- **AND** the page reports that more memories remain

#### Scenario: One oversized memory still makes progress
- **GIVEN** a single memory larger than the byte budget
- **WHEN** a caller reaches it during enumeration
- **THEN** it is returned alone on its page and the next cursor moves past it

### Requirement: Enumeration SHALL preserve scope and authorization

Pagination SHALL NOT widen what a caller can read. Owner filters and the
verified caller scope SHALL apply to every page, and a cursor SHALL NOT let a
caller read memories outside that scope. Listing SHALL keep showing only live
memories: archived and expired memories stay hidden.

#### Scenario: Scoped caller sees only its own memories
- **GIVEN** memories belonging to two users
- **WHEN** a caller restricted to the first user enumerates all pages
- **THEN** no memory of the second user is returned

#### Scenario: Cursor from another scope reveals nothing
- **GIVEN** a cursor produced while listing the second user's memories
- **WHEN** a caller restricted to the first user lists with that cursor
- **THEN** only the first user's memories can be returned

#### Scenario: Expired and archived memories stay hidden
- **WHEN** a caller enumerates a scope containing expired and archived memories
- **THEN** none of them appears on any page

### Requirement: MCP and REST SHALL return the same page shape

The MCP `get_all_memories` tool and the REST memory list endpoint SHALL each
return one page as an object holding the memories, the next cursor and
whether more remain. Both SHALL accept the limit, the cursor and the
embedding option, including when a client sends them as strings.

#### Scenario: REST list returns a page object
- **WHEN** a client requests the REST memory list
- **THEN** the body is an object with the memories, the next cursor and a more-remaining flag
- **AND** the body is not a bare array

#### Scenario: MCP list returns a page object
- **WHEN** a client calls the MCP `get_all_memories` tool
- **THEN** the result is an object with the memories, the next cursor and a more-remaining flag

#### Scenario: String-encoded parameters are accepted
- **WHEN** an MCP client sends the limit as `"25"` and the embedding option as `"false"`
- **THEN** the call succeeds with a page of at most 25 memories

#### Scenario: Invalid paging parameters are a client error
- **WHEN** a REST client sends a limit above the maximum or a malformed cursor
- **THEN** the response is a client error, not a server error

### Requirement: The library's complete listing SHALL stay available and bounded per query

The existing library call that returns every memory in a scope SHALL keep its
signature and SHALL keep returning the complete set, so existing Rust callers
are unaffected. It SHALL obtain that set through bounded pages, so no single
database response is unbounded, and SHALL fail rather than return a partial
set.

#### Scenario: Complete listing returns everything
- **GIVEN** a scope holding several pages of memories
- **WHEN** a Rust caller requests the complete listing
- **THEN** every memory in the scope is returned once

#### Scenario: Failure is not a partial result
- **WHEN** a page fails while the complete listing is being assembled
- **THEN** the call returns an error and no partial list

### Requirement: Storage backends without pagination SHALL fail closed

A storage backend that has not implemented paginated listing SHALL return an
error for it. It SHALL NOT fall back to an unbounded or unscoped listing.

#### Scenario: Backend without pagination returns an error
- **GIVEN** a storage backend that has not implemented paginated listing
- **WHEN** a caller requests a page
- **THEN** the call fails with an error saying pagination is not supported

### Requirement: Bulk deletion SHALL not materialize memory content

Deleting every memory in a scope SHALL read record identifiers only, in
bounded batches, and never memory content or embeddings. It SHALL keep its
existing semantics: it removes archived and expired memories too, touches
only the given scope, and returns the number deleted.

#### Scenario: Scope larger than one batch is fully deleted
- **GIVEN** a scope holding more memories than one deletion batch
- **WHEN** a caller deletes all memories in that scope
- **THEN** the returned count equals the number stored
- **AND** the scope is empty afterwards

#### Scenario: Other scopes are untouched
- **GIVEN** memories belonging to two users
- **WHEN** all memories of the first user are deleted
- **THEN** the second user's memories all remain

#### Scenario: Expired memories are deleted too
- **GIVEN** a scope containing expired memories
- **WHEN** a caller deletes all memories in that scope
- **THEN** the expired memories are removed and counted

### Requirement: Interrupted bulk deletion SHALL report resumable progress

When bulk deletion fails partway, the error SHALL state how many memories
were deleted before the failure. Running the same deletion again SHALL remove
the remainder.

#### Scenario: Failure reports progress and a rerun finishes
- **GIVEN** a bulk deletion that fails after deleting some memories
- **WHEN** the caller reads the error and runs the same deletion again
- **THEN** the error states the number already deleted
- **AND** the rerun deletes the remaining memories

### Requirement: Persona mindmap generation SHALL consume pages

Generating a persona mindmap SHALL read the user's memories through bounded
pages without embeddings and SHALL hold only the per-category state it
renders, not the full collection. The resulting mindmap SHALL be the same as
before for the same memories, and its description SHALL state the true total.

#### Scenario: Persona over a multi-page collection
- **GIVEN** a user with several pages of memories across categories
- **WHEN** a persona mindmap is generated through MCP or REST
- **THEN** the mindmap has one branch per category with its leaves
- **AND** its description states the total number of memories read

### Requirement: Listing failures SHALL be diagnosable without exposing memory data

A failed listing or bulk deletion SHALL report and log the operation, the
page size, whether embeddings were requested and which scope filters were
present. It SHALL NOT log or return memory content, embedding values, scope
identifier values or credentials.

#### Scenario: Failure names the operation and its bounds
- **WHEN** a paginated listing fails at the database
- **THEN** the error and the log entry name the operation, the page size and the embedding option
- **AND** they state which scope filters were set without their values

#### Scenario: Failure carries no private data
- **WHEN** a paginated listing fails at the database
- **THEN** the error and the log entry contain no memory content and no credentials
