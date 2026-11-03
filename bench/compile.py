#!/usr/bin/env python3
"""Compilation times of the benchmark programs: Isocron, C (GCC, Clang
through `zig cc`), Rust, Go and Zig, with the options of bench/build.sh.

For each program:
  * cold: every cache empty (Rust: a new target directory, so rayon and the
    shared library are compiled too; Go: a new GOCACHE, so the runtime and
    the standard library are compiled; Zig and `zig cc`: a new cache, so
    compiler_rt and the libc stubs are compiled). GCC and Isocron have no
    cache: their cold and warm times are the same measurement.
  * warm: the caches filled by a first build, then the program's own source
    changed (a comment appended) and built again: the usual edit-compile cycle.
Wall time and CPU time (user + system of the compiler and its children) are
reported, median of the repetitions. The sources are copied to a scratch
directory, so the repository is never modified.

Writes results/<name>.json and results/<name>.md.
Usage: bench/compile.py [--cold 3] [--warm 5] [--kernels sum,dot,...] [--name xeon-compile]
"""

import argparse
import datetime
import json
import os
import resource
import shutil
import statistics
import subprocess
import sys
import time

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run  # noqa: E402
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mdstyle  # the marks of the verdicts (D235)

ROOT = run.ROOT
ZIG = os.environ.get("ZIG") or shutil.which("zig") or os.path.expanduser("~/.local/opt/zig-x86_64-linux-0.14.1/zig")
KERNELS = ["sum", "dot", "std_dev", "histogram", "moving_average", "filter_sum", "prefix_sum", "matvec"]
VARIANTS = ["idiomatic", "opt"]
STRICT = ["-O3", "-march=native", "-ffp-contract=off", "-std=c11", "-D_POSIX_C_SOURCE=200809L"]
LABELS = {"isc": "Isocron", "c-gcc": "C (GCC)", "c-clang": "C (Clang, zig cc)", "rust": "Rust", "go": "Go", "zig": "Zig"}


def goamd64():
    """The level bench/build.sh chooses (DECISIONS D45)."""
    flags = open("/proc/cpuinfo").read().split()
    return "v3" if all(f in flags for f in ("avx2", "fma", "bmi2")) else "v2"


def env_base():
    e = dict(os.environ)
    e["PATH"] = os.path.expanduser("~/.cargo/bin") + ":" + e.get("PATH", "")
    return e


def timed(cmd, cwd=None, env=None):
    r0 = resource.getrusage(resource.RUSAGE_CHILDREN)
    t0 = time.perf_counter()
    p = subprocess.run(cmd, cwd=cwd, env=env or env_base(), capture_output=True, text=True)
    wall = time.perf_counter() - t0
    r1 = resource.getrusage(resource.RUSAGE_CHILDREN)
    if p.returncode:
        raise RuntimeError("%s failed:\n%s" % (" ".join(cmd), p.stderr[-3000:]))
    return wall, (r1.ru_utime - r0.ru_utime) + (r1.ru_stime - r0.ru_stime)


def append_comment(path, marker):
    with open(path, "a") as f:
        f.write("\n%s %s\n" % ("--" if path.endswith(".isc") else "//", marker))


def program_list(kernels):
    out = [("isc", k, "-") for k in kernels]
    for impl in ("c-gcc", "c-clang", "rust", "go", "zig"):
        out += [(impl, k, v) for k in kernels for v in VARIANTS]
    return out


