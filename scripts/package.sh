#!/usr/bin/env bash
# Builds the driver and lays out ./dist the way Querya Desktop and
# querya-ext-tester read an extension: manifest.json, the executable named by
# "main", and the assets the manifest points to.
set -euo pipefail
cd "$(dirname "$0")/.."

cargo build --release
main=$(sed -n 's/.*"main": *"\([^"]*\)".*/\1/p' manifest.json)

rm -rf dist
mkdir -p "dist/$(dirname "$main")"
cp target/release/querya-example-driver "dist/$main"
cp manifest.json dist/
cp -r assets dist/
echo "dist/ ready: $(ls dist | tr '\n' ' ')"
