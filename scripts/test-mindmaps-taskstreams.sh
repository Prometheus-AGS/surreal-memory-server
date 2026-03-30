#!/bin/bash
# Comprehensive test for Mindmaps and TaskStreams
set -e

RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

BASE_URL="http://localhost:3001"
PASSED=0
FAILED=0

echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Mindmaps & TaskStreams Comprehensive Test Suite${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo

test_endpoint() {
    local name="$1"
    local method="$2"
    local url="$3"
    local data="$4"
    local expected_code="${5:-200}"

    echo -n "Testing: $name ... "

    if [ "$method" = "GET" ]; then
        response=$(curl -s -w "\n%{http_code}" "$url")
    else
        response=$(curl -s -w "\n%{http_code}" -X "$method" -H "Content-Type: application/json" -d "$data" "$url")
    fi

    http_code=$(echo "$response" | tail -n1)
    body=$(echo "$response" | sed '$d')

    if [ "$http_code" -eq "$expected_code" ]; then
        echo -e "${GREEN}✓ PASS${NC} (HTTP $http_code)"
        ((PASSED++))
        return 0
    else
        echo -e "${RED}✗ FAIL${NC} (Expected $expected_code, got $http_code)"
        echo -e "  Response: $body"
        ((FAILED++))
        return 1
    fi
}

# ══════════════════════════════════════════════════════════════
echo -e "${YELLOW}━━━ Task Stream Operations${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Create task stream" "POST" "$BASE_URL/api/v1/taskstreams" \
    '{"name":"feature-dev","description":"Feature development","user_id":"dev-user","model_id":"gpt-4o"}' 201

test_endpoint "Add memory to task stream" "POST" "$BASE_URL/api/v1/memory" \
    '{"content":"Implement user authentication","user_id":"dev-user","task_stream_id":"feature-dev"}' 201

test_endpoint "Add another memory" "POST" "$BASE_URL/api/v1/memory" \
    '{"content":"Add JWT token validation","user_id":"dev-user","task_stream_id":"feature-dev"}' 201

test_endpoint "List task streams" "GET" "$BASE_URL/api/v1/taskstreams?user_id=dev-user" ""

test_endpoint "Get task stream" "GET" "$BASE_URL/api/v1/taskstreams/feature-dev" ""

test_endpoint "Get task context (with token budget)" "GET" "$BASE_URL/api/v1/taskstreams/feature-dev/context?model_id=gpt-4o" ""

test_endpoint "Archive task stream" "POST" "$BASE_URL/api/v1/taskstreams/feature-dev/archive" "{}" ""

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Mindmap Operations - Simple Creation${NC}"
# ══════════════════════════════════════════════════════════════

# Test SCHEMALESS mindmap with minimal data
test_endpoint "Create minimal mindmap" "POST" "$BASE_URL/api/v1/mindmaps" \
    '{"name":"simple-map","map_type":"radial","root_label":"Root","user_id":"test-user"}' 201

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Mindmap Operations - Full Features${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Create persona mindmap" "POST" "$BASE_URL/api/v1/mindmaps" \
    '{"name":"user-persona","map_type":"radial","root_label":"User Profile","description":"User persona map","user_id":"test-user","nodes":[{"id":"root","label":"User Profile"}],"edges":[],"tags":["persona","user"]}' 201

test_endpoint "List mindmaps" "GET" "$BASE_URL/api/v1/mindmaps?user_id=test-user" ""

test_endpoint "Get specific mindmap" "GET" "$BASE_URL/api/v1/mindmaps/user-persona?user_id=test-user" ""

test_endpoint "Add node to mindmap" "POST" "$BASE_URL/api/v1/mindmaps/user-persona/nodes?user_id=test-user" \
    '{"id":"skills","label":"Technical Skills","parent_id":"root","node_type":"category"}' ""

test_endpoint "Add node with color" "POST" "$BASE_URL/api/v1/mindmaps/user-persona/nodes?user_id=test-user" \
    '{"id":"rust","label":"Rust Programming","parent_id":"skills","node_type":"skill","color":"#FF6B6B"}' ""

test_endpoint "Add node with metadata" "POST" "$BASE_URL/api/v1/mindmaps/user-persona/nodes?user_id=test-user" \
    '{"id":"experience","label":"5 years","parent_id":"rust","metadata":{"level":"expert","projects":10}}' ""

test_endpoint "Export mindmap as JSON" "GET" "$BASE_URL/api/v1/mindmaps/user-persona/export?format=json&user_id=test-user" ""

test_endpoint "Export mindmap as Markdown" "GET" "$BASE_URL/api/v1/mindmaps/user-persona/export?format=markdown&user_id=test-user" ""

test_endpoint "Export mindmap as Mermaid" "GET" "$BASE_URL/api/v1/mindmaps/user-persona/export?format=mermaid&user_id=test-user" ""

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Mindmap Types - All 5 Types${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Create Concept map" "POST" "$BASE_URL/api/v1/mindmaps" \
    '{"name":"concept-map","map_type":"concept","root_label":"Core Concept","user_id":"test-user"}' 201

test_endpoint "Create Argument map" "POST" "$BASE_URL/api/v1/mindmaps" \
    '{"name":"argument-map","map_type":"argument","root_label":"Main Claim","user_id":"test-user"}' 201

test_endpoint "Create Tree map" "POST" "$BASE_URL/api/v1/mindmaps" \
    '{"name":"tree-map","map_type":"tree","root_label":"Organization","user_id":"test-user"}' 201

test_endpoint "Create Temporal map" "POST" "$BASE_URL/api/v1/mindmaps" \
    '{"name":"temporal-map","map_type":"temporal","root_label":"Timeline","user_id":"test-user"}' 201

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Advanced Features - Task Stream Auto-Summarization${NC}"
# ══════════════════════════════════════════════════════════════

# Create stream with auto-summarization
test_endpoint "Create stream with auto-summarize" "POST" "$BASE_URL/api/v1/taskstreams" \
    '{"name":"big-project","description":"Large project","user_id":"dev-user","model_id":"gpt-4o","auto_summarize":true}' 201

# Add many memories to trigger summarization
for i in {1..5}; do
    curl -s -X POST "$BASE_URL/api/v1/memory" \
        -H "Content-Type: application/json" \
        -d "{\"content\":\"Task step $i completed successfully\",\"user_id\":\"dev-user\",\"task_stream_id\":\"big-project\"}" > /dev/null
done

test_endpoint "Verify stream has memories" "GET" "$BASE_URL/api/v1/taskstreams/big-project" ""

test_endpoint "Check token budget status" "GET" "$BASE_URL/api/v1/taskstreams/big-project/context?model_id=gpt-4o" ""

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Cleanup${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Delete mindmap" "DELETE" "$BASE_URL/api/v1/mindmaps/simple-map?user_id=test-user" "" 200

# ══════════════════════════════════════════════════════════════
echo -e "\n${BLUE}═══════════════════════════════════════════════════${NC}"
echo -e "${BLUE}  Test Summary${NC}"
echo -e "${BLUE}═══════════════════════════════════════════════════${NC}"
echo -e "Total Tests: $((PASSED + FAILED))"
echo -e "${GREEN}Passed: $PASSED${NC}"
if [ $FAILED -gt 0 ]; then
    echo -e "${RED}Failed: $FAILED${NC}"
else
    echo -e "Failed: 0"
fi
echo

if [ $FAILED -eq 0 ]; then
    echo -e "${GREEN}✓ All mindmap and task stream tests passed!${NC}"
    exit 0
else
    echo -e "${RED}✗ Some tests failed${NC}"
    exit 1
fi