def build_commands(impl, kernel, variant, work):
    """Returns (prepare, cold_build, warm_build) closures for one program."""
    e = env_base()
    if impl == "isc":
        src = os.path.join(work, kernel + ".isc")
        iscc = os.path.join(ROOT, "target", "release", "iscc")

        def prepare():
            d = "poc" if os.path.exists(os.path.join(ROOT, "examples", "poc", kernel + ".isc")) else "phase7"
            shutil.copy(os.path.join(ROOT, "examples", d, kernel + ".isc"), src)

        def build():
            return timed([iscc, "build", src, "-o", os.path.join(work, "out")], env=e)
        return prepare, build, build, src
    if impl == "c-gcc":
        src = os.path.join(work, kernel + ".c")
        cmd = ["gcc"] + STRICT + ["-DVARIANT=%d" % VARIANTS.index(variant), "-I", os.path.join(HERE, "c"),
                                  "-o", os.path.join(work, "out"), src, "-lm"]

        def prepare():
            shutil.copy(os.path.join(HERE, "c", kernel + ".c"), src)

        def build():
            return timed(cmd, env=e)
        return prepare, build, build, src
    if impl == "c-clang":
        src = os.path.join(work, kernel + ".c")
        cache = os.path.join(work, "zig-cache")
        ez = dict(e, ZIG_GLOBAL_CACHE_DIR=cache, ZIG_LOCAL_CACHE_DIR=cache)
        cmd = [ZIG, "cc", "-target", "x86_64-linux-gnu"] + STRICT + ["-DVARIANT=%d" % VARIANTS.index(variant), "-I",
                                                                    os.path.join(HERE, "c"), "-o", os.path.join(work, "out"), src, "-lm"]

        def prepare():
            shutil.rmtree(cache, ignore_errors=True)
            shutil.copy(os.path.join(HERE, "c", kernel + ".c"), src)

        def build():
            return timed(cmd, env=ez)
        return prepare, build, build, src
    if impl == "rust":
        crate = os.path.join(work, "rust")
        target = os.path.join(work, "rust-target")
        src = os.path.join(crate, "src", "bin", "%s_%s.rs" % (kernel, variant))
        er = dict(e, RUSTFLAGS="-C target-cpu=native", CARGO_TARGET_DIR=target)
        cmd = ["cargo", "build", "--release", "-q", "--offline", "--bin", "%s_%s" % (kernel, variant)]

        def prepare():
            shutil.rmtree(crate, ignore_errors=True)
            shutil.rmtree(target, ignore_errors=True)
            shutil.copytree(os.path.join(HERE, "rust"), crate, ignore=shutil.ignore_patterns("target"))

        def build():
            return timed(cmd, cwd=crate, env=er)
        return prepare, build, build, src
    if impl == "go":
        mod = os.path.join(work, "go")
        cache = os.path.join(work, "go-cache")
        eg = dict(e, GOCACHE=cache, GOFLAGS="-mod=mod", GOAMD64=goamd64())
        pkg = "./cmd/%s_%s" % (kernel, variant)
        cmd = ["go", "build", "-o", os.path.join(work, "out"), pkg]

        def prepare():
            shutil.rmtree(mod, ignore_errors=True)
            shutil.rmtree(cache, ignore_errors=True)
            shutil.copytree(os.path.join(HERE, "go"), mod)

        def build():
            return timed(cmd, cwd=mod, env=eg)
        d = os.path.join(mod, "cmd", "%s_%s" % (kernel, variant))
        return prepare, build, build, d
    if impl == "zig":
        zdir = os.path.join(work, "zig")
        cache = os.path.join(work, "zig-cache")
        ez = dict(e, ZIG_GLOBAL_CACHE_DIR=cache, ZIG_LOCAL_CACHE_DIR=cache)
        src = os.path.join(zdir, "%s_%s.zig" % (kernel, variant))
        cmd = [ZIG, "build-exe", "-O", "ReleaseFast", "-mcpu=native", "-femit-bin=" + os.path.join(work, "out"), os.path.basename(src)]

        def prepare():
            shutil.rmtree(zdir, ignore_errors=True)
            shutil.rmtree(cache, ignore_errors=True)
            shutil.copytree(os.path.join(HERE, "zig"), zdir)

        def build():
            return timed(cmd, cwd=zdir, env=ez)
        return prepare, build, build, src
    raise ValueError(impl)


def source_file(path):
    """The file to change for a warm build (a Go package: its first file)."""
    if os.path.isdir(path):
        return os.path.join(path, sorted(f for f in os.listdir(path) if f.endswith(".go"))[0])
    return path


def measure(args):
    res = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "host": run.machine_label(), "kernels": args.kernels,
           "machine": run.machine(), "cold_reps": args.cold, "warm_reps": args.warm, "programs": []}
    work = os.path.join(HERE, "build", "compile-work")
    shutil.rmtree(work, ignore_errors=True)
    os.makedirs(work)
    try:
        for impl, kernel, variant in program_list(args.kernels):
            prepare, cold, warm, src = build_commands(impl, kernel, variant, work)
            no_cache = impl in ("isc", "c-gcc")
            colds, warms = [], []
            for i in range(args.cold):
                prepare()
                colds.append(cold())
            for i in range(args.warm):
                append_comment(source_file(src), "warm %d %f" % (i, time.time()))
                warms.append(warm())
            row = {"impl": impl, "kernel": kernel, "variant": variant, "no_cache": no_cache,
                   "cold_wall": [c[0] for c in colds], "cold_cpu": [c[1] for c in colds],
                   "warm_wall": [w[0] for w in warms], "warm_cpu": [w[1] for w in warms]}
            res["programs"].append(row)
            run.log("%-8s %-10s %-9s cold %.3f s (cpu %.3f)  warm %.3f s (cpu %.3f)" % (
                impl, kernel, variant, statistics.median(row["cold_wall"]), statistics.median(row["cold_cpu"]),
                statistics.median(row["warm_wall"]), statistics.median(row["warm_cpu"])))
    finally:
        shutil.rmtree(work, ignore_errors=True)
    return res


