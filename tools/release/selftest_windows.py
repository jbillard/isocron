#!/usr/bin/env python3
"""Checks that an iscc.exe needs nothing else (D248): on the Windows machine
of tools/windows/run.py (without LLVM, a compiler or a linker), it builds
the ten programs of the Benchmarks Game, whose outputs must be those of the
programs built here by REFERENCE (an iscc of this machine), and builds for
WebAssembly; `iscc run` runs a program.

    selftest_windows.py ISCC.EXE REFERENCE
"""

import os
import subprocess
import sys
import tempfile

PROGRAMS = [("binarytrees", "10"), ("fannkuchredux", "7"), ("fasta", "1000"), ("mandelbrot", "200"), ("nbody", "1000"),
            ("pidigits", "27"), ("spectralnorm", "100"), ("knucleotide", "0"), ("regexredux", "0"), ("revcomp", "0")]


def main():
    iscc, ref = sys.argv[1], os.path.abspath(sys.argv[2])
    root = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
    bg = os.path.join(root, "examples", "benchmarks_game")
    d = tempfile.mkdtemp(prefix="isc-winself-")
    for f in [iscc] + [os.path.join(bg, n + ".isc") for n, _ in PROGRAMS]:
        subprocess.run(["cp", f, d], check=True)
    with open(os.path.join(d, "in.txt"), "wb") as f:
        f.write(subprocess.run([ref, "run", os.path.join(bg, "fasta.isc"), "1000"], check=True, capture_output=True).stdout)
    lines = ["@echo off"]
    for n, a in PROGRAMS:
        lines += ["iscc.exe build %s.isc > %s.log 2>&1 || (echo %s: build failed & type %s.log)" % (n, n, n, n), "echo == " + n, "%s.exe %s < in.txt" % (n, a)]
    lines += ["echo == run", "iscc.exe run fasta.isc 10", "echo == wasm", "iscc.exe build fasta.isc --target wasm32-wasi -o f.wasm && echo ok"]
    with open(os.path.join(d, "go.cmd"), "w") as f:
        f.write("\r\n".join(lines) + "\r\n")
    runner = os.path.join(root, "tools", "windows", "run.py")
    out = subprocess.run([runner, "--dir", d, os.path.join(d, "go.cmd")], capture_output=True).stdout
    parts = {}
    for chunk in out.split(b"== ")[1:]:
        name, _, rest = chunk.partition(b"\r\n")
        parts[name.decode()] = rest
    fail = 0
    for n, a in PROGRAMS:
        exe = os.path.join(d, n + ".linux")
        subprocess.run([ref, "build", os.path.join(bg, n + ".isc"), "-o", exe], check=True, capture_output=True)
        want = subprocess.run([exe, a], stdin=open(os.path.join(d, "in.txt"), "rb"), capture_output=True).stdout
        same = parts.get(n) == want
        fail |= not same
        print("%s: %s" % (n, "the same output" if same else "DIFFERENT"))
    want_run = subprocess.run([ref, "run", os.path.join(bg, "fasta.isc"), "10"], capture_output=True).stdout
    run_ok = parts.get("run", b"").replace(b"\r\n", b"\n") == want_run
    wasm_ok = parts.get("wasm", b"").strip() == b"ok"
    print("iscc run: %s, wasm: %s" % ("ok" if run_ok else "DIFFERENT", "ok" if wasm_ok else "FAILED"))
    sys.exit(1 if fail or not run_ok or not wasm_ok else 0)


if __name__ == "__main__":
    main()
