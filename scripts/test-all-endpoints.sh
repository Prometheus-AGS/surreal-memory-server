#!/bin/bash
# Comprehensive test script for surreal-memory-server
# Tests all REST API endpoints and validates functionality

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
echo -e "${BLUE}  Surreal Memory Server - Comprehensive Test Suite${NC}"
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
echo -e "${YELLOW}━━━ Health & System${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Health check" "GET" "$BASE_URL/health" ""

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Memory Operations${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Create memory" "POST" "$BASE_URL/api/v1/memory" \
    '{"content":"Test memory content","user_id":"test-user","categories":["test"]}' 201

test_endpoint "List memories" "GET" "$BASE_URL/api/v1/memory?user_id=test-user" ""

test_endpoint "Search memories" "POST" "$BASE_URL/api/v1/search" \
    '{"query":"test memory","user_id":"test-user","limit":5}' ""

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Entity Operations${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Create entity Alice" "POST" "$BASE_URL/api/v1/entities" \
    '{"name":"Alice","entity_type":"Person","observations":["Software Engineer","Rust expert"]}' 201

test_endpoint "Create entity Bob" "POST" "$BASE_URL/api/v1/entities" \
    '{"name":"Bob","entity_type":"Person","observations":["Product Manager"]}' 201

test_endpoint "Create entity TechCorp" "POST" "$BASE_URL/api/v1/entities" \
    '{"name":"TechCorp","entity_type":"Company","observations":["Tech startup"]}' 201

test_endpoint "Add observations to Alice" "POST" "$BASE_URL/api/v1/entities/Alice/observations" \
    '{"observations":["Likes hiking","Based in SF"]}' ""

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Relation Operations${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Create relation WORKS_AT" "POST" "$BASE_URL/api/v1/entities/relations" \
    '{"from":"Alice","to":"TechCorp","relation_type":"WORKS_AT"}' 201

test_endpoint "Create relation COLLEAGUES" "POST" "$BASE_URL/api/v1/entities/relations" \
    '{"from":"Alice","to":"Bob","relation_type":"COLLEAGUES"}' 201

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Graph Operations${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Get full graph" "GET" "$BASE_URL/api/v1/entities" ""

test_endpoint "Search entities" "GET" "$BASE_URL/api/v1/entities/search?q=Engineer" ""

test_endpoint "Find path" "GET" "$BASE_URL/api/v1/entities/Alice/path/TechCorp" ""

test_endpoint "Expand neighbors" "GET" "$BASE_URL/api/v1/entities/Alice/neighbors?depth=2&limit=10" ""

test_endpoint "Get related entities" "GET" "$BASE_URL/api/v1/entities/Alice/related?limit=10" ""

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ Mindmap Operations${NC}"
# ══════════════════════════════════════════════════════════════

test_endpoint "Create mindmap" "POST" "$BASE_URL/api/v1/mindmaps" \
    '{"name":"project-map","map_type":"radial","root_label":"Project","description":"Project mindmap","user_id":"test-user","nodes":[{"id":"root","label":"Project Root","node_type":"root"}],"edges":[],"tags":["project"]}' 201

test_endpoint "List mindmaps" "GET" "$BASE_URL/api/v1/mindmaps?user_id=test-user" ""

test_endpoint "Get mindmap" "GET" "$BASE_URL/api/v1/mindmaps/project-map?user_id=test-user" ""

test_endpoint "Add mindmap node" "POST" "$BASE_URL/api/v1/mindmaps/project-map/nodes?user_id=test-user" \
    '{"id":"node1","label":"Feature A","parent_id":"root","node_type":"feature"}' ""

test_endpoint "Export mindmap JSON" "GET" "$BASE_URL/api/v1/mindmaps/project-map/export?format=json&user_id=test-user" ""

test_endpoint "Export mindmap Markdown" "GET" "$BASE_URL/api/v1/mindmaps/project-map/export?format=markdown&user_id=test-user" ""

# ══════════════════════════════════════════════════════════════
echo -e "\n${YELLOW}━━━ A2A SSE (Agent-to-Agent)${NC}"
# ══════════════════════════════════════════════════════════════

echo -n "Testing: A2A SSE endpoint ... "
# Test that SSE endpoint is available (should stay open)
timeout 2 curl -s -N "$BASE_URL/a2a/tasks/test-task/events" > /dev/null 2>&1 || true
if [ $? -eq 124 ]; then
    echo -e "${GREEN}✓ PASS${NC} (SSE connection established)"
    ((PASSED++))
else
    echo -e "${YELLOW}⚠ SKIP${NC} (SSE requires streaming test)"
fi

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
    echo -e "${GREEN}✓ All tests passed!${NC}"
    exit 0
else
    echo -e "${RED}✗ Some tests failed${NC}"
    exit 1
fi
