#!/usr/bin/env bash
# Downloads the source archive of the Benchmarks Game and extracts the
# programs we compare with (programs.txt) into upstream/, with their
# license (BSD 3-clause) and reference outputs. Their code is only built
# and run inside the Docker image (image.sh, run.sh).
set -euo pipefail
here="$(cd "$(dirname "$0")" && pwd)"
url="https://salsa.debian.org/benchmarksgame-team/benchmarksgame/-/raw/master/public/download/benchmarksgame-sourcecode.zip"
mkdir -p "$here/build"
# --offline: the archive downloaded before (build/sources.zip)
if [ "${1:-}" != --offline ] || [ ! -f "$here/build/sources.zip" ]; then
    curl -sSfL -o "$here/build/sources.zip" "$url"
fi
sha256sum "$here/build/sources.zip" | tee "$here/build/sources.sha256"
python3 -I - "$here/build/sources.zip" "$here/programs.txt" "$here/references.txt" "$here/upstream" <<'PY'
import os, sys, zipfile
z = zipfile.ZipFile(sys.argv[1])
want = [l.split()[0] for l in open(sys.argv[2]) if l.strip() and not l.startswith("#")]
dirs = {w.split("/")[0] for w in want}
# the reference outputs and inputs of every benchmark of references.txt
refs = [l.split() for l in open(sys.argv[3]) if l.strip() and not l.startswith("#")]
dirs |= {r[0] for r in refs}
names = set(z.namelist())
for d in sorted(dirs):
    want += [n for n in (d + "/LICENSE", d + "/200_out", d + "/1000_out") if n in names]
for r in refs:
    want += ["%s/%s_out" % (r[0], r[1])] + (["%s/%s" % (r[0], r[2])] if len(r) > 2 else [])
want = list(dict.fromkeys(want))
for n in want:
    if n not in names or ".." in n or n.startswith("/"):
        sys.exit("missing in the archive: " + n)
    z.extract(n, sys.argv[4])
    print("extracted", n)
PY
