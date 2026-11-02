#!/usr/bin/env python3
"""Block parallelism (SPEC section 15, stage 2 phase 6), point 3 of the
improvements after the POC: Isocron and the parallel reference variants
(C with OpenMP, Rust with rayon, Go with goroutines, Zig with std.Thread)
with 1, 2, 4 and 8 threads, on the four POC kernels, small and large inputs.

Each run is pinned to the first hardware thread of the first T physical
cores; the number of threads is given to every program through
ISC_THREADS, OMP_NUM_THREADS, RAYON_NUM_THREADS and GOMAXPROCS. The protocol
is that of run.py (calibrated repetitions, interleaved runs, median, minimum,
IQR). For every implementation the report says whether its result depends on
the number of threads, and compares it with Isocron's.

Writes results/xeon.json and results/xeon.md.
Usage: bench/parallel.py [--quick] [--threads 1,2,4,8] [--sizes small,large] [--kernels ...]
"""

import argparse
import datetime
import json
import os
import re
import socket
import subprocess
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
sys.path.insert(0, HERE)
import run  # noqa: E402

IMPLS = {
    "isc": "Isocron",
    "c-gcc-omp": "C (GCC, OpenMP)",
    "rust-par": "Rust (rayon)",
    "go-par": "Go (goroutines)",
    "zig-par": "Zig (std.Thread)",
}


def primary_cpus():
    """The first hardware thread of each physical core, in core order."""
    rows = [l.split(",") for l in subprocess.run(["lscpu", "-p=CPU,CORE"], capture_output=True, text=True).stdout.splitlines()
            if l and not l.startswith("#")]
    first = {}
    for cpu, core in rows:
        first.setdefault(int(core), int(cpu))
    return [first[c] for c in sorted(first)]


def programs(kernel):
    pat = re.compile(r"^(?P<impl>isc|c-gcc-omp|rust-par|go-par|zig-par)_%s(?:_(?P<var>idiomatic|opt))?$" % run.BIN_KERNEL.get(kernel, kernel))
    out = []
    for name in sorted(os.listdir(run.BIN)):
        m = pat.match(name)
        if m:
            out.append({"bin": name, "impl": m.group("impl"), "variant": m.group("var") or "-"})
    out.sort(key=lambda p: (list(IMPLS).index(p["impl"]), p["variant"]))
    return out


def env_for(t):
    e = dict(os.environ)
    for k in ("ISC_THREADS", "OMP_NUM_THREADS", "RAYON_NUM_THREADS", "GOMAXPROCS"):
        e[k] = str(t)
    e["OMP_PROC_BIND"] = "false"
    return e


def run_once(cpus, t, exe, inputs, out, reps):
    cmd = ["taskset", "-c", ",".join(map(str, cpus[:t])), exe] + inputs + [out, str(reps)]
    p = subprocess.run(cmd, capture_output=True, text=True, env=env_for(t))
    if p.returncode != 0:
        raise RuntimeError("%s failed (%d): %s" % (" ".join(cmd), p.returncode, p.stderr.strip()))
    return int(p.stdout.split()[0])


