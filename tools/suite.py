"""The complete suite (`cargo test`, then `ISC_ALLOC=glibc cargo test`, then
`ISC_ALLOC=check cargo test` (every program of the tests on the allocator
checked, D546: a block freed twice, written past its end or after its free,
left at the end of `main`, D575), then
the tests of isc-mem in its variants of x86-64, and clippy on the crates that
deny its lints for each target and level of x86-64, D572) in
a few minutes instead of a quarter of an hour (D223): one build, then every
test binary run three times (Isocron's allocator, glibc's, the checked one),
JOBS at a time,
the longest first (their times of the previous run, in LOG.times), each
allocator with its own directory of temporary files (ISC_TEST_TMP); the doc
tests once for Isocron's allocator and once for glibc's. One line per binary in LOG, then a summary line
`SUITE OK` or `SUITE FAILED` with the failed binaries; exit code 1 on a
failure.
    python3 tools/suite.py WORKTREE TARGET LOG JOBS
    python3 tools/suite.py . target suite.log 6"""
import json, os, subprocess, sys, time, concurrent.futures as cf
wt, target, log, jobs = sys.argv[1], sys.argv[2], sys.argv[3], int(sys.argv[4])
env = dict(os.environ, CARGO_TARGET_DIR=target)
out = open(log, "w", buffering=1)
t0 = time.time()
b = subprocess.run(["cargo", "test", "--no-run", "--message-format=json"], cwd=wt, env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
if b.returncode != 0:
    out.write(b.stderr + "\nBUILD FAILED\n"); sys.exit(1)
bins = []
for l in b.stdout.splitlines():
    try:
        m = json.loads(l)
    except ValueError:
        continue
    if m.get("reason") == "compiler-artifact" and m.get("executable") and m.get("profile", {}).get("test"):
        bins.append((m["executable"], os.path.dirname(m["manifest_path"])))
out.write("build %.0f s, %d binaries\n" % (time.time() - t0, len(bins)))
# clippy (D572) on the crates that deny its lints (`isc-mem`,
# `isc-runtime-core`, `isc-runtime`: every unsafe operation in a block with
# its reason, D564 to D569), for every target the runtime is built for and
# the levels of x86-64 whose code differs (AVX2, AVX-512), each in its own
# directory (incremental from one suite to the next); two at a time while
# the tests run; a configuration whose target is not installed is skipped
installed = subprocess.run(["rustup", "target", "list", "--installed"], stdout=subprocess.PIPE, stderr=subprocess.DEVNULL, text=True).stdout.split()
lint_configs = []
for name, triple, flags in (("x86-64", None, ""), ("x86-64-v3", None, "-C target-cpu=x86-64-v3"), ("x86-64-v4", None, "-C target-cpu=x86-64-v4"),
                            ("windows-gnu", "x86_64-pc-windows-gnu", ""), ("windows-msvc", "x86_64-pc-windows-msvc", ""),
                            ("wasip1", "wasm32-wasip1", ""), ("wasip1-threads", "wasm32-wasip1-threads", "--cfg=isc_wasm_threads")):
    for crates, args in (("core", ["-p", "isc-runtime-core", "-p", "isc-mem"]), ("runtime", ["--manifest-path", "crates/isc-runtime/Cargo.toml"])):
        lint_configs.append(("%s %s" % (crates, name), triple, flags, args, os.path.join(target, "clippy-%s-%s" % (crates, name))))
def lint(c):
    label, triple, flags, args, tdir = c
    if triple and triple not in installed:
        return label, None, "skipped: target %s not installed" % triple
    e = dict(env, CARGO_TARGET_DIR=tdir)
    e.pop("RUSTFLAGS", None)
    if flags:
        e["RUSTFLAGS"] = flags
    p = subprocess.run(["cargo", "clippy", "-q"] + args + (["--target", triple] if triple else []), cwd=wt, env=e, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return label, p.returncode, p.stdout
lint_pool = cf.ThreadPoolExecutor(2)
lint_futures = [lint_pool.submit(lint, c) for c in lint_configs]
# the longest first (their time in the previous run, if any)
prev = {}
try:
    for l in open(log + ".times"):
        n, s = l.split(); prev[n] = float(s)
except OSError:
    pass
runs = [(bn, cwd, alloc) for alloc in ("", "glibc", "check") for bn, cwd in bins]
runs.sort(key=lambda r: -prev.get(os.path.basename(r[0]).rsplit("-", 1)[0], 1000))
def run(r):
    bn, cwd, alloc = r
    e = dict(env)
    if alloc:
        e["ISC_ALLOC"] = alloc
    # each allocator its own directory of temporary files
    e["ISC_TEST_TMP"] = os.path.join(target, "isc-tests-" + (alloc or "isc"))
    os.makedirs(e["ISC_TEST_TMP"], exist_ok=True)
    s = time.time()
    p = subprocess.run([bn], cwd=cwd, env=e, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    return (bn, alloc), p.returncode, time.time() - s, p.stdout
failed = []
times = {}
with cf.ThreadPoolExecutor(jobs) as ex:
    for (bn, alloc), code, d, text in ex.map(run, runs):
        name = os.path.basename(bn).rsplit("-", 1)[0]
        times[name] = max(times.get(name, 0), d)
        res = [l for l in text.splitlines() if l.startswith("test result")]
        out.write("%-28s %-6s %6.1f s  %s\n" % (name, alloc or "isc", d, res[-1] if res else "code %d" % code))
        if code != 0:
            failed.append((name, alloc))
            out.write(text[-4000:] + "\n")
for alloc in ("", "glibc"):
    e = dict(env)
    if alloc:
        e["ISC_ALLOC"] = alloc
    p = subprocess.run(["cargo", "test", "--doc", "-q"], cwd=wt, env=e, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    out.write("doc tests %-6s code %d\n" % (alloc or "isc", p.returncode))
    if p.returncode != 0:
        failed.append(("doc", alloc)); out.write(p.stdout[-4000:] + "\n")
# isc-mem in its variants (D288): its tests with its own symbols, so that
# the whole test process copies, fills and compares with them, for x86-64
# and, when the processor has them, x86-64-v3 and v4, each built apart
cpu = open("/proc/cpuinfo").read() if os.path.exists("/proc/cpuinfo") else ""
for level, need in (("", ""), ("x86-64-v3", "avx2"), ("x86-64-v4", "avx512bw")):
    if need and (" " + need + " ") not in cpu:
        continue
    e = dict(env)
    if level:
        e["RUSTFLAGS"] = "-C target-cpu=" + level
        e["CARGO_TARGET_DIR"] = os.path.join(target, "mem-" + level)
    p = subprocess.run(["cargo", "test", "-q", "-p", "isc-mem", "--features", "symbols"], cwd=wt, env=e, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    res = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    out.write("isc-mem %-10s       %s\n" % (level or "x86-64", res[0] if res else "code %d" % p.returncode))
    if p.returncode != 0:
        failed.append(("isc-mem", level or "x86-64")); out.write(p.stdout[-4000:] + "\n")
# isc-runtime-core for x86-64-v4 when the processor has it (D306): its
# searches of AVX-512 (VBMI, GFNI) against its general ones, compiled only
# there
if " avx512bw " in cpu:
    e = dict(env)
    e["RUSTFLAGS"] = "-C target-cpu=x86-64-v4"
    e["CARGO_TARGET_DIR"] = os.path.join(target, "core-x86-64-v4")
    p = subprocess.run(["cargo", "test", "-q", "-p", "isc-runtime-core"], cwd=wt, env=e, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    res = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    out.write("isc-runtime-core x86-64-v4  %s\n" % (res[0] if res else "code %d" % p.returncode))
    if p.returncode != 0:
        failed.append(("isc-runtime-core", "x86-64-v4")); out.write(p.stdout[-4000:] + "\n")
for fut in lint_futures:
    label, code, text = fut.result()
    if code is None:
        out.write("clippy %-24s %s\n" % (label, text))
        continue
    out.write("clippy %-24s %s\n" % (label, "ok" if code == 0 else "code %d" % code))
    if code != 0:
        failed.append(("clippy", label)); out.write(text[-4000:] + "\n")
lint_pool.shutdown()
with open(log + ".times", "w") as f:
    for n, d in times.items():
        f.write("%s %.1f\n" % (n, d))
out.write("SUITE %s: %d runs, %d failed %s, %.1f min\n" % ("OK" if not failed else "FAILED", len(runs), len(failed), failed, (time.time() - t0) / 60))
sys.exit(1 if failed else 0)