def fmt(s):
    return "%.2f s" % s


def report(res, name):
    m = res["machine"]
    fixed = m.get("governor") == "performance" and m.get("no_turbo") == "1"
    L = []
    w = L.append
    w("# Isocron — compilation times")
    w("")
    w("Generated by `bench/compile.py` on %s, machine %s (%s). Raw data: `results/%s.json`." % (
        res["date"], run.host_label(res.get("host")), m.get("cpu"), name))
    w("")
    w("The programs of the benchmarks (SPEC section 14) built with the options of `bench/build.sh`: Isocron")
    w("(`iscc build`: front-end, LLVM at O3, link with the embedded runtime), C with GCC and with Clang (`zig cc`),")
    w("Rust (`cargo build --release`), Go (`go build`), Zig (`zig build-exe -O ReleaseFast`). *Cold*: every cache")
    w("empty — Rust compiles rayon and the shared library of the references, Go its runtime and standard library,")
    w("Zig and `zig cc` their compiler_rt and libc stubs. *Warm*: the caches filled, the program's own source")
    w("changed and built again (the edit-compile cycle). GCC and Isocron have no cache: one measurement for")
    w("both. Wall time and CPU time (user + system, the compiler and its children), median of %d cold and %d warm" % (
        res["cold_reps"], res["warm_reps"]))
    w("builds. Conditions: %s; the compilers may use the 8 hardware threads of the 4 benchmark cores." % (
        "fixed frequency (turbo off), other processes confined" if fixed else "frequency not fixed"))
    w("")
    w("## Summary")
    w("")
    w("Median over the programs of each implementation (Isocron: %d programs; others: %d kernels × 2 variants)." % (len(res.get("kernels", KERNELS)), len(res.get("kernels", KERNELS))))
    w("")
    w("| Implementation | Warm wall | Warm CPU | Cold wall | Cold CPU |")
    w("| --- | --- | --- | --- | --- |")
    for impl, label in LABELS.items():
        rows = [r for r in res["programs"] if r["impl"] == impl]
        if not rows:
            continue
        med = lambda key: statistics.median(statistics.median(r[key]) for r in rows)
        cold = ("same (no cache)", "same") if rows[0]["no_cache"] else (fmt(med("cold_wall")), fmt(med("cold_cpu")))
        w("| %s | %s | %s | %s | %s |" % (label, fmt(med("warm_wall")), fmt(med("warm_cpu")), cold[0], cold[1]))
    w("")
    w("## Every program")
    w("")
    w("| Implementation | Kernel | Variant | Warm wall | Warm CPU | Cold wall | Cold CPU |")
    w("| --- | --- | --- | --- | --- | --- | --- |")
    for r in res["programs"]:
        med = lambda key: statistics.median(r[key])
        cold = ("—", "—") if r["no_cache"] else (fmt(med("cold_wall")), fmt(med("cold_cpu")))
        w("| %s | %s | %s | %s | %s | %s | %s |" % (LABELS[r["impl"]], r["kernel"], r["variant"], fmt(med("warm_wall")),
                                                  fmt(med("warm_cpu")), cold[0], cold[1]))
    w("")
    return "\n".join(L)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--cold", type=int, default=3)
    ap.add_argument("--warm", type=int, default=5)
    ap.add_argument("--name", default="xeon-compile")
    ap.add_argument("--kernels", default=",".join(KERNELS))
    ap.add_argument("--report-only", action="store_true")
    args = ap.parse_args()
    args.kernels = args.kernels.split(",")
    jpath = os.path.join(run.RESULTS, args.name + ".json")
    res = json.load(open(jpath)) if args.report_only else measure(args)
    if not args.report_only:
        json.dump(res, open(jpath, "w"), indent=1)
    open(os.path.join(run.RESULTS, args.name + ".md"), "w").write(mdstyle.verdicts(report(res, args.name)) + "\n")
    run.log("wrote", os.path.join(run.RESULTS, args.name + ".md"))


if __name__ == "__main__":
    main()
