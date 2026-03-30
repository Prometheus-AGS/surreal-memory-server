#!/bin/bash
# Load the openAI api key and other secrets
set -a
[ -f .env ] && source .env
set +a

# MCP specific overrides to run safely as a child process via stdio
export API_PORT=0
export MCP_STDIO=true

# SurrealDB connection details referencing the host-exposed port 28000 (from docker-compose)
export SURREAL_MODE="server"
export SURREAL_ENDPOINT="ws://localhost:28000"
export SURREAL_USERNAME="root"
export SURREAL_PASSWORD="root"
export SURREAL_NAMESPACE="memory"
export SURREAL_DATABASE="main"
export EMBEDDING_PROVIDER="openai"

# Stream pure JSON-RPC over stdio
cd /Users/gqadonis/Projects/references/surreal-memory-server
exec cargo run -q --release --bin surreal-memory-server
