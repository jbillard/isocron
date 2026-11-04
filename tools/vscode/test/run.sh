#!/usr/bin/env bash
# The tests of the extension of VS Code (D267), in a container of Node.js:
# test/test.js with vscode-textmate and vscode-oniguruma (the engine of the
# grammars of VS Code), installed in the container only (the network is
# needed for that), the repository mounted read-only; then the package made
# by package.py, its entries listed.
#   tools/vscode/test/run.sh
set -euo pipefail
root="$(cd "$(dirname "$0")/../../.." && pwd)"
docker run --rm -v "$root":/src:ro -w /tmp node:22.11.0-alpine sh -c \
    'npm install --silent --no-audit --no-fund vscode-textmate@9.1.0 vscode-oniguruma@2.0.1 > /dev/null &&
     node --check /src/tools/vscode/extension.js &&
     NODE_PATH=/tmp/node_modules node /src/tools/vscode/test/test.js /src'
out="$(mktemp -d)"
trap 'rm -f "$out"/isocron.vsix; rmdir "$out"' EXIT
python3 "$root/tools/vscode/package.py" "$out/isocron.vsix" > /dev/null
python3 -c 'import sys, zipfile; z = zipfile.ZipFile(sys.argv[1]); assert z.testzip() is None; print("package:", ", ".join(z.namelist()))' "$out/isocron.vsix"
