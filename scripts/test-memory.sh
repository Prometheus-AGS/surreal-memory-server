#!/bin/bash
# Test script for surreal-memory-server
# Sends MCP JSON-RPC requests via stdin to test memory operations
#
# Usage:
#   ./scripts/test-memory.sh
#
# For debugging in Zed, run this script which will start the server
# and send test requests through stdin.

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}=== Surreal Memory Server Test ===${NC}"
echo ""

# Change to project root
cd "$(dirname "$0")/.."

# Build the project first
echo -e "${YELLOW}Building project with features: metal,local-embeddings...${NC}"
cargo build --features "metal,local-embeddings" 2>&1 | head -20

if [ $? -ne 0 ]; then
    echo -e "${RED}Build failed!${NC}"
    exit 1
fi

echo -e "${GREEN}Build successful!${NC}"
echo ""

# MCP JSON-RPC Messages
# Note: MCP uses JSON-RPC 2.0 over stdio

# 1. Initialize request
INIT_REQUEST='{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-client","version":"1.0.0"}}}'

# 2. Initialized notification (REQUIRED - no id field, this is a notification not a request)
# This MUST be sent after receiving the initialize response and before any other requests
INITIALIZED_NOTIFICATION='{"jsonrpc":"2.0","method":"notifications/initialized"}'

# 3. List tools request
LIST_TOOLS='{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'

# 4. Create entity request - Person: John Doe
CREATE_ENTITY_1='{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_entity","arguments":{"name":"John Doe","entity_type":"Person","observations":["Software engineer at TechCorp","Lives in San Francisco","Expert in Rust programming","Enjoys hiking and photography"]}}}'

# 5. Create entity request - Company: TechCorp
CREATE_ENTITY_2='{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_entity","arguments":{"name":"TechCorp","entity_type":"Company","observations":["Technology company founded in 2015","Headquartered in Silicon Valley","Specializes in AI and machine learning","Has over 500 employees"]}}}'

# 6. Create entity request - Project: Memory Server
CREATE_ENTITY_3='{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"create_entity","arguments":{"name":"Memory Server","entity_type":"Project","observations":["MCP-based memory server","Built with Rust and SurrealDB","Supports semantic search with embeddings","Uses Candle for local ML inference"]}}}'

# 7. Create relation: John Doe -> works_at -> TechCorp
CREATE_RELATION_1='{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"create_relation","arguments":{"from":"John Doe","to":"TechCorp","relation_type":"works_at"}}}'

# 8. Create relation: John Doe -> maintains -> Memory Server
CREATE_RELATION_2='{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"create_relation","arguments":{"from":"John Doe","to":"Memory Server","relation_type":"maintains"}}}'

# 9. Add observations to John Doe (note: parameter is "entity_name" not "name")
ADD_OBSERVATIONS='{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"add_observations","arguments":{"entity_name":"John Doe","observations":["Recently promoted to Senior Engineer","Working on vector database integration","Presented at RustConf 2024"]}}}'

# 10. Search for entities
SEARCH_ENTITIES='{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"search_entities","arguments":{"query":"Rust programming"}}}'

# 11. Semantic search
SEMANTIC_SEARCH='{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"semantic_search","arguments":{"query":"machine learning and AI technology","limit":5}}}'

# 12. Read the full graph
READ_GRAPH='{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"read_graph","arguments":{}}}'

# Combine all requests (one per line for the server to process)
# Note: initialized notification MUST come right after initialize request
ALL_REQUESTS=$(cat <<EOF
$INIT_REQUEST
$INITIALIZED_NOTIFICATION
$LIST_TOOLS
$CREATE_ENTITY_1
$CREATE_ENTITY_2
$CREATE_ENTITY_3
$CREATE_RELATION_1
$CREATE_RELATION_2
$ADD_OBSERVATIONS
$SEARCH_ENTITIES
$SEMANTIC_SEARCH
$READ_GRAPH
EOF
)

echo -e "${YELLOW}Starting server and sending test requests...${NC}"
echo ""
echo -e "${BLUE}Requests to be sent:${NC}"
echo "1. Initialize MCP connection"
echo "2. Send initialized notification (required by MCP protocol)"
echo "3. List available tools"
echo "4. Create entity: John Doe (Person)"
echo "5. Create entity: TechCorp (Company)"
echo "6. Create entity: Memory Server (Project)"
echo "7. Create relation: John Doe -> works_at -> TechCorp"
echo "8. Create relation: John Doe -> maintains -> Memory Server"
echo "9. Add observations to John Doe"
echo "10. Search for 'Rust programming'"
echo "11. Semantic search for 'machine learning and AI technology'"
echo "12. Read full knowledge graph"
echo ""

# Set environment variables for the server
export RUST_LOG=info
export EMBEDDING_PROVIDER=local
export LOCAL_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5
export MODEL_CACHE_DIR=./models
export SURREAL_MODE=embedded
export SURREAL_PATH=./data/test-memory.db

# Clean up old test data
rm -rf ./data/test-memory.db 2>/dev/null || true

echo -e "${YELLOW}Running server with embedded SurrealDB...${NC}"
echo -e "${YELLOW}(First run may take time to download embedding models)${NC}"
echo ""

# Run the server with the test requests piped to stdin
# The server will process each JSON-RPC request and respond
#
# We use a subshell to keep stdin open long enough for the server to process
# all requests. The sleep at the end gives time for async operations (like
# embedding generation) to complete before stdin closes.
{
    echo "$ALL_REQUESTS"
    # Wait for server to process all requests (embedding can take several seconds)
    sleep 10
} | ./target/debug/surreal-memory-server

echo ""
echo -e "${GREEN}=== Test Complete ===${NC}"
