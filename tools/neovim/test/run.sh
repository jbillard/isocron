#!/usr/bin/env bash
# The language server of Isocron in Neovim (D269), in a container of Debian
# 13 (its Neovim, installed in the container only: the network is needed
# for that), the repository mounted read-only: test/test.lua with
# tools/neovim/isocron.lua, in a copy of the files of the test of VS Code.
#   tools/neovim/test/run.sh [ISC_LSP]   (default: target/release/isc-lsp)
set -euo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"
lsp="$(realpath "${1:-$root/target/release/isc-lsp}")"
docker run --rm -v "$root":/src:ro -v "$lsp":/isc-lsp:ro debian:trixie-slim sh -c \
    'apt-get update -qq > /dev/null && apt-get install -y -qq --no-install-recommends neovim > /dev/null 2>&1 &&
     nvim --version | head -1 &&
     cp -r /src/tools/vscode/test/integration/workspace /w && cd /w &&
     ISC_LSP=/isc-lsp nvim --headless --clean -l /src/tools/neovim/test/test.lua'
