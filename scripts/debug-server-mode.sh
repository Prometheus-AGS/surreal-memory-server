#!/bin/bash
# Debug test script for surreal-memory-server in SERVER MODE
# Uses SurrealDB running in Docker via docker-compose
#
# Prerequisites:
#   1. Start SurrealDB: docker compose up -d
#   2. Verify it's running: docker compose ps
#
# Usage:
#   ./scripts/debug-server-mode.sh
#
# This script runs the server with cargo run connected to the
# SurrealDB instance in docker-compose.yaml
#
# Note: Requests are sent sequentially with delays to allow
# embedding generation to complete before dependent operations.

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Change to project root
cd "$(dirname "$0")/.."

echo -e "${BLUE}=== Surreal Memory Server Debug (Server Mode) ===${NC}"
echo ""

# Check if Docker is running
if ! docker info > /dev/null 2>&1; then
    echo -e "${RED}Error: Docker is not running!${NC}"
    echo "Please start Docker and try again."
    exit 1
fi

# Check if SurrealDB container is running
if ! docker compose ps --format json 2>/dev/null | grep -q '"running"'; then
    echo -e "${YELLOW}SurrealDB container is not running. Starting it...${NC}"
    docker compose up -d
    echo -e "${YELLOW}Waiting for SurrealDB to start...${NC}"
    sleep 3
    echo -e "${GREEN}SurrealDB started.${NC}"
else
    echo -e "${GREEN}SurrealDB container is already running.${NC}"
fi

echo ""

# Set environment variables for SERVER mode
export RUST_LOG=debug
export SURREAL_MODE=server
export SURREAL_ENDPOINT=ws://localhost:8000
export SURREAL_USERNAME=root
export SURREAL_PASSWORD=root
export SURREAL_NAMESPACE=memory
export SURREAL_DATABASE=mcp
export EMBEDDING_PROVIDER=local
export LOCAL_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5
export MODEL_CACHE_DIR=./models

# MCP JSON-RPC Messages
# =====================

# 1. Initialize MCP connection (required first)
INIT_REQUEST='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"debug-server-mode","version":"1.0.0"}}}'

# 2. Initialized notification (REQUIRED - no id field, this is a notification not a request)
# This MUST be sent after receiving the initialize response and before any other requests
INITIALIZED_NOTIFICATION='{"jsonrpc":"2.0","method":"notifications/initialized"}'

# 3. List available tools
LIST_TOOLS='{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'

# 4. Create a person entity
CREATE_PERSON='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_entity","arguments":{"name":"Alice Smith","entity_type":"Person","observations":["Senior software engineer","Works on distributed systems","Expert in Rust and Go","Based in Seattle"]}}}'

# 5. Create a company entity
CREATE_COMPANY='{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_entity","arguments":{"name":"CloudTech Inc","entity_type":"Company","observations":["Cloud infrastructure company","Founded in 2018","Provides serverless compute solutions","Headquarters in Seattle"]}}}'

# 6. Create a project entity
CREATE_PROJECT='{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"create_entity","arguments":{"name":"Nimbus","entity_type":"Project","observations":["Serverless function runtime","Written in Rust","Supports WebAssembly modules","Low latency cold starts"]}}}'

# 7. Create relation: Alice -> works_at -> CloudTech
CREATE_RELATION_1='{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"create_relation","arguments":{"from":"Alice Smith","to":"CloudTech Inc","relation_type":"works_at"}}}'

# 8. Create relation: Alice -> leads -> Nimbus
CREATE_RELATION_2='{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"create_relation","arguments":{"from":"Alice Smith","to":"Nimbus","relation_type":"leads"}}}'

# 9. Create relation: CloudTech -> develops -> Nimbus
CREATE_RELATION_3='{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"create_relation","arguments":{"from":"CloudTech Inc","to":"Nimbus","relation_type":"develops"}}}'

# 10. Add more observations to Alice
ADD_OBSERVATIONS='{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"add_observations","arguments":{"entity_name":"Alice Smith","observations":["Presented at KubeCon 2024","Published paper on serverless cold starts","Mentors junior engineers"]}}}'

# 11. Search for entities by text
SEARCH_TEXT='{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"search_entities","arguments":{"query":"Rust programming"}}}'

