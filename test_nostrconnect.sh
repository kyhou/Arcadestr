#!/bin/bash
# Comprehensive test for nostrconnect:// URI generation

set -e

echo "=========================================="
echo "Nostrconnect URI Generation Test Suite"
echo "=========================================="
echo ""

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

# Test counter
TESTS_PASSED=0
TESTS_FAILED=0

# Function to run a test
run_test() {
    local test_name="$1"
    local test_command="$2"
    
    echo -n "Testing: $test_name... "
    if eval "$test_command" > /dev/null 2>&1; then
        echo -e "${GREEN}✓ PASSED${NC}"
        ((TESTS_PASSED++))
    else
        echo -e "${RED}✗ FAILED${NC}"
        ((TESTS_FAILED++))
    fi
}

cd /home/joel/Sync/Projetos/Arcadestr

# Test 1: Unit tests
echo "1. Running unit tests..."
run_test "URI generation basic" "cargo test -p arcadestr-core test_generate_nostrconnect_uri_basic --quiet"
run_test "URI generation with perms" "cargo test -p arcadestr-core test_generate_nostrconnect_uri_with_perms --quiet"
run_test "URI generation with name" "cargo test -p arcadestr-core test_generate_nostrconnect_uri_with_name --quiet"
run_test "URI generation URL encoding" "cargo test -p arcadestr-core test_generate_nostrconnect_uri_url_encoding --quiet"
run_test "URI generation unique" "cargo test -p arcadestr-core test_generate_nostrconnect_uri_unique --quiet"

echo ""
echo "2. Running integration tests..."
run_test "Integration - URI generation" "cargo test -p arcadestr-core --test nostrconnect_tests test_nostrconnect_uri_generation --quiet"
run_test "Integration - URI parsing" "cargo test -p arcadestr-core --test nostrconnect_tests test_nostrconnect_uri_parses_correctly --quiet"
run_test "Integration - Multiple URIs unique" "cargo test -p arcadestr-core --test nostrconnect_tests test_multiple_uris_are_unique --quiet"

echo ""
echo "3. Building components..."
run_test "Core library build" "cargo build -p arcadestr-core --quiet"
run_test "Desktop app build" "cargo build -p arcadestr-desktop --quiet"

echo ""
echo "=========================================="
echo "Test Summary"
echo "=========================================="
echo -e "Tests Passed: ${GREEN}$TESTS_PASSED${NC}"
echo -e "Tests Failed: ${RED}$TESTS_FAILED${NC}"
echo ""

if [ $TESTS_FAILED -eq 0 ]; then
    echo -e "${GREEN}✓ All tests passed!${NC}"
    echo ""
    echo "The nostrconnect:// URI generation is working correctly."
    echo ""
    echo "To test manually:"
    echo "1. Run: cargo tauri dev"
    echo "2. Click 'Generate nostrconnect:// URI' button"
    echo "3. A URI should appear in the text area"
    exit 0
else
    echo -e "${RED}✗ Some tests failed!${NC}"
    exit 1
fi
