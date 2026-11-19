#!/usr/bin/env python3
"""rtiow (okaneco/rtiow, Rust) and its port to Isocron (bench/rtiow/isocron):
the times of their builds, of their renders, and their memory, on the same
images (the two programs write the same bytes; every render is compared
with the first one of the Rust program).

Under the protocol of the machine (bench/machine.sh):

  bench/machine.sh apply parallel
  bench/machine.sh run --script bench/rtiow/measure.py build
  bench/machine.sh run --script bench/rtiow/measure.py render --threads 8
  bench/machine.sh apply single
  bench/machine.sh run --script bench/rtiow/measure.py render --threads 1
  bench/rtiow/measure.py alloc      (Valgrind's DHAT: the allocations)
  bench/rtiow/measure.py memory     (the peak resident memory)
  bench/rtiow/measure.py report     (results/rtiow.md)

Each step writes bench/rtiow/results/<step>.json. Builds: a clean build of
the Rust program (its dependencies too) with the features it ships with
(`images`, `threads`) and without `images` (the port has no JPEG), its
build after one file touched, and `iscc build`; each 3 times, the median,
in wall time and in processor time (the children's user and system time).
Renders: the scene of the program (`sphere`, the third book) and the final
scene of the second book (`final`), interleaved, 5 times each (3 on one
thread), the median, the minimum, the quartiles. Memory (`memory`, apart
from the timed renders): the peak resident memory of the program alone,
`VmHWM` of its /proc/<pid>/status read every millisecond (the `ru_maxrss`
of a child counts the memory of its parent before its `exec`: the Python
process, about 14 MiB).
"""

import argparse
import datetime
import json
import os
import re
import shutil
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(os.path.dirname(HERE))
sys.path.insert(0, os.path.dirname(HERE))
import run  # noqa: E402  (machine(), quartiles())

OUT = os.path.join(HERE, "results")
WORK = os.path.join(ROOT, "target", "rtiow", "measure")
RUST_SRC = os.path.join(HERE, "rust")
ISC_MAIN = os.path.join(HERE, "isocron", "main.isc")
ISCC = os.path.join(ROOT, "target", "release", "iscc")
CARGO = os.path.expanduser("~/.cargo/bin/cargo")

# the Rust builds: name, features, RUSTFLAGS
RUST_BUILDS = [
    ("rust", ["--no-default-features", "--features", "threads"], ""),
    ("rust-native", ["--no-default-features", "--features", "threads"], "-C target-cpu=native"),
]

# the renders: scene, samples, width, height (the program's defaults: 100
# samples of 384 by 216)
RENDERS = [("sphere", 100, 384, 216), ("final", 20, 384, 216)]
SEED = 42


def log(*a):
    print(*a, file=sys.stderr, flush=True)


def timed_rusage(cmd, env=None, cwd=None):
    """Runs `cmd`: its wall time (s), its processor time (s, user and system
    of it and of its children, by wait4) and its `ru_maxrss` (not its
    memory: see `peak_memory`)."""
    t0 = time.perf_counter()
    p = subprocess.Popen(cmd, env=env, cwd=cwd, stdout=subprocess.DEVNULL, stderr=subprocess.PIPE)
    _, status, ru = os.wait4(p.pid, 0)
    wall = time.perf_counter() - t0
    err = p.stderr.read().decode()
    p.stderr.close()
    p.returncode = os.waitstatus_to_exitcode(status)
    if p.returncode != 0:
        raise RuntimeError("%s failed (%d): %s" % (" ".join(cmd), p.returncode, err[-2000:]))
    return wall, ru.ru_utime + ru.ru_stime, ru.ru_maxrss


def save(name, data):
    os.makedirs(OUT, exist_ok=True)
    path = os.path.join(OUT, name + ".json")
    with open(path, "w") as f:
        json.dump(data, f, indent=1)
    log("written", path)


def summary(xs):
    q1, med, q3 = run.quartiles(xs) if len(xs) >= 2 else (xs[0], xs[0], xs[0])
    return {"all": xs, "median": med, "min": min(xs), "q1": q1, "q3": q3}


# ----- builds -------------------------------------------------------------------


def rust_dir(name):
    return os.path.join(WORK, name)


def rust_bin(name):
    return os.path.join(rust_dir(name), "release", "rtiow")


