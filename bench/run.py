#!/usr/bin/env python3
"""Benchmark protocol of SPEC section 14 for the POC (task P6).

For every kernel, input size and implementation built by `build.sh`:
  * the repetition count r is calibrated so that one measurement (one
    process run, r timed calls) lasts at least 100 ms;
  * 5 warm-up measurements are discarded, then 30 are recorded;
  * the median and the interquartile range of the time per call are reported;
  * the result written by the program is compared with Isocron's, and
    Isocron's compiled result with its reference interpreter.

Writes results/poc.json (raw data) and results/poc.md (report).

Usage: bench/run.py [--quick] [--sizes small,large] [--kernels sum,dot] [--cpu 3]
"""

import argparse
import datetime
import json
import math
import os
import random
import re
import statistics
import struct
import subprocess
import sys
import time
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import mdstyle  # the marks of the verdicts (D235)

HERE = os.path.dirname(os.path.abspath(__file__))
ROOT = os.path.dirname(HERE)
BIN = os.path.join(HERE, "bin")
DATA = os.path.join(HERE, "data")
OUT = os.path.join(HERE, "build", "out")
RESULTS = os.path.join(ROOT, "results")

# Small: fits in the L2 cache of the test machine (1 MiB per pair of cores):
# 256 KiB per input array. Large: 10^7 elements (section 14).
SIZES = {
    "small": {"f64": 32_768, "u8": 262_144, "matrix": (128, 256)},
    "large": {"f64": 10_000_000, "u8": 10_000_000, "matrix": (4096, 4096)},
}
KERNELS = {
    "sum": ["a.f64"],
    "dot": ["a.f64", "b.f64"],
    "std_dev": ["a.f64"],
    "histogram": ["u.u8"],
    # the histogram programs on a skewed input: value k with probability
    # 2^-(k+1), so that successive increments often hit the same counter
    "histogram_skewed": ["s.u8"],
    # SPEC section 14, phase 7 (examples/phase7)
    "moving_average": ["a.f64"],
    # comparison B (section 14): the best algorithm of each language, O(n)
    "moving_average_b": ["a.f64"],
    "filter_sum": ["a.f64"],
    "prefix_sum": ["i.f64"],
    "matvec": ["m.f64", "v.f64"],
}
# kernel -> name of its programs
BIN_KERNEL = {"histogram_skewed": "histogram"}


def elements(kernel, n):
    """The elements a kernel reads (cycles per element in the reports)."""
    if kernel.startswith("histogram"):
        return n["u8"]
    if kernel == "matvec":
        return n["matrix"][0] * n["matrix"][1]
    return n["f64"]
MIN_MEASURE_NS = 100_000_000
WARMUP = 5
MEASURES = 30


def log(*a):
    print(*a, file=sys.stderr, flush=True)


# ---------------------------------------------------------------------------
# data


def gen_f64(path, n, seed):
    rng = random.Random(seed)
    chunk = 1 << 16
    with open(path, "wb") as f:
        left = n
        while left > 0:
            k = min(chunk, left)
            xs = [rng.uniform(-1.0, 1.0) * 10.0 ** rng.randint(-3, 3) for _ in range(k)]
            f.write(struct.pack("<%dd" % k, *xs))
            left -= k


