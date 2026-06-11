#!/usr/bin/env bash
set -euo pipefail

docker compose up --build postgres rustfs rustfs-init whisper
