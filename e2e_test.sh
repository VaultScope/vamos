#!/usr/bin/env bash
set -e

echo "Starting E2E Instance Ordering Test..."

# 1. Start the API in the background
cd /d/Projects/Pegasus/VaultScope-API
cargo run &
API_PID=$!

# Wait for API to start
sleep 5

# 2. Check catalog
echo "Fetching catalog..."
curl -s http://localhost:3000/api/storefront/catalog > /tmp/catalog.json
cat /tmp/catalog.json

# Extract a product ID (assuming we have jq, but we can just grep)
# Actually, if we don't have products, the backend subagent is supposed to seed them.
# We will run this script AFTER the subagents finish.

kill $API_PID
echo "Done"
