#!/usr/bin/env python3
"""countwords against the programs of the original benchmark, built from
their sources and run in one container (the image `isocron-bg`: gcc 14,
g++ 14, Go 1.24, Rust 1.84; Zig 0.7.1 as the original measures had it),
pinned to the CPU of the measures, Isocron's program with them:

  git clone https://github.com/benhoyt/countwords target/ext/countwords
  (at d8c5cf3af2f58f4ca8b4f6d59cfd3329f815c453)
  Zig 0.7.1 (ziglang.org) unpacked in target/ext/zig
  bench/countwords/measure.py input; bench/machine.sh run --script bench/countwords/measure.py build
  bench/countwords/others/others.py build
  bench/machine.sh run --script bench/countwords/others/others.py run

The scope of the measures runs without the group `docker`: `sg docker`."""
import os, shlex, subprocess, sys

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(os.path.dirname(HERE)))
EXT = os.path.join(ROOT, "target", "ext")
OUT = os.path.join(ROOT, "target", "countwords", "others")
MEASURE = os.path.join(ROOT, "target", "countwords", "measure")


def docker(args):
    cmd = ["docker", "run", "--rm"] + args
    subprocess.run(["sg", "docker", "-c", " ".join(shlex.quote(c) for c in cmd)], check=True)


def build():
    os.makedirs(os.path.join(OUT, "out"), exist_ok=True)
    docker(["-v", os.path.join(EXT, "countwords") + ":/src:ro", "-v", os.path.join(OUT, "out") + ":/out",
            "-v", os.path.join(EXT, "zig") + ":/zig:ro", "-v", os.path.join(HERE, "build.sh") + ":/build.sh:ro",
            "--entrypoint", "sh", "isocron-bg", "/build.sh"])


def run():
    cpu = os.environ.get("ISC_BENCH_CPU", "0")
    docker(["--cpuset-cpus", cpu, "-v", os.path.join(OUT, "out") + ":/out:ro",
            "-v", os.path.join(MEASURE, "isc") + ":/isc:ro", "-v", MEASURE + ":/in:ro",
            "-v", os.path.join(HERE, "time.py") + ":/time.py:ro", "--entrypoint", "python3", "isocron-bg", "/time.py"])


{"build": build, "run": run}[sys.argv[1]]()