def cargo_build(name, features, rustflags, clean):
    env = dict(os.environ, CARGO_TARGET_DIR=rust_dir(name), RUSTFLAGS=rustflags)
    if clean and os.path.isdir(rust_dir(name)):
        shutil.rmtree(rust_dir(name))
    cmd = [CARGO, "build", "--release", "--offline", "--quiet"] + features
    return timed_rusage(cmd, env=env, cwd=RUST_SRC)


def isc_bin():
    return os.path.join(WORK, "isc", "rtiow")


def isc_build(out=None, env=None, extra=()):
    os.makedirs(os.path.join(WORK, "isc"), exist_ok=True)
    cmd = [ISCC, "build", ISC_MAIN, "-o", out or isc_bin()] + list(extra)
    return timed_rusage(cmd, env=env, cwd=os.path.dirname(ISC_MAIN))


def build(args):
    res = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "machine": run.machine(), "builds": {}}
    reps = args.reps
    rows = {
        "rust-shipped-clean": lambda: cargo_build("rust-shipped", [], "", True),
        "rust-clean": lambda: cargo_build("rust", RUST_BUILDS[0][1], "", True),
        "rust-native-clean": lambda: cargo_build("rust-native", RUST_BUILDS[1][1], RUST_BUILDS[1][2], True),
        "isc": lambda: isc_build(),
    }
    for name, f in rows.items():
        walls, cpus = [], []
        for k in range(reps):
            wall, cpu, rss = f()
            log("%s %d: %.2f s (processor %.2f s)" % (name, k, wall, cpu))
            walls.append(wall)
            cpus.append(cpu)
        res["builds"][name] = {"wall": summary(walls), "cpu": summary(cpus)}
    # the Rust program built again after one of its files touched
    walls, cpus = [], []
    touched = os.path.join(RUST_SRC, "src", "material.rs")
    for k in range(reps):
        os.utime(touched)
        wall, cpu, rss = cargo_build("rust", RUST_BUILDS[0][1], "", False)
        log("rust-touched %d: %.2f s (processor %.2f s)" % (k, wall, cpu))
        walls.append(wall)
        cpus.append(cpu)
    res["builds"]["rust-touched"] = {"wall": summary(walls), "cpu": summary(cpus)}
    res["sizes"] = {
        "rust-shipped": os.path.getsize(rust_bin("rust-shipped")),
        "rust": os.path.getsize(rust_bin("rust")),
        "rust-native": os.path.getsize(rust_bin("rust-native")),
        "isc": os.path.getsize(isc_bin()),
    }
    res["versions"] = {"rustc": run.sh("%s --version" % os.path.join(os.path.dirname(CARGO), "rustc")), "iscc": run.sh("git -C %s log --format=%%h -1" % ROOT)}
    save("build", res)


# ----- renders -------------------------------------------------------------------


def cpus_for(threads):
    cpus = os.environ.get("ISC_BENCH_CPUS", "")
    if threads == 1:
        return os.environ.get("ISC_BENCH_CPU", cpus.split(",")[0] if cpus else "0")
    return cpus


def render(args):
    t = args.threads
    impls = [("rust", rust_bin("rust")), ("rust-native", rust_bin("rust-native")), ("isc", isc_bin())]
    for name, exe in impls:
        if not os.path.exists(exe):
            raise SystemExit("%s missing: run the step `build` first" % exe)
    env = dict(os.environ, RAYON_NUM_THREADS=str(t), ISC_THREADS=str(t))
    cpus = cpus_for(t)
    reps = args.reps or (5 if t > 1 else 3)
    res = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "machine": run.machine(), "threads": t, "cpus": cpus, "renders": {}}
    os.makedirs(WORK, exist_ok=True)
    for scene, samples, w, h in RENDERS:
        ref = os.path.join(WORK, "ref-%s.ppm" % scene)
        rows = {name: {"wall": [], "cpu": []} for name, _ in impls}
        same = {name: True for name, _ in impls}
        for k in range(reps):
            for name, exe in impls:
                out = os.path.join(WORK, "%s-%s.ppm" % (name, scene))
                cmd = ["taskset", "-c", cpus, exe, out, str(samples), str(w), str(h), str(SEED), scene]
                wall, cpu, _ = timed_rusage(cmd, env=env)
                if k == 0 and name == "rust":
                    shutil.copy(out, ref)
                same[name] = same[name] and open(out, "rb").read() == open(ref, "rb").read()
                rows[name]["wall"].append(wall)
                rows[name]["cpu"].append(cpu)
                log("%s %s %d: %.3f s, processor %.3f s%s" % (scene, name, k, wall, cpu, "" if same[name] else ", OTHER BYTES"))
        res["renders"][scene] = {
            "samples": samples, "width": w, "height": h, "seed": SEED,
            "impls": {name: {"wall": summary(r["wall"]), "cpu": summary(r["cpu"]), "same_bytes": same[name]} for name, r in rows.items()},
        }
    save("render-%d" % t, res)


