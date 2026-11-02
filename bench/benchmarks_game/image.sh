#!/usr/bin/env bash
# Builds the Docker image `isocron-bg` (Dockerfile): copies the iscc of the
# host (`cargo build --release` first) into the build context.
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
root="$(dirname "$(dirname "$here")")"
cp "$root/target/release/iscc" "$here/iscc"
docker build -t isocron-bg "$here"
rm -f "$here/iscc"
