# TaskStreams REST API Documentation

TaskStreams provide named task contexts with model-aware token budgeting and rolling auto-summarization. Now fully accessible via REST API!

## Base URL
```
http://localhost:3001/api/v1/taskstreams
```

## Endpoints

### 1. Create Task Stream
Create a new task stream with optional model configuration and auto-summarization.

**Request:**
```http
POST /api/v1/taskstreams
Content-Type: application/json

{
  "name": "feature-development",
  "description": "Main feature development stream",
  "user_id": "user-123",
  "agent_id": "agent-456",
  "model_id": "gpt-4o",
  "auto_summarize": true
}
```

**Response:** `201 Created`
```json
{
  "name": "feature-development",
  "description": "Main feature development stream",
  "user_id": "user-123",
  "agent_id": "agent-456",
  "status": "active",
  "total_tokens": 0,
  "auto_summarize": true,
  "summary_count": 0,
  "model_id": "gpt-4o",
  "created_at": "2026-03-28T06:00:00Z",
  "last_active": "2026-03-28T06:00:00Z"
}
```

### 2. List Task Streams
Get all task streams, optionally filtered by user or agent.

**Request:**
```http
GET /api/v1/taskstreams?user_id=user-123
```

**Response:** `200 OK`
```json
[
  {
    "name": "feature-development",
    "description": "Main feature development stream",
    "user_id": "user-123",
    "status": "active",
    "total_tokens": 1250,
    "auto_summarize": true,
    "summary_count": 0,
    "model_id": "gpt-4o",
    "created_at": "2026-03-28T06:00:00Z",
    "last_active": "2026-03-28T06:15:00Z"
  }
]
```

### 3. Get Task Stream
Retrieve a specific task stream by name.

**Request:**
```http
GET /api/v1/taskstreams/feature-development
```

**Response:** `200 OK`
```json
{
  "name": "feature-development",
  "description": "Main feature development stream",
  "user_id": "user-123",
  "status": "active",
  "total_tokens": 1250,
  "auto_summarize": true,
  "summary_count": 0,
  "model_id": "gpt-4o",
  "created_at": "2026-03-28T06:00:00Z",
  "last_active": "2026-03-28T06:15:00Z"
}
```

### 4. Get Task Context
Get memory context for a task stream with token budget information.

**Request:**
```http
GET /api/v1/taskstreams/feature-development/context?model_id=gpt-4o&max_tokens=100000
```

**Response:** `200 OK`
```json
{
  "name": "feature-development",
  "total_tokens": 1250,
  "budget_limit": 112000,
  "needs_summarization": false,
  "memory_count": 15,
  "model_id": "gpt-4o"
}
```

**Query Parameters:**
- `model_id` (optional): Model to use for token budget calculation (default: "gpt-4o")
- `max_tokens` (optional): Override token budget limit

### 5. Archive Task Stream
Mark a task stream as archived (completed/inactive).

**Request:**
```http
POST /api/v1/taskstreams/feature-development/archive
```

**Response:** `200 OK`
```json
{
  "name": "feature-development",
  "status": "archived",
  ...
}
```

### 6. Auto-Summarize Task Stream
Trigger automatic summarization of task stream memories.

**Request:**
```http
POST /api/v1/taskstreams/feature-development/summarize
```

**Response:** `200 OK`
```json
{
  "name": "feature-development",
  "summary_count": 1,
  "total_tokens": 800,
  ...
}
```

### 7. Delete Task Stream
Delete a task stream and its associated memories.

**Request:**
```http
DELETE /api/v1/taskstreams/feature-development
```

**Response:** `204 No Content`

---

## Adding Memories to Task Streams

Memories can be associated with task streams using the existing memory API:

**Request:**
```http
POST /api/v1/memory
Content-Type: application/json

{
  "content": "Implemented user authentication with JWT",
  "user_id": "user-123",
  "task_stream_id": "feature-development"
}
```

---

## Model Profiles

Built-in token budgets for common models:

| Model | Context Window | Budget (80%) | Summarization Threshold (80% of budget) |
|-------|----------------|--------------|----------------------------------------|
| gpt-4o | 128K | 112K | 89.6K |
| gpt-4o-mini | 128K | 112K | 89.6K |
| claude-3-5-sonnet | 200K | 176K | 140.8K |
| claude-3-opus | 200K | 176K | 140.8K |
| gemini-1.5-pro | 2M | 1.76M | 1.4M |
| gemini-2.0-flash | 1M | 880K | 704K |
| llama-3.3-70b | 128K | 112K | 89.6K |
| mistral-large | 128K | 112K | 89.6K |

---

## Auto-Summarization

When `auto_summarize: true` (default), the system automatically:

1. Tracks total token count across all memories in the stream
2. Compares against model's budget limit (80% of context window)
3. Triggers summarization when threshold reached (80% of budget)
4. Condenses older memories into summary format
5. Maintains recent memories for context continuity

**Summarization Trigger:**
```
needs_summarization = total_tokens >= summarization_threshold
```

For GPT-4o: `total_tokens >= 89,600` triggers auto-summarization

---

## Status Values

Task streams can have the following statuses:

- `active` - Currently in use, accepting new memories
- `archived` - Completed or inactive, no longer accepting updates

---

## Integration with Memory API

Task streams integrate seamlessly with the memory API:

```bash
# Create a task stream
curl -X POST http://localhost:3001/api/v1/taskstreams \
  -H "Content-Type: application/json" \
  -d '{"name":"my-task","user_id":"user-1","model_id":"gpt-4o"}'

# Add memories to the task stream
curl -X POST http://localhost:3001/api/v1/memory \
  -H "Content-Type: application/json" \
  -d '{"content":"Step 1 complete","user_id":"user-1","task_stream_id":"my-task"}'

# Check token budget status
curl http://localhost:3001/api/v1/taskstreams/my-task/context?model_id=gpt-4o

# Archive when done
curl -X POST http://localhost:3001/api/v1/taskstreams/my-task/archive
```

---

## Error Responses

### 404 Not Found
```json
{
  "error": "Task stream not found"
}
```

### 500 Internal Server Error
```json
{
  "error": "Failed to create task stream: <detailed error message>"
}
```

---

## Best Practices

1. **Unique Names:** Task stream names must be unique across the system
2. **Model Selection:** Choose model_id based on your token budget needs
3. **Auto-Summarization:** Leave enabled for long-running tasks
4. **User/Agent Scoping:** Use user_id or agent_id to organize streams
5. **Archiving:** Archive completed streams to maintain clean lists
6. **Context Checking:** Monitor token budget to avoid surprises

---

## Example Workflow

```bash
# 1. Create stream for a feature
curl -X POST http://localhost:3001/api/v1/taskstreams \
  -H "Content-Type: application/json" \
  -d '{"name":"auth-feature","description":"User authentication","user_id":"dev-1","model_id":"gpt-4o"}'

# 2. Add development steps as memories
for i in {1..10}; do
  curl -X POST http://localhost:3001/api/v1/memory \
    -H "Content-Type: application/json" \
    -d "{\"content\":\"Step $i completed\",\"user_id\":\"dev-1\",\"task_stream_id\":\"auth-feature\"}"
done

# 3. Check token budget
curl http://localhost:3001/api/v1/taskstreams/auth-feature/context?model_id=gpt-4o

# 4. Archive when complete
curl -X POST http://localhost:3001/api/v1/taskstreams/auth-feature/archive

# 5. List all streams
curl http://localhost:3001/api/v1/taskstreams?user_id=dev-1
```