# ----- memory --------------------------------------------------------------------


def no_huge_pages():
    """In the child, before its `exec`: no transparent huge page for it."""
    import ctypes
    ctypes.CDLL(None).prctl(41, 1, 0, 0, 0)


def peak_memory(cmd, env):
    """The peak resident memory of `cmd` alone, in KiB: `VmHWM` (the high
    water mark of the memory of its program, after its `exec`), read every
    millisecond until it exits. Without huge pages
    (`prctl(PR_SET_THP_DISABLE)`, kept across the `exec`): the system gives
    them or not by the state of its memory, and a peak moved by 2 MiB from
    one run to the next (D727)."""
    p = subprocess.Popen(cmd, env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, preexec_fn=no_huge_pages)
    hwm = 0
    while p.poll() is None:
        try:
            for line in open("/proc/%d/status" % p.pid):
                if line.startswith("VmHWM:"):
                    hwm = max(hwm, int(line.split()[1]))
        except (FileNotFoundError, ProcessLookupError, ValueError):
            pass
        time.sleep(0.001)
    if p.returncode != 0:
        raise RuntimeError("%s failed (%d)" % (" ".join(cmd), p.returncode))
    return hwm


def memory(args):
    res = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "renders": {}}
    impls = [("rust", rust_bin("rust")), ("isc", isc_bin())]
    for t in (1, 8):
        env = dict(os.environ, RAYON_NUM_THREADS=str(t), ISC_THREADS=str(t))
        for scene, samples, w, h in RENDERS + [("sphere", 2000, 2, 2), ("final", 200, 2, 2)]:
            key = "%s-%dx%d-%d" % (scene, w, h, t)
            res["renders"][key] = {"scene": scene, "samples": samples, "width": w, "height": h, "threads": t}
            for name, exe in impls:
                out = os.path.join(WORK, "mem-%s-%s.ppm" % (name, scene))
                kib = peak_memory([exe, out, str(samples), str(w), str(h), str(SEED), scene], env)
                log(key, name, kib, "KiB")
                res["renders"][key][name] = kib
    save("memory", res)


# ----- allocations ---------------------------------------------------------------


def dhat(cmd, env):
    p = subprocess.run(["valgrind", "--tool=dhat", "--dhat-out-file=/dev/null"] + cmd, env=env, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError("%s failed: %s" % (" ".join(cmd), p.stderr[-2000:]))
    def get(label):
        m = re.search(r"%s:\s+([\d,]+) bytes in ([\d,]+) blocks" % re.escape(label), p.stderr)
        return [int(m.group(1).replace(",", "")), int(m.group(2).replace(",", ""))] if m else None
    return {"total": get("Total"), "at_gmax": get("At t-gmax"), "at_end": get("At t-end")}


def alloc(args):
    # the Isocron program linked with glibc's allocator (ISC_ALLOC=glibc),
    # so that Valgrind sees its blocks, and for AVX2 (Valgrind does not
    # know AVX-512, which `iscc` uses on this processor)
    glibc = os.path.join(WORK, "isc", "rtiow-glibc")
    isc_build(glibc, env=dict(os.environ, ISC_ALLOC="glibc"), extra=["--isa", "avx2"])
    env = dict(os.environ, RAYON_NUM_THREADS="1", ISC_THREADS="1", ISC_ALLOC="glibc")
    res = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "renders": {}}
    for scene, samples, w, h in [("sphere", 10, 96, 54), ("final", 4, 96, 54)]:
        res["renders"][scene] = {"samples": samples, "width": w, "height": h}
        for name, exe in [("rust", rust_bin("rust")), ("isc", glibc)]:
            out = os.path.join(WORK, "dhat-%s-%s.ppm" % (name, scene))
            r = dhat([exe, out, str(samples), str(w), str(h), str(SEED), scene], env)
            log(scene, name, r)
            res["renders"][scene][name] = r
    save("alloc", res)