def gen_data(size):
    d = os.path.join(DATA, size)
    os.makedirs(d, exist_ok=True)
    n = SIZES[size]
    for name, seed in (("a.f64", 1), ("b.f64", 2)):
        p = os.path.join(d, name)
        if not os.path.exists(p) or os.path.getsize(p) != 8 * n["f64"]:
            log("generating", p)
            gen_f64(p, n["f64"], seed + (0 if size == "small" else 100))
    p = os.path.join(d, "u.u8")
    if not os.path.exists(p) or os.path.getsize(p) != n["u8"]:
        log("generating", p)
        with open(p, "wb") as f:
            f.write(random.Random(3 if size == "small" else 103).randbytes(n["u8"]))
    # integers 0 .. 999 stored as f64 (the input of prefix_sum)
    p = os.path.join(d, "i.f64")
    if not os.path.exists(p) or os.path.getsize(p) != 8 * n["f64"]:
        log("generating", p)
        rng = random.Random(5 if size == "small" else 105)
        with open(p, "wb") as f:
            left = n["f64"]
            while left > 0:
                k = min(1 << 16, left)
                f.write(struct.pack("<%dd" % k, *[float(rng.randrange(1000)) for _ in range(k)]))
                left -= k
    # a matrix (row after row) and a vector (matvec)
    rows, cols = n["matrix"]
    for name, count, seed in (("m.f64", rows * cols, 6), ("v.f64", cols, 7)):
        p = os.path.join(d, name)
        if not os.path.exists(p) or os.path.getsize(p) != 8 * count:
            log("generating", p)
            rng = random.Random(seed + (0 if size == "small" else 100))
            with open(p, "wb") as f:
                left = count
                while left > 0:
                    k = min(1 << 16, left)
                    f.write(struct.pack("<%dd" % k, *[rng.uniform(-1.0, 1.0) for _ in range(k)]))
                    left -= k
    p = os.path.join(d, "s.u8")
    if not os.path.exists(p) or os.path.getsize(p) != n["u8"]:
        log("generating", p)
        rng = random.Random(4 if size == "small" else 104)
        # geometric: the number of leading zero bits of a random 64-bit word
        out = bytearray(n["u8"])
        for i in range(n["u8"]):
            w = rng.getrandbits(64)
            out[i] = 64 - w.bit_length() if w else 64
        with open(p, "wb") as f:
            f.write(bytes(out))
    return d


# ---------------------------------------------------------------------------
# implementations

LABELS = {
    "isc": ("Isocron", "isocron"),
    # `allow_reassoc` (section 10), compared with the reassociated variants
    "isc-reassoc": ("Isocron (allow_reassoc)", "isocron-reassoc"),
    "c-gcc": ("C (GCC)", "strict"),
    "c-clang": ("C (Clang)", "strict"),
    "c-gcc-reassoc": ("C (GCC, reassociated)", "reassoc"),
    "c-clang-reassoc": ("C (Clang, reassociated)", "reassoc"),
    "rust": ("Rust", "strict"),
    "go": ("Go", "strict"),
    "zig": ("Zig", "strict"),
    "zig-reassoc": ("Zig (reassociated)", "reassoc"),
    # parallel variants: measured by bench/parallel.py, not here
    "c-gcc-omp": ("C (GCC, OpenMP)", "parallel"),
    "rust-par": ("Rust (rayon)", "parallel"),
    "go-par": ("Go (goroutines)", "parallel"),
    "zig-par": ("Zig (std.Thread)", "parallel"),
}


def implementations(kernel):
    pat = re.compile(r"^(?P<impl>[a-z-]+)_%s(?:_(?P<var>idiomatic|opt))?$" % BIN_KERNEL.get(kernel, kernel))
    out = []
    for name in sorted(os.listdir(BIN)):
        m = pat.match(name)
        if not m:
            continue
        impl, var = m.group("impl"), m.group("var") or "-"
        label, kind = LABELS[impl]
        if kind == "parallel":
            continue
        out.append({"bin": name, "impl": impl, "label": label, "variant": var, "kind": kind})
    out.sort(key=lambda r: (r["kind"] != "isocron", r["bin"]))
    return out


def run_once(cpu, exe, inputs, out, reps):
    cmd = ["taskset", "-c", str(cpu), exe] + inputs + [out, str(reps)]
    p = subprocess.run(cmd, capture_output=True, text=True)
    if p.returncode != 0:
        raise RuntimeError("%s failed (%d): %s" % (" ".join(cmd), p.returncode, p.stderr.strip()))
    return int(p.stdout.split()[0])


def calibrate(cpu, exe, inputs, out):
    reps = 1
    for _ in range(20):
        ns = run_once(cpu, exe, inputs, out, reps)
        if ns >= MIN_MEASURE_NS:
            return reps
        reps = max(reps + 1, math.ceil(reps * MIN_MEASURE_NS * 1.15 / max(ns, 1)))
    return reps


def effective_ghz(cpu, exe, inputs, out, reps):
    """Effective frequency of the core running the process: all its cycles
    (user and kernel, since loading the input happens in the kernel) per
    nanosecond of task clock, measured by `perf stat` (None when the counters
    are not available)."""
    cmd = ["perf", "stat", "-x", ",", "-e", "cycles,task-clock", "taskset", "-c", str(cpu), exe] + inputs + [out, str(reps)]
    try:
        p = subprocess.run(cmd, capture_output=True, text=True, env=dict(os.environ, LC_ALL="C"))
    except FileNotFoundError:
        return None
    cycles = msec = None
    for line in p.stderr.splitlines():
        f = line.split(",")
        if len(f) > 2 and f[2].startswith("cycles") and f[0].replace(".", "").isdigit():
            cycles = float(f[0])
        if len(f) > 2 and f[2] == "task-clock":
            try:
                msec = float(f[0])
            except ValueError:
                pass
    if not cycles or not msec:
        return None
    return cycles / (msec * 1e6)