# 12. Semantic search using embeddings
SEMANTIC_SEARCH='{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"semantic_search","arguments":{"query":"cloud computing and serverless technology","limit":5}}}'

# 13. Read the full knowledge graph
READ_GRAPH='{"jsonrpc":"2.0","id":12,"method":"tools/call","params":{"name":"read_graph","arguments":{}}}'

# Function to send a request and wait
# We use a FIFO to communicate with the server process
send_request() {
    local request="$1"
    local delay="${2:-0.5}"
    echo "$request"
    sleep "$delay"
}

# Build the request sequence with proper delays
# Entity creation requires embedding generation (~1-2 seconds each with Metal GPU)
# Relations and other operations are faster
build_requests() {
    # Initialize MCP connection
    send_request "$INIT_REQUEST" 0.1
    send_request "$INITIALIZED_NOTIFICATION" 0.1

    # List tools (fast operation)
    send_request "$LIST_TOOLS" 0.3

    # Create entities - these take time due to embedding generation
    # Send them with longer delays to allow completion
    send_request "$CREATE_PERSON" 3
    send_request "$CREATE_COMPANY" 3
    send_request "$CREATE_PROJECT" 3

    # Create relations - entities should exist now
    send_request "$CREATE_RELATION_1" 0.5
    send_request "$CREATE_RELATION_2" 0.5
    send_request "$CREATE_RELATION_3" 0.5

    # Add observations - requires entity to exist and triggers re-embedding
    send_request "$ADD_OBSERVATIONS" 3

    # Search operations
    send_request "$SEARCH_TEXT" 0.5
    send_request "$SEMANTIC_SEARCH" 2

    # Read full graph
    send_request "$READ_GRAPH" 0.5

    # Keep connection open briefly for final responses
    sleep 2
}

echo -e "${BLUE}Test Sequence:${NC}"
echo "  1. Initialize MCP connection"
echo "  2. Send initialized notification (required by MCP protocol)"
echo "  3. List available tools"
echo "  4. Create entity: Alice Smith (Person)"
echo "  5. Create entity: CloudTech Inc (Company)"
echo "  6. Create entity: Nimbus (Project)"
echo "  7. Create relation: Alice -> works_at -> CloudTech"
echo "  8. Create relation: Alice -> leads -> Nimbus"
echo "  9. Create relation: CloudTech -> develops -> Nimbus"
echo "  10. Add observations to Alice Smith"
echo "  11. Text search for 'Rust programming'"
echo "  12. Semantic search for 'cloud computing and serverless'"
echo "  13. Read full knowledge graph"
echo ""

echo -e "${YELLOW}Environment (SERVER MODE):${NC}"
echo "  SURREAL_MODE=$SURREAL_MODE"
echo "  SURREAL_ENDPOINT=$SURREAL_ENDPOINT"
echo "  SURREAL_USERNAME=$SURREAL_USERNAME"
echo "  SURREAL_NAMESPACE=$SURREAL_NAMESPACE"
echo "  SURREAL_DATABASE=$SURREAL_DATABASE"
echo "  EMBEDDING_PROVIDER=$EMBEDDING_PROVIDER"
echo "  LOCAL_EMBEDDING_MODEL=$LOCAL_EMBEDDING_MODEL"
echo "  RUST_LOG=$RUST_LOG"
echo ""

echo -e "${GREEN}Starting server with cargo run (server-only + metal for local embeddings)...${NC}"
echo -e "${YELLOW}(First run may download embedding models ~50MB)${NC}"
echo -e "${YELLOW}(Requests are sent sequentially with delays for embedding generation)${NC}"
echo ""
echo -e "${BLUE}========================================${NC}"
echo ""

# Run with cargo run for debugging support
# Uses server-only feature (no embedded RocksDB) + metal for GPU-accelerated local embeddings
#
# Requests are sent sequentially with delays to allow embedding generation
# to complete before dependent operations (like creating relations that
# reference entities, or adding observations to entities).
build_requests | cargo run --features "server-only,metal"

echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}=== Debug Test Complete ===${NC}"
echo ""
echo -e "${YELLOW}Useful commands:${NC}"
echo "  View SurrealDB logs:  docker compose logs -f surrealdb"
echo "  Stop SurrealDB:       docker compose down"
echo "  Reset SurrealDB data: docker compose down -v"