# ----- report --------------------------------------------------------------------


def report(args):
    def load(name):
        path = os.path.join(OUT, name + ".json")
        return json.load(open(path)) if os.path.exists(path) else None
    names = {"rust-shipped": "Rust, as shipped (with `images`)", "rust": "Rust", "rust-native": "Rust, `target-cpu=native`", "isc": "Isocron"}
    lines = ["# rtiow: Rust and Isocron", ""]
    b = load("build")
    if b:
        m = b["machine"]
        lines += ["%s, %s, %s; %s; Isocron %s." % (m["cpu"], m["os"], m["kernel"], b["versions"]["rustc"], b["versions"]["iscc"]), ""]
        lines += ["## Builds", "", "| Build | Wall (median) | Processor (median) |", "| --- | ---: | ---: |"]
        labels = {"rust-shipped-clean": "Rust, clean, as shipped (with `images`)", "rust-clean": "Rust, clean (without `images`)",
                  "rust-native-clean": "Rust, clean, `target-cpu=native`", "rust-touched": "Rust, one file touched", "isc": "Isocron (`iscc build`)"}
        for k, r in b["builds"].items():
            lines.append("| %s | %.2f s | %.2f s |" % (labels.get(k, k), r["wall"]["median"], r["cpu"]["median"]))
        lines += ["", "| Program | Size |", "| --- | ---: |"]
        for k, n in b["sizes"].items():
            lines.append("| %s | %d KiB |" % (names.get(k, k), n // 1024))
        lines.append("")
    for t in (8, 1):
        r = load("render-%d" % t)
        if not r:
            continue
        lines += ["## Renders, %d thread%s" % (t, "s" if t > 1 else ""), "",
                  "| Scene | Program | Wall (median) | Min | Q1 to Q3 | Processor (median) | Same bytes |", "| --- | --- | ---: | ---: | ---: | ---: | --- |"]
        for scene, x in r["renders"].items():
            for name, y in x["impls"].items():
                lines.append("| %s (%d samples, %dx%d) | %s | %.3f s | %.3f s | %.3f to %.3f s | %.3f s | %s |" % (
                    scene, x["samples"], x["width"], x["height"], names.get(name, name), y["wall"]["median"], y["wall"]["min"],
                    y["wall"]["q1"], y["wall"]["q3"], y["cpu"]["median"], "yes" if y["same_bytes"] else "NO"))
        lines.append("")
    mem = load("memory")
    if mem:
        lines += ["## Peak resident memory (`VmHWM`, without huge pages)", "", "| Scene | Threads | Rust | Isocron |", "| --- | ---: | ---: | ---: |"]
        for key, x in mem["renders"].items():
            lines.append("| %s (%d samples, %dx%d) | %d | %.1f MiB | %.1f MiB |" % (x["scene"], x["samples"], x["width"], x["height"], x["threads"], x["rust"] / 1024.0, x["isc"] / 1024.0))
        lines.append("")
    a = load("alloc")
    if a:
        lines += ["## Allocations (Valgrind's DHAT, one thread)", "", "| Scene | Program | Blocks | Bytes | Peak of the heap |", "| --- | --- | ---: | ---: | ---: |"]
        for scene, x in a["renders"].items():
            for name in ("rust", "isc"):
                y = x[name]
                lines.append("| %s (%d samples, %dx%d) | %s | %d | %d | %d |" % (scene, x["samples"], x["width"], x["height"], names[name], y["total"][1], y["total"][0], y["at_gmax"][0]))
        lines.append("")
    path = os.path.join(OUT, "rtiow.md")
    open(path, "w").write("\n".join(lines))
    log("written", path)


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("step", choices=["build", "render", "alloc", "memory", "report"])
    ap.add_argument("--threads", type=int, default=8)
    ap.add_argument("--reps", type=int, default=0)
    args = ap.parse_args()
    if args.step == "build":
        args.reps = args.reps or 3
        build(args)
    elif args.step == "render":
        render(args)
    elif args.step == "alloc":
        alloc(args)
    elif args.step == "memory":
        memory(args)
    else:
        report(args)


if __name__ == "__main__":
    main()
