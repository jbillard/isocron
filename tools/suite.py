"""The complete suite (`cargo test`, then `ISC_ALLOC=glibc cargo test`, then
the tests of isc-mem in its variants of x86-64) in
a few minutes instead of a quarter of an hour (D223): one build, then every
test binary run twice (Isocron's allocator, then glibc's), JOBS at a time,
the longest first (their times of the previous run, in LOG.times), each
allocator with its own directory of temporary files (ISC_TEST_TMP); the doc
tests once per allocator. One line per binary in LOG, then a summary line
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
# the longest first (their time in the previous run, if any)
prev = {}
try:
    for l in open(log + ".times"):
        n, s = l.split(); prev[n] = float(s)
except OSError:
    pass
runs = [(bn, cwd, alloc) for alloc in ("", "glibc") for bn, cwd in bins]
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
with open(log + ".times", "w") as f:
    for n, d in times.items():
        f.write("%s %.1f\n" % (n, d))
out.write("SUITE %s: %d runs, %d failed %s, %.1f min\n" % ("OK" if not failed else "FAILED", len(runs), len(failed), failed, (time.time() - t0) / 60))
sys.exit(1 if failed else 0)
