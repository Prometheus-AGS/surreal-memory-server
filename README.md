# Surreal Memory MCP Server

A high-performance Model Context Protocol (MCP) memory server with semantic search capabilities.

## Features

- 🦀 **Pure Rust** - Fast, safe, and efficient
- 🗄️ **SurrealDB** - Embedded or server mode with native vector support
- 🧠 **Multiple Embedding Providers**:
  - **Local** (Candle) - No API keys, runs offline
  - **OpenAI** - High quality embeddings
  - **Cohere** - Multilingual support
- 🔍 **Vector Search** - Semantic similarity search
- 📊 **Knowledge Graph** - Entities, relations, and observations

## Quick Start
```bash
# Clone and build
cargo build --release

# Run with local embeddings (no API key needed)
EMBEDDING_PROVIDER=local ./target/release/rust-memory-mcp

# Run with OpenAI
EMBEDDING_PROVIDER=openai OPENAI_API_KEY=sk-... ./target/release/rust-memory-mcp
```

## Embedding Providers

### Local (Recommended for Privacy)
```bash
EMBEDDING_PROVIDER=local
LOCAL_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5
```

**Supported Models:**
- `BAAI/bge-small-en-v1.5` - 384 dim, fast (recommended)
- `BAAI/bge-base-en-v1.5` - 768 dim, balanced
- `BAAI/bge-large-en-v1.5` - 1024 dim, highest quality
- `sentence-transformers/all-MiniLM-L6-v2` - 384 dim, lightweight

**First run:** Model downloads automatically from Hugging Face (~100MB-1GB depending on model)

**GPU Support:**
```bash
# CUDA
cargo build --release --features cuda

# Metal (macOS)
cargo build --release --features metal
```

### OpenAI
```bash
EMBEDDING_PROVIDER=openai
OPENAI_API_KEY=sk-...
OPENAI_EMBEDDING_MODEL=text-embedding-3-small
```

### Cohere
```bash
EMBEDDING_PROVIDER=cohere
COHERE_API_KEY=...
COHERE_EMBEDDING_MODEL=embed-english-v3.0
```

## Configuration for Claude/Cursor
```json
{
  "mcpServers": {
    "memory": {
      "command": "/path/to/rust-memory-mcp",
      "env": {
        "EMBEDDING_PROVIDER": "local",
        "LOCAL_EMBEDDING_MODEL": "BAAI/bge-small-en-v1.5"
      }
    }
  }
}
```

## Performance

| Provider | Speed | Quality | Cost | Privacy |
|----------|-------|---------|------|---------|
| Local (bge-small) | ⚡⚡⚡ | ⭐⭐⭐ | Free | ✅ |
| Local (bge-base) | ⚡⚡ | ⭐⭐⭐⭐ | Free | ✅ |
| OpenAI | ⚡⚡ | ⭐⭐⭐⭐⭐ | $$ | ❌ |
| Cohere | ⚡⚡ | ⭐⭐⭐⭐ | $ | ❌ |