def calibrate(cpus, t, exe, inputs, out):
    reps = 1
    for _ in range(20):
        ns = run_once(cpus, t, exe, inputs, out, reps)
        if ns >= run.MIN_MEASURE_NS:
            return reps
        reps = max(reps + 1, -(-reps * run.MIN_MEASURE_NS * 115 // (100 * max(ns, 1))))
    return reps


def measure(args):
    os.makedirs(run.OUT, exist_ok=True)
    cpus = primary_cpus()
    threads = [t for t in args.threads if t <= len(cpus)]
    res = {"date": datetime.datetime.now().isoformat(timespec="seconds"), "host": socket.gethostname(), "machine": run.machine(), "cpus": cpus,
           "threads": threads, "protocol": {"warmup": args.warmup, "measures": args.measures}, "sizes": run.SIZES, "runs": {}}
    for size in args.sizes:
        d = run.gen_data(size)
        res["runs"][size] = {}
        for kernel in args.kernels:
            inputs = [os.path.join(d, f) for f in run.KERNELS[kernel]]
            cfgs = []
            for p in programs(kernel):
                for t in threads:
                    c = dict(p, threads=t, exe=os.path.join(run.BIN, p["bin"]),
                             out=os.path.join(run.OUT, "par_%s_%s_t%d.f64" % (size, p["bin"], t)), samples=[])
                    c["reps"] = calibrate(cpus, t, c["exe"], inputs, c["out"])
                    cfgs.append(c)
            for i in range(args.warmup + args.measures):
                k = i % len(cfgs)
                for c in cfgs[k:] + cfgs[:k]:
                    ns = run_once(cpus, c["threads"], c["exe"], inputs, c["out"], c["reps"])
                    if i >= args.warmup:
                        c["samples"].append(ns / c["reps"])
            ref = next(c["out"] for c in cfgs if c["impl"] == "isc" and c["threads"] == 1)
            rows = []
            for c in cfgs:
                q1, med, q3 = run.quartiles(c["samples"])
                same_t1 = open(c["out"], "rb").read() == open(
                    next(x["out"] for x in cfgs if x["bin"] == c["bin"] and x["threads"] == threads[0]), "rb").read()
                check, rel = run.compare(c["out"], ref)
                rows.append({k: c[k] for k in ("bin", "impl", "variant", "threads", "reps", "samples")} |
                            dict(median=med, q1=q1, q3=q3, iqr=q3 - q1, min=min(c["samples"]), check=check, rel=rel,
                                 same_as_1_thread=same_t1))
                run.log("%-5s %-9s %-30s T=%d median %12.1f ns  min %12.1f  %s%s" % (
                    size, kernel, c["bin"], c["threads"], med, min(c["samples"]), check,
                    "" if same_t1 else "  (differs from 1 thread)"))
            res["runs"][size][kernel] = rows
    return res


def report(res):
    m = res["machine"]
    L = []
    w = L.append
    fixed = m.get("governor") == "performance" and m.get("no_turbo") == "1"
    th = res["threads"]
    w("# Isocron — block parallelism on %s" % run.host_label(res.get("host")))
    w("")
    w("Generated by `bench/parallel.py` on %s, machine %s (%s, %s logical CPUs). Raw data: `results/%s.json`." % (
        res["date"], run.host_label(res.get("host")), m.get("cpu"), m.get("cores"), res.get("name", "xeon")))
    w("")
    w("Isocron computes the blocks of the fixed reduction form (B = 4096 elements) and of element-wise maps on a")
    w("thread pool: block j is computed by one thread and stored in its own slot, the upper levels of the tree")
    w("stay sequential, so the result is the same for any number of threads (SPEC section 10). The reference")
    w("variants: C with OpenMP (GCC), Rust with rayon, Go with goroutines, Zig with `std.Thread.Pool`; *idiomatic*")
    w("is the usual parallel reduction of each language (OpenMP `reduction(+)`, rayon `par_iter().sum()`, one chunk")
    w("per goroutine or thread), *opt* computes the blocks of the fixed form in parallel like Isocron. Clang has")
    w("no parallel variant: `zig cc` does not ship the OpenMP runtime.")
    w("")
    if res.get("note_hist"):
        w("The two histogram kernels were %s." % res["note_hist"])
        w("")
    for note in res.get("notes", []):
        w("Note: %s." % note)
        w("")
    w("Conditions: T threads pinned to the first hardware thread of the first T physical cores (CPUs %s); %s; %d" % (
        ",".join(map(str, res["cpus"][:max(th)])), "fixed frequency (turbo off)" if fixed else
        "**frequency not fixed** (governor `%s`, turbo enabled: changing it was not permitted on this server), other "
        "processes not confined" % m.get("governor"), res["protocol"]["measures"]))
    w("interleaved measurements after %d warm-up measurements, each lasting at least 100 ms." % res["protocol"]["warmup"])
    w("")
    # summary
    w("## Medians")
    w("")
    w("Median time per call with T threads, best variant of each implementation. *Speed-up*: Isocron with T threads")
    w("versus Isocron with 1 thread.")
    w("")
    w("| Kernel | Size | " + " | ".join("%s T=%d" % (IMPLS[i], t) for i in IMPLS for t in (th if i == "isc" else [max(th)])) + " | Speed-up T=%d |" % max(th))
    w("|" + " --- |" * (2 + len(th) + len(IMPLS) - 1 + 1))
    for size, kernels in res["runs"].items():
        for kernel, rows in kernels.items():
            cells = []
            for impl in IMPLS:
                for t in (th if impl == "isc" else [max(th)]):
                    rs = [r for r in rows if r["impl"] == impl and r["threads"] == t]
                    b = min(rs, key=lambda r: r["median"]) if rs else None
                    cells.append("—" if b is None else "%s%s" % (run.fmt_ns(b["median"]), "" if b["variant"] == "-" else " (%s)" % b["variant"]))
            i1 = next(r for r in rows if r["impl"] == "isc" and r["threads"] == th[0])
            imax = next(r for r in rows if r["impl"] == "isc" and r["threads"] == max(th))
            w("| %s | %s | %s | %.2f× |" % (kernel, size, " | ".join(cells), i1["median"] / imax["median"]))
    w("")
    # determinism
    w("## Does the result depend on the number of threads?")
    w("")
    w("| Kernel | Size | Implementation | Variant | Same result for every T | Same result as Isocron |")
    w("| --- | --- | --- | --- | --- | --- |")
    for size, kernels in res["runs"].items():
        for kernel, rows in kernels.items():
            seen = {}
            for r in rows:
                key = (r["impl"], r["variant"])
                s = seen.setdefault(key, {"same": True, "iso": True, "rel": 0.0})
                s["same"] &= r["same_as_1_thread"]
                s["iso"] &= r["check"] == "identical"
                s["rel"] = max(s["rel"], r["rel"])
            for (impl, var), s in seen.items():
                w("| %s | %s | %s | %s | %s | %s |" % (kernel, size, IMPLS[impl], var, "yes" if s["same"] else "**no**",
                                                       "yes" if s["iso"] else "no (rel. up to %.1e)" % s["rel"]))
    w("")
    # details
    w("## Detailed results")
    w("")
    for size, kernels in res["runs"].items():
        n = run.SIZES[size]
        for kernel, rows in kernels.items():
            elems = run.elements(kernel, n)
            w("### %s, %s (%s elements)" % (kernel, size, "{:,}".format(elems).replace(",", " ")))
            w("")
            w("Median (minimum) per call.")
            w("")
            w("| Implementation | Variant | " + " | ".join("T=%d" % t for t in th) + " |")
            w("|" + " --- |" * (2 + len(th)))
            keys = []
            for r in rows:
                if (r["impl"], r["variant"]) not in keys:
                    keys.append((r["impl"], r["variant"]))
            for impl, var in keys:
                cells = []
                for t in th:
                    r = next((r for r in rows if (r["impl"], r["variant"], r["threads"]) == (impl, var, t)), None)
                    cells.append("—" if r is None else "%s (%s)" % (run.fmt_ns(r["median"]), run.fmt_ns(r["min"])))
                w("| %s | %s | %s |" % (IMPLS[impl], var, " | ".join(cells)))
            w("")
    return "\n".join(L)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--quick", action="store_true")
    ap.add_argument("--threads", default="1,2,4,8")
    ap.add_argument("--sizes", default="small,large")
    ap.add_argument("--kernels", default="sum,dot,std_dev,histogram,histogram_skewed,moving_average,moving_average_b,filter_sum,prefix_sum,matvec")
    ap.add_argument("--name", default="xeon")
    ap.add_argument("--report-only", action="store_true")
    args = ap.parse_args()
    args.threads = [int(t) for t in args.threads.split(",")]
    args.sizes = args.sizes.split(",")
    args.kernels = args.kernels.split(",")
    args.warmup, args.measures = (1, 3) if args.quick else (run.WARMUP, run.MEASURES)
    jpath = os.path.join(run.RESULTS, args.name + ".json")
    res = json.load(open(jpath)) if args.report_only else measure(args)
    res.setdefault("name", args.name)
    if not args.report_only:
        json.dump(res, open(jpath, "w"), indent=1)
    open(os.path.join(run.RESULTS, args.name + ".md"), "w").write(report(res) + "\n")
    run.log("wrote", os.path.join(run.RESULTS, args.name + ".md"))


if __name__ == "__main__":
    main()
