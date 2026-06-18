#!/usr/bin/env bash
set -euo pipefail

cd "$(dirname "$0")/../frontend"

if [[ ! -d node_modules ]]; then
  npm ci
fi

echo "Starting React frontend on http://127.0.0.1:4173"
echo "API requests are proxied to http://127.0.0.1:8080"
exec npm run dev -- --port 4173
