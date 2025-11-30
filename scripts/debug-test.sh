#!/bin/bash
# Debug test script for surreal-memory-server
# Uses cargo run for easy debugging with Zed's debugger
#
# Usage:
#   ./scripts/debug-test.sh
#
# This script runs the server with cargo run and pipes MCP JSON-RPC
# requests to stdin. Useful for debugging with breakpoints in Zed.

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Change to project root
cd "$(dirname "$0")/.."

echo -e "${BLUE}=== Surreal Memory Server Debug Test ===${NC}"
echo ""

# Set environment variables for the server
export RUST_LOG=debug
export EMBEDDING_PROVIDER=local
export LOCAL_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5
export MODEL_CACHE_DIR=./models
export SURREAL_MODE=embedded
export SURREAL_PATH=./data/debug-test.db

# Clean up old test data for fresh start
echo -e "${YELLOW}Cleaning up old test database...${NC}"
rm -rf ./data/debug-test.db 2>/dev/null || true

# MCP JSON-RPC Messages
# =====================

# 1. Initialize MCP connection (required first)
INIT_REQUEST='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"debug-test","version":"1.0.0"}}}'

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

# Combine all requests (newline-separated JSON-RPC)
# Note: initialized notification MUST come right after initialize request
ALL_REQUESTS=$(cat <<EOF
$INIT_REQUEST
$INITIALIZED_NOTIFICATION
$LIST_TOOLS
$CREATE_PERSON
$CREATE_COMPANY
$CREATE_PROJECT
$CREATE_RELATION_1
$CREATE_RELATION_2
$CREATE_RELATION_3
$ADD_OBSERVATIONS
$SEARCH_TEXT
$SEMANTIC_SEARCH
$READ_GRAPH
EOF
)

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

echo -e "${YELLOW}Environment:${NC}"
echo "  EMBEDDING_PROVIDER=$EMBEDDING_PROVIDER"
echo "  LOCAL_EMBEDDING_MODEL=$LOCAL_EMBEDDING_MODEL"
echo "  SURREAL_MODE=$SURREAL_MODE"
echo "  SURREAL_PATH=$SURREAL_PATH"
echo "  RUST_LOG=$RUST_LOG"
echo ""

echo -e "${GREEN}Starting server with cargo run...${NC}"
echo -e "${YELLOW}(First run may download embedding models ~50MB)${NC}"
echo ""
echo -e "${BLUE}========================================${NC}"
echo ""

# Run with cargo run for debugging support
# The --features flag enables metal GPU acceleration and local embeddings
#
# We use a subshell to keep stdin open long enough for the server to process
# all requests. The sleep at the end gives time for async operations (like
# embedding generation) to complete before stdin closes.
{
    echo "$ALL_REQUESTS"
    # Wait for server to process all requests (embedding can take several seconds)
    sleep 10
} | cargo run --features "metal,local-embeddings"

echo ""
echo -e "${BLUE}========================================${NC}"
echo -e "${GREEN}=== Debug Test Complete ===${NC}"
