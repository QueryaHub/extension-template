#!/usr/bin/env bash
# Builds the extension and runs the Querya test harness against the database
# from testenv/compose.yml (start it first: docker compose -f testenv/compose.yml up -d --wait).
#
# Needs querya-ext-tester in PATH (the dev container installs it), or point
# QUERYA_EXT_TESTER at the binary.
set -euo pipefail
cd "$(dirname "$0")/.."

TESTER="${QUERYA_EXT_TESTER:-querya-ext-tester}"
HOST="${DB_HOST:-127.0.0.1}"
PORT="${DB_PORT:-6379}"

scripts/package.sh
"$TESTER" ./dist \
  --target-version "${TARGET_VERSION:-0.5.0}" \
  --connect "{\"host\":\"$HOST\",\"port\":$PORT}" \
  --query "GET querya:app:name" \
  --table querya:app:name \
  --sandbox \
  --junit querya-ext-test-junit.xml