def quartiles(xs):
    q = statistics.quantiles(xs, n=4, method="inclusive")
    return q[0], statistics.median(xs), q[2]


def compare(path, ref):
    a = open(path, "rb").read()
    b = open(ref, "rb").read()
    if a == b:
        return "identical", 0.0
    xs = struct.unpack("<%dd" % (len(a) // 8), a)
    ys = struct.unpack("<%dd" % (len(b) // 8), b)
    if len(xs) != len(ys):
        return "WRONG LENGTH", float("inf")
    rel = 0.0
    for x, y in zip(xs, ys):
        if x != y:
            rel = max(rel, abs(x - y) / max(abs(y), 1e-300))
    return "differs", rel


def max_diffs(path, ref):
    """Largest absolute and relative differences of two f64 files of the
    same length (comparison B against the exact sums of the windows)."""
    xs = struct.unpack("<%dd" % (os.path.getsize(path) // 8), open(path, "rb").read())
    ys = struct.unpack("<%dd" % (os.path.getsize(ref) // 8), open(ref, "rb").read())
    if len(xs) != len(ys):
        return float("inf"), float("inf")
    ab = rel = 0.0
    for x, y in zip(xs, ys):
        if x != y:
            d = abs(x - y)
            ab = max(ab, d)
            rel = max(rel, d / max(abs(y), 1e-300))
    return ab, rel


# ---------------------------------------------------------------------------
# machine description


def sh(cmd):
    try:
        return subprocess.run(cmd, shell=True, capture_output=True, text=True).stdout.strip()
    except Exception:
        return ""


# The name of the machine in the reports: $ISC_BENCH_MACHINE, else its CPU.
# Never its host name, which would identify the machine in a public
# repository (DECISIONS D163).
def machine_label():
    return os.environ.get("ISC_BENCH_MACHINE") or machine().get("cpu") or "unknown CPU"


def host_label(h):
    return h or machine_label()


def machine():
    lscpu = sh("LC_ALL=C lscpu")
    get = lambda k: next((l.split(":", 1)[1].strip() for l in lscpu.splitlines() if l.startswith(k + ":")), "?")
    zig = os.environ.get("ZIG") or sh("command -v zig") or os.path.expanduser("~/.local/opt/zig-x86_64-linux-0.14.1/zig")
    os_name = sh(". /etc/os-release && echo $PRETTY_NAME")
    return {
        "cpu": get("Model name"),
        "cores": get("CPU(s)"),
        "max_mhz": get("CPU max MHz"),
        "l1d": get("L1d cache"),
        "l2": get("L2 cache"),
        "flags_avx": "avx" in get("Flags").split(),
        "flags_avx2": "avx2" in get("Flags").split(),
        "flags_fma": "fma" in get("Flags").split(),
        "flags_avx512f": "avx512f" in get("Flags").split(),
        "ram": sh("LC_ALL=C free -h | awk '/Mem:/ {print $2}'"),
        "os": os_name,
        "kernel": sh("uname -r"),
        "governor": sh("cat /sys/devices/system/cpu/cpu0/cpufreq/scaling_governor"),
        "no_turbo": sh("cat /sys/devices/system/cpu/intel_pstate/no_turbo 2>/dev/null || cat /sys/devices/system/cpu/cpufreq/boost 2>/dev/null"),
        "gcc": sh("gcc --version | head -1"),
        "clang": sh("%s cc --version | head -1" % zig),
        "rustc": sh("PATH=$HOME/.cargo/bin:$PATH rustc --version"),
        "go": sh("go version"),
        "zig": sh("%s version" % zig),
        "llvm": "LLVM " + sh("ls /usr/lib/x86_64-linux-gnu/ | grep -o 'libLLVM.so.[0-9.]*' | head -1 | sed 's/libLLVM.so.//'"),
        "python": sys.version.split()[0],
    }


# ---------------------------------------------------------------------------
# main


def measure(args):
    os.makedirs(OUT, exist_ok=True)
    results = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "machine": machine(), "cpu": args.cpu,
               "protocol": {"warmup": args.warmup, "measures": args.measures, "min_measure_ns": MIN_MEASURE_NS},
               "sizes": SIZES, "runs": {}}
    iscc = os.path.join(ROOT, "target", "release", "iscc")
    for size in args.sizes:
        d = gen_data(size)
        results["runs"][size] = {}
        for kernel in args.kernels:
            inputs = [os.path.join(d, f) for f in KERNELS[kernel]]
            ref = os.path.join(OUT, "%s_%s_isc.f64" % (size, kernel))
            prog = BIN_KERNEL.get(kernel, kernel)
            run_once(args.cpu, os.path.join(BIN, "isc_" + prog), inputs, ref, 1)
            # the compiled program against the reference interpreter
            interp_out = os.path.join(OUT, "%s_%s_interp.f64" % (size, kernel))
            t0 = time.time()
            src = next(f for f in (os.path.join(ROOT, "examples", d, prog + ".isc") for d in ("poc", "phase7")) if os.path.exists(f))
            subprocess.run([iscc, "run", src] + inputs + [interp_out, "1"], check=True, capture_output=True)
            oracle = compare(ref, interp_out)[0]
            log("%s %s: compiled vs interpreter: %s (%.1fs)" % (size, kernel, oracle, time.time() - t0))
            impls = implementations(kernel)
            for impl in impls:
                impl["exe"] = os.path.join(BIN, impl["bin"])
                impl["out"] = os.path.join(OUT, "%s_%s.f64" % (size, impl["bin"]))
                impl["reps"] = calibrate(args.cpu, impl["exe"], inputs, impl["out"])
                impl["samples"] = []
            # Interleaved measurements: one run of every implementation per
            # round, in an order that rotates from round to round, so that a
            # slow drift of the machine affects all of them alike.
            for i in range(args.warmup + args.measures):
                k = i % len(impls)
                for impl in impls[k:] + impls[:k]:
                    ns = run_once(args.cpu, impl["exe"], inputs, impl["out"], impl["reps"])
                    if i >= args.warmup:
                        impl["samples"].append(ns / impl["reps"])
            rows = []
            for impl in impls:
                samples = impl["samples"]
                q1, med, q3 = quartiles(samples)
                check, rel = compare(impl["out"], ref)
                ghz = effective_ghz(args.cpu, impl["exe"], inputs, impl["out"], impl["reps"])
                row = {k: v for k, v in impl.items() if k not in ("exe", "out")}
                row.update(median=med, q1=q1, q3=q3, iqr=q3 - q1, min=min(samples), check=check, rel=rel, ghz=ghz)
                if impl["kind"] == "isocron":
                    row["oracle"] = oracle
                # comparison B: the distance to the exact sums of the windows
                exact = os.path.join(OUT, "%s_moving_average_isc.f64" % size)
                if kernel == "moving_average_b" and os.path.exists(exact):
                    row["exact_abs"], row["exact_rel"] = max_diffs(impl["out"], exact)
                rows.append(row)
                log("%-5s %-9s %-34s r=%-6d median %12.1f ns  min %12.1f  IQR %9.1f  %s GHz  %s" % (
                    size, kernel, impl["bin"], impl["reps"], med, min(samples), q3 - q1,
                    "%.3f" % ghz if ghz else "?", check if check == "identical" else "%s %.2e" % (check, rel)))
            results["runs"][size][kernel] = rows
    return results


def fmt_ns(ns):
    if ns >= 1e6:
        return "%.3f ms" % (ns / 1e6)
    if ns >= 1e3:
        return "%.2f µs" % (ns / 1e3)
    return "%.0f ns" % ns


def verdict(isc, other):
    """SPEC section 14: beats if the median is lower by more than the
    measured dispersion (the larger of the two IQRs)."""
    disp = max(isc["iqr"], other["iqr"])
    if isc["median"] < other["median"] - disp:
        return "beats"
    if isc["median"] <= other["median"] + disp:
        return "equals"
    return "slower"


def best(rows, pred):
    c = [r for r in rows if pred(r)]
    return min(c, key=lambda r: r["median"]) if c else None


def report(res, name="poc"):
    m = res["machine"]
    L = []
    w = L.append
    w("# Isocron POC — benchmark results")
    w("")
    w("Generated by `bench/run.py` on %s (protocol of SPEC section 14, task P6)." % res["date"])
    w("Raw data, including every sample: `results/%s.json`." % name)
    w("")
    if res.get("note_hist"):
        w("The two histogram kernels were %s." % res["note_hist"])
        w("")
    for note in res.get("notes", []):
        w("Note: %s." % note)
        w("")
    # ---- summary table of medians
    w("## Medians")
    w("")
    w("Median time per call of the kernel (lower is better). For each language, the best of its strict")
    w("variants (idiomatic and hand-optimized); the reassociated C/Zig variants are listed separately because")
    w("they may change the floating-point result, next to Isocron's `allow_reassoc` programs")
    w("(`examples/reassoc/`, section 14).")
    w("")
    langs = [("Isocron", lambda r: r["kind"] == "isocron"),
             ("C (GCC)", lambda r: r["impl"] == "c-gcc"),
             ("C (Clang)", lambda r: r["impl"] == "c-clang"),
             ("Rust", lambda r: r["impl"] == "rust"),
             ("Go", lambda r: r["impl"] == "go"),
             ("Zig", lambda r: r["impl"] == "zig"),
             ("Isocron allow_reassoc", lambda r: r["kind"] == "isocron-reassoc"),
             ("best reassociated", lambda r: r["kind"] == "reassoc")]
    w("| Kernel | Size | " + " | ".join(n for n, _ in langs) + " |")
    w("|" + " --- |" * (len(langs) + 2))
    for size, kernels in res["runs"].items():
        for kernel, rows in kernels.items():
            cells = []
            for _, pred in langs:
                b = best(rows, pred)
                cells.append("—" if b is None else "%s%s" % (fmt_ns(b["median"]), "" if b["kind"].startswith("isocron") or b["variant"] == "-" else " (%s)" % b["variant"]))
            w("| %s | %s | %s |" % (kernel, size, " | ".join(cells)))
    w("")
    # ---- decision point
    w("## Decision point (SPEC section 15)")
    w("")
    w("> On the small size (L2 cache), if Isocron does not at least equal the best strict variant of C and of")
    w("> Rust on the sum and the dot product, the generated assembly is analyzed before going further.")
    w("")
    w("Verdict rule (section 14): *beats* when Isocron's median is lower by more than the dispersion")
    w("(the larger IQR of the two); *equals* when the medians are within the dispersion; *slower* otherwise.")
    w("")
    w("| Kernel | Size | Isocron | Best strict C | Verdict | Best Rust | Verdict |")
    w("| --- | --- | --- | --- | --- | --- | --- |")
    decision_ok = True
    for size, kernels in res["runs"].items():
        for kernel, rows in kernels.items():
            isc = best(rows, lambda r: r["kind"] == "isocron")
            bc = best(rows, lambda r: r["impl"] in ("c-gcc", "c-clang"))
            br = best(rows, lambda r: r["impl"] == "rust")
            vc, vr = verdict(isc, bc), verdict(isc, br)
            if size == "small" and kernel in ("sum", "dot") and "slower" in (vc, vr):
                decision_ok = False
            w("| %s | %s | %s ± %s | %s (%s, %s) ± %s | **%s** | %s (%s) ± %s | **%s** |" % (
                kernel, size, fmt_ns(isc["median"]), fmt_ns(isc["iqr"]), fmt_ns(bc["median"]), bc["label"], bc["variant"],
                fmt_ns(bc["iqr"]), vc, fmt_ns(br["median"]), br["variant"], fmt_ns(br["iqr"]), vr))
    w("")
    if "small" in res["runs"] and all(k in res["runs"]["small"] for k in ("sum", "dot")):
        if decision_ok:
            w("**Result: the decision point is passed** — on the small size, Isocron at least equals the best strict")
            w("variant of C and of Rust on the sum and the dot product.")
        else:
            w("**Result: the decision point is not passed** — on the small size, Isocron is slower than the best")
            w("strict variant of C or Rust on the sum or the dot product; the generated assembly must be analyzed")
            w("(see the analysis section below).")
    w("")
    w("Cases where Isocron's median is above the best strict median of another language, even when the")
    w("verdict is *equals* (difference within the dispersion):")
    w("")
    slower = []
    for size, kernels in res["runs"].items():
        for kernel, rows in kernels.items():
            isc = best(rows, lambda r: r["kind"] == "isocron")
            b = best(rows, lambda r: r["kind"] == "strict")
            if b["median"] < isc["median"]:
                mins = ""
                if "min" in b and "min" in isc:
                    mins = "; minimums %s vs %s, %+.1f %%" % (fmt_ns(b["min"]), fmt_ns(isc["min"]),
                                                             100 * (isc["min"] / b["min"] - 1))
                slower.append("* %s, %s: %s (%s) is %.1f %% faster than Isocron (medians %s vs %s; dispersion %s%s)." % (
                    kernel, size, b["label"], b["variant"], 100 * (isc["median"] / b["median"] - 1),
                    fmt_ns(b["median"]), fmt_ns(isc["median"]), fmt_ns(max(b["iqr"], isc["iqr"])), mins))
    L.extend(slower or ["* none"])
    w("")
    w("Cases where the best reassociated median (C, Zig) is below the median of Isocron's `allow_reassoc`")
    w("program (section 14):")
    w("")
    slower_r = []
    for size, kernels in res["runs"].items():
        for kernel, rows in kernels.items():
            ir = best(rows, lambda r: r["kind"] == "isocron-reassoc")
            br = best(rows, lambda r: r["kind"] == "reassoc")
            if ir and br and br["median"] < ir["median"]:
                slower_r.append("* %s, %s: %s (%s) is %.1f %% faster than Isocron allow_reassoc (medians %s vs %s; dispersion %s)." % (
                    kernel, size, br["label"], br["variant"], 100 * (ir["median"] / br["median"] - 1),
                    fmt_ns(br["median"]), fmt_ns(ir["median"]), fmt_ns(max(br["iqr"], ir["iqr"]))))
    L.extend(slower_r or ["* none"])
    w("")
    w("On the large size every float kernel is limited by memory bandwidth, so all vectorized implementations")
    w("reach the same level (section 14: the large size checks scaling, it does not decide). The histogram is")
    w("limited by the store-to-load dependency of `h[x[i]] += 1`; Isocron's code is the same loop as C's")
    w("idiomatic one (`movzbl` then `addl $1, (h, x, 4)`), plus the allocation of its 256 counters per call.")
    w("")
    # ---- detail tables
    w("## Detailed results")
    w("")
    w("`r`: calibrated repetitions per measurement (each measurement lasts at least 100 ms). Median, minimum and")
    w("IQR of the time per call over %d measurements after %d warm-up measurements, interleaved between" % (
        res["protocol"]["measures"], res["protocol"]["warmup"]))
    w("implementations. *Cycles per element*: median time multiplied by the effective frequency measured by")
    w("`perf stat` (all cycles / task clock) on one extra run. *Result*: comparison of the written")
    w("output with Isocron's output (bit for bit; otherwise the largest relative difference).")
    w("")
    for size, kernels in res["runs"].items():
        n = SIZES[size]
        for kernel, rows in kernels.items():
            elems = elements(kernel, n)
            w("### %s, %s (%s elements)" % (kernel, size, "{:,}".format(elems).replace(",", " ")))
            w("")
            isc = best(rows, lambda r: r["kind"] == "isocron")
            w("| Implementation | Variant | r | Median | Min | IQR | Cycles per element | vs Isocron | Result |")
            w("| --- | --- | --- | --- | --- | --- | --- | --- | --- |")
            for r in sorted(rows, key=lambda r: r["median"]):
                res_txt = r["check"] if r["check"] == "identical" else "%s (rel. %.1e)" % (r["check"], r["rel"])
                if r["kind"] == "isocron":
                    res_txt = "reference; compiled vs interpreter: %s" % r.get("oracle", "?")
                cpe = "%.3f" % (r["median"] * r["ghz"] / elems) if r.get("ghz") else "?"
                w("| %s%s | %s | %d | %s | %s | %s | %s | %.2f× | %s |" % (
                    "**" if r["kind"] == "isocron" else "", r["label"] + ("**" if r["kind"] == "isocron" else ""),
                    r["variant"], r["reps"], fmt_ns(r["median"]), fmt_ns(r["min"]) if "min" in r else "?",
                    fmt_ns(r["iqr"]), cpe, r["median"] / isc["median"], res_txt))
            w("")
    # ---- floating-point gaps
    w("## Floating-point differences")
    w("")
    w("Isocron's reduction has the fixed form of section 10 (V = 32 interleaved accumulators, blocks of 4096")
    w("elements, then a tree). Every hand-optimized (`opt`) variant implements the same form and is bit-identical.")
    w("The idiomatic variants add sequentially, from left to right, so they round differently; the reassociated")
    w("variants let the compiler choose the order. The histogram is exact everywhere.")
    w("")
    w("| Kernel | Size | Implementation | Variant | Largest relative difference |")
    w("| --- | --- | --- | --- | --- |")
    for size, kernels in res["runs"].items():
        for kernel, rows in kernels.items():
            for r in rows:
                if r["check"] != "identical" and r["kind"] != "isocron":
                    w("| %s | %s | %s | %s | %.2e |" % (kernel, size, r["label"], r["variant"], r["rel"]))
    w("")
    b_rows = [(size, r) for size, kernels in res["runs"].items() for r in kernels.get("moving_average_b", []) if "exact_rel" in r]
    if b_rows:
        w("### Comparison B: the moving average in O(n)")
        w("")
        w("Comparison B (section 14) lets each language use its best algorithm: Isocron computes the prefix")
        w("sums (`scan`) and one difference per window; C, Rust, Go and Zig a running sum. Both round")
        w("differently from the exact sums of the windows (the output of `moving_average`, the fixed form):")
        w("")
        w("| Size | Implementation | Variant | Largest absolute difference | Largest relative difference |")
        w("| --- | --- | --- | --- | --- |")
        for size, r in b_rows:
            w("| %s | %s | %s | %.2e | %.2e |" % (size, r["label"], r["variant"], r["exact_abs"], r["exact_rel"]))
        w("")
    # ---- machine and protocol
    w("## Machine and toolchains")
    w("")
    w("| Item | Value |")
    w("| --- | --- |")
    for k, label in [("cpu", "CPU"), ("cores", "Logical CPUs"), ("max_mhz", "Max frequency (MHz)"), ("l1d", "L1d cache"),
                     ("l2", "L2 cache"), ("ram", "RAM"), ("os", "OS"), ("kernel", "Linux kernel"),
                     ("governor", "CPU frequency governor"), ("no_turbo", "intel_pstate no_turbo"),
                     ("llvm", "Isocron back-end"), ("gcc", "GCC"), ("clang", "Clang (zig cc)"), ("rustc", "Rust"),
                     ("go", "Go"), ("zig", "Zig")]:
        w("| %s | %s |" % (label, m.get(k, "?")))
    w("| AVX | %s |" % ("yes" + (", AVX2" if m.get("flags_avx2") else "") if m.get("flags_avx") else "no (SSE4.2 only: SIMD is SSE2, 2 doubles per register)"))
    w("| Pinning | `taskset -c %s` (one core) |" % res["cpu"])
    w("")
    w("Compiler options (section 14):")
    w("")
    w("| Implementation | Options |")
    w("| --- | --- |")
    w("| Isocron | `iscc build` (LLVM `default<O3>`, host CPU; no fast-math flag) |")
    w("| C strict (GCC, Clang) | `-O3 -march=native -ffp-contract=off` |")
    w("| C reassociated | `-O3 -march=native -fassociative-math -fno-signed-zeros -fno-trapping-math` |")
    w("| Rust | `cargo build --release`, `RUSTFLAGS=-C target-cpu=native` |")
    if m.get("flags_avx2"):
        w("| Go | `go build`, `GOAMD64=v3` |")
    else:
        w("| Go | `go build`, `GOAMD64=v2` (the CPU has no AVX2, so `v3` binaries do not start) |")
    w("| Zig | `zig build-exe -O ReleaseFast -mcpu=native`; reassociated: `@setFloatMode(.optimized)` |")
    w("")
    w("## Protocol and limits")
    w("")
    w("* Inputs: random `f64` values `u * 10^e` with `u` uniform in [-1, 1) and `e` uniform in {-3..3} (so that")
    w("  the order of additions matters), and uniform random bytes. Small size: %d `f64` (256 KiB per array) and" % SIZES["small"]["f64"])
    w("  %d bytes; large size: 10^7 elements." % SIZES["small"]["u8"])
    w("* Only the kernel is timed (data loading excluded); Isocron uses `bench` (section 12), the other")
    w("  languages time the same loop and protect each result against elimination (`asm` barriers in C,")
    w("  `std::hint::black_box` in Rust, `std.mem.doNotOptimizeAway` in Zig, a global sink and `//go:noinline` in Go).")
    w("* Every Isocron call allocates its result (the histogram's 256 counters) and frees it, as the language")
    w("  semantics require; the C, Rust, Go and Zig histograms use a stack array.")
    if m.get("governor") == "performance" and m.get("no_turbo") == "1":
        ghz = [r["ghz"] for ks in res["runs"].values() for rows in ks.values() for r in rows if r.get("ghz")]
        w("* Fixed frequency: governor `performance`, turbo off (`bench/machine.sh apply`, restored afterwards).")
        if ghz:
            w("  Effective frequency measured by `perf stat` (cycles / task clock): %.3f to %.3f GHz over all runs." % (min(ghz), max(ghz)))
    else:
        w("* The frequency governor (`%s`) and the turbo could not be changed (no root access, DECISIONS.md)." % m.get("governor"))
        w("  The warm-up measurements and the median limit the effect; the IQR reports it.")
    w("* Measurements are interleaved: each round runs every implementation of the kernel once, in an order")
    w("  that rotates from round to round.")
    if m.get("flags_fma"):
        w("* The CPU has FMA. Isocron never contracts `a * b + c` (section 10), and neither do the strict")
        w("  references: `-ffp-contract=off` in C, every product rounded by an explicit `float64(...)` in Go")
        w("  (DECISIONS D34), no contraction by default in Rust and Zig; the bit-identical results confirm it.")
    else:
        w("* No FMA instruction exists on this CPU, so no compiler could contract `a * b + c` (section 14, FMA).")
    if m.get("flags_avx512f"):
        w("* AVX-512: Isocron and Zig write the accumulators as explicit vectors, which LLVM keeps in 512-bit")
        w("  registers; GCC, Clang and Rust vectorize with the default tuning of `-march=native` for this CPU,")
        w("  256-bit vectors. `results/xeon-lanes.md` compares them all with 512-bit vectors preferred.")
    w("* One thread: every program is pinned to one CPU (Isocron's thread pool then has one thread). The")
    w("  parallel variants (OpenMP, rayon, goroutines, `std.Thread`) are measured in `results/xeon.md`.")
    w("")
    return "\n".join(L)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--quick", action="store_true", help="1 warm-up and 3 measurements (to test the script)")
    ap.add_argument("--sizes", default="small,large")
    ap.add_argument("--kernels", default="sum,dot,std_dev,histogram,histogram_skewed,moving_average,moving_average_b,filter_sum,prefix_sum,matvec")
    ap.add_argument("--cpu", type=int, default=int(os.environ.get("ISC_BENCH_CPU", "3")))
    ap.add_argument("--name", default="poc", help="results/<name>.md and .json")
    ap.add_argument("--report-only", action="store_true", help="rewrite poc.md from poc.json")
    ap.add_argument("--ghz-only", action="store_true", help="measure the effective frequency again (one run per implementation) and rewrite poc.md")
    args = ap.parse_args()
    args.sizes = args.sizes.split(",")
    args.kernels = args.kernels.split(",")
    args.warmup, args.measures = (1, 3) if args.quick else (WARMUP, MEASURES)
    os.makedirs(RESULTS, exist_ok=True)
    jpath = os.path.join(RESULTS, args.name + ".json")
    if args.report_only or args.ghz_only:
        res = json.load(open(jpath))
    if args.ghz_only:
        for size, kernels in res["runs"].items():
            d = os.path.join(DATA, size)
            for kernel, rows in kernels.items():
                inputs = [os.path.join(d, f) for f in KERNELS[kernel]]
                for r in rows:
                    out = os.path.join(OUT, "%s_%s.f64" % (size, r["bin"]))
                    r["ghz"] = effective_ghz(res["cpu"], os.path.join(BIN, r["bin"]), inputs, out, r["reps"])
                    log("%-5s %-9s %-34s %s GHz" % (size, kernel, r["bin"], "%.3f" % r["ghz"] if r["ghz"] else "?"))
        json.dump(res, open(jpath, "w"), indent=1)
    if not (args.report_only or args.ghz_only):
        res = measure(args)
        json.dump(res, open(jpath, "w"), indent=1)
    open(os.path.join(RESULTS, args.name + ".md"), "w").write(mdstyle.verdicts(report(res, args.name)) + "\n")
    log("wrote", os.path.join(RESULTS, args.name + ".md"))


if __name__ == "__main__":
    main()
