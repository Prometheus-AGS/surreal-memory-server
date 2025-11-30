#!/bin/bash
# Interactive test script for surreal-memory-server
# Allows sending individual MCP JSON-RPC requests for debugging
#
# Usage:
#   ./scripts/test-interactive.sh
#
# This script provides a menu to send different MCP requests to the server.

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Change to project root
cd "$(dirname "$0")/.."

# Set environment variables for the server
export RUST_LOG=info
export EMBEDDING_PROVIDER=local
export LOCAL_EMBEDDING_MODEL=BAAI/bge-small-en-v1.5
export MODEL_CACHE_DIR=./models
export SURREAL_MODE=embedded
export SURREAL_PATH=./data/interactive-test.db

show_menu() {
    echo ""
    echo -e "${BLUE}=== MCP Test Menu ===${NC}"
    echo -e "${CYAN}1${NC}) Initialize connection"
    echo -e "${CYAN}2${NC}) Send initialized notification (REQUIRED after init)"
    echo -e "${CYAN}3${NC}) List tools"
    echo -e "${CYAN}4${NC}) Create entity (John Doe)"
    echo -e "${CYAN}5${NC}) Create entity (TechCorp)"
    echo -e "${CYAN}6${NC}) Create entity (Memory Server)"
    echo -e "${CYAN}7${NC}) Create relation (John -> works_at -> TechCorp)"
    echo -e "${CYAN}8${NC}) Add observations to John Doe"
    echo -e "${CYAN}9${NC}) Search entities"
    echo -e "${CYAN}10${NC}) Semantic search"
    echo -e "${CYAN}11${NC}) Read full graph"
    echo -e "${CYAN}12${NC}) Delete entity"
    echo -e "${CYAN}c${NC}) Custom JSON request"
    echo -e "${CYAN}r${NC}) Reset database"
    echo -e "${CYAN}q${NC}) Quit"
    echo ""
    echo -e "${YELLOW}Note: After option 1, you MUST send option 2 before any other requests${NC}"
    echo ""
    echo -n "Select option: "
}

get_request() {
    case $1 in
        1)
            echo '{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"test-client","version":"1.0.0"}}}'
            ;;
        2)
            # Initialized notification - no id field, this is a notification not a request
            echo '{"jsonrpc":"2.0","method":"notifications/initialized"}'
            ;;
        3)
            echo '{"jsonrpc":"2.0","id":2,"method":"tools/list","params":{}}'
            ;;
        4)
            echo '{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"create_entity","arguments":{"name":"John Doe","entity_type":"Person","observations":["Software engineer at TechCorp","Lives in San Francisco","Expert in Rust programming","Enjoys hiking and photography"]}}}'
            ;;
        5)
            echo '{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"create_entity","arguments":{"name":"TechCorp","entity_type":"Company","observations":["Technology company founded in 2015","Headquartered in Silicon Valley","Specializes in AI and machine learning","Has over 500 employees"]}}}'
            ;;
        6)
            echo '{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"create_entity","arguments":{"name":"Memory Server","entity_type":"Project","observations":["MCP-based memory server","Built with Rust and SurrealDB","Supports semantic search with embeddings","Uses Candle for local ML inference"]}}}'
            ;;
        7)
            echo '{"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"create_relation","arguments":{"from":"John Doe","to":"TechCorp","relation_type":"works_at"}}}'
            ;;
        8)
            # Note: parameter is "entity_name" not "name"
            echo '{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"add_observations","arguments":{"entity_name":"John Doe","observations":["Recently promoted to Senior Engineer","Working on vector database integration","Presented at RustConf 2024"]}}}'
            ;;
        9)
            echo '{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"search_entities","arguments":{"query":"Rust programming"}}}'
            ;;
        10)
            echo '{"jsonrpc":"2.0","id":9,"method":"tools/call","params":{"name":"semantic_search","arguments":{"query":"machine learning and AI technology","limit":5}}}'
            ;;
        11)
            echo '{"jsonrpc":"2.0","id":10,"method":"tools/call","params":{"name":"read_graph","arguments":{}}}'
            ;;
        12)
            echo '{"jsonrpc":"2.0","id":11,"method":"tools/call","params":{"name":"delete_entity","arguments":{"name":"John Doe"}}}'
            ;;
        *)
            echo ""
            ;;
    esac
}

print_request() {
    local req="$1"
    echo -e "${YELLOW}Request:${NC}"
    echo "$req" | python3 -m json.tool 2>/dev/null || echo "$req"
}

send_request() {
    local req="$1"
    echo -e "${GREEN}Sending to server...${NC}"
    echo ""

    # Send request to server and capture response
    response=$(echo "$req" | timeout 60 ./target/debug/surreal-memory-server 2>&1)

    echo -e "${BLUE}Response:${NC}"
    echo "$response" | python3 -m json.tool 2>/dev/null || echo "$response"
}

# Main
echo -e "${BLUE}=== Surreal Memory Server Interactive Test ===${NC}"
echo ""

# Check if binary exists
if [ ! -f "./target/debug/surreal-memory-server" ]; then
    echo -e "${YELLOW}Binary not found. Building...${NC}"
    cargo build --features "metal,local-embeddings"
fi

echo -e "${GREEN}Server binary ready.${NC}"
echo -e "${YELLOW}Note: Each request starts a new server instance.${NC}"
echo -e "${YELLOW}Database persists at: $SURREAL_PATH${NC}"
echo ""
echo -e "${RED}IMPORTANT: MCP Protocol requires:${NC}"
echo -e "${RED}  1. Send 'Initialize connection' (option 1)${NC}"
echo -e "${RED}  2. Send 'initialized notification' (option 2) immediately after${NC}"
echo -e "${RED}  3. Then you can send any other requests${NC}"

while true; do
    show_menu
    read -r choice

    case $choice in
        q|Q)
            echo -e "${GREEN}Goodbye!${NC}"
            exit 0
            ;;
        r|R)
            echo -e "${YELLOW}Resetting database...${NC}"
            rm -rf "$SURREAL_PATH" 2>/dev/null || true
            echo -e "${GREEN}Database reset.${NC}"
            ;;
        c|C)
            echo -e "${CYAN}Enter JSON-RPC request (single line):${NC}"
            read -r custom_req
            if [ -n "$custom_req" ]; then
                print_request "$custom_req"
                echo ""
                send_request "$custom_req"
            fi
            ;;
        [1-9]|10|11|12)
            req=$(get_request "$choice")
            if [ -n "$req" ]; then
                print_request "$req"
                echo ""
                send_request "$req"
            fi
            ;;
        *)
            echo -e "${RED}Invalid option${NC}"
            ;;
    esac
done
