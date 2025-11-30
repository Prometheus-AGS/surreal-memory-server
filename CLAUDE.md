# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Project Overview

This is a high-performance Model Context Protocol (MCP) memory server built in Rust, providing semantic search capabilities with multiple embedding providers. The server uses SurrealDB for storage and supports both local and cloud-based embedding generation.

## Development Commands

### Building
```bash
# Standard build
cargo build --release

# Build with specific features
cargo build --release --features cuda          # CUDA GPU support
cargo build --release --features metal         # Metal GPU support (macOS)
cargo build --release --features server-only   # Server-only mode (no embedded DB)

# Use build script (recommended for Apple Silicon)
./build.sh
```

### Quality Gate & Testing
```bash
# Run the complete quality gate (required before commits)
./scripts/quality-check.sh

# Individual quality checks
cargo fmt --all --check                        # Code formatting
cargo clippy --all-targets --features embedded,metal -- -D warnings  # Linting
cargo test --all-targets --features embedded,metal     # Tests

# Set custom features for quality checks
FEATURE_FLAGS=embedded,cuda ./scripts/quality-check.sh
```

### Running
```bash
# With local embeddings (default)
EMBEDDING_PROVIDER=local ./target/release/surreal-memory-server

# With OpenAI
EMBEDDING_PROVIDER=openai OPENAI_API_KEY=sk-... ./target/release/surreal-memory-server

# Pre-download embedding models
./download-model.sh
```

## Architecture

### Core Modules

**Embeddings Module** (`src/embeddings/`):
- `candle.rs` - Local embeddings using Candle ML framework
- `cohere.rs` - Cohere API embedding provider
- `openai.rs` - OpenAI API embedding provider
- Supports flexible provider switching via environment configuration

**MCP Module** (`src/mcp/`):
- `handlers.rs` - Model Context Protocol request/response handling
- Implements MCP server functionality for Claude/AI client integration

**Storage Module** (`src/storage/`):
- `surreal.rs` - SurrealDB storage implementation with vector support
- `models.rs` - Data models for entities, relations, and observations
- Supports both embedded (RocksDB) and server modes

### Configuration System

The server uses environment variables for configuration:

**Database Configuration**:
- `SURREAL_MODE=embedded` (default) or `server`
- `SURREAL_PATH=./data/memory.db` (embedded mode)
- `SURREAL_ENDPOINT=ws://localhost:8000` (server mode)

**Embedding Configuration**:
- `EMBEDDING_PROVIDER=local|openai|cohere`
- `LOCAL_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5` (for local provider)
- `MODEL_CACHE_DIR=./models` (local model storage)

### Feature Flags

- `embedded` (default) - Includes RocksDB support for embedded SurrealDB
- `server-only` - Server-only mode without embedded database
- `cuda` - CUDA GPU acceleration for local embeddings
- `metal` - Metal GPU acceleration for Apple devices

## Development Guidelines

### Code Quality Standards
- All code must pass the quality gate (`./scripts/quality-check.sh`) before commits
- Warnings are treated as errors (`RUSTFLAGS=-Dwarnings`)
- Follow coding standards in `docs/coding-standards/README.md`
- Use structured logging with tracing crate

### Build System
- `build.sh` runs quality checks before building release binaries
- Default features are `embedded,metal` for development
- Override with `FEATURE_FLAGS` environment variable

### Model Management
- Local embedding models are cached in `./models/` directory
- Use `download-model.sh` to pre-download models from Hugging Face
- Supported models include BGE and sentence-transformers variants

### Testing Strategy
- Unit tests for individual modules
- Integration tests for MCP protocol handling
- Mock implementations available for I/O and embedding providers
- Quality gate ensures warning-free builds and passing tests

## Common Patterns

### Adding New Embedding Provider
1. Create provider module in `src/embeddings/`
2. Implement embedding trait
3. Add configuration parsing in main module
4. Update provider selection logic
5. Add integration tests

### Extending Storage Models
1. Update models in `src/storage/models.rs`
2. Add migration if needed
3. Update SurrealDB queries in `src/storage/surreal.rs`
4. Test with both embedded and server modes

### MCP Protocol Extensions
1. Add handlers in `src/mcp/handlers.rs`
2. Follow MCP specification patterns
3. Ensure proper error handling and logging
4. Add integration tests for new capabilities

## Performance Considerations

- Local embeddings provide privacy but require computational resources
- GPU acceleration (CUDA/Metal) significantly improves embedding performance
- SurrealDB embedded mode is faster for single-instance deployments
- Vector search performance depends on embedding dimension and dataset size
- Consider model size vs. quality trade-offs for local embeddings

## Dependencies & Tools

**Core Dependencies**:
- `rmcp` - MCP protocol implementation
- `surrealdb` - Database with native vector support
- `candle-*` - Local ML inference
- `tokio` - Async runtime

**Development Tools**:
- `huggingface-cli` - Model downloading (optional)
- Standard Rust toolchain with clippy, rustfmt
- Quality gate enforces warnings-as-errors policy