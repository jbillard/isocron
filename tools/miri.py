"""The unit tests of isc-runtime-core under Miri (D574), which finds the
undefined behaviour of unsafe code that a test run does not show (reads of
memory not written, out of its block, references that alias). Miri is about
a thousand times slower than a compiled test: the tests list themselves,
are shared out among JOBS groups, each its own target directory (one cargo
at a time per directory), each test with its own process. The tests take
smaller sizes under `cfg(miri)`; the paths in assembly are compiled out
there (Miri does not run assembly), the paths of AVX2 and AVX-512 are not
compiled for the target of base. Needs the toolchain nightly with Miri:
    rustup toolchain install nightly --profile minimal --component miri,rust-src
    python3 tools/miri.py [JOBS] [TIMEOUT_SECONDS]
One line a test, then `MIRI OK` or `MIRI FAILED` with the failed tests."""
import os, subprocess, sys, time, concurrent.futures as cf
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
jobs = int(sys.argv[1]) if len(sys.argv) > 1 else 8
limit = int(sys.argv[2]) if len(sys.argv) > 2 else 1800
base = ["cargo", "+nightly", "miri", "test", "-q", "-p", "isc-runtime-core", "--lib", "--"]
def env_for(k):
    return dict(os.environ, CARGO_TARGET_DIR=os.path.join(root, "target", "miri-g%d" % k))
t0 = time.time()
listed = subprocess.run(base + ["--list"], cwd=root, env=env_for(0), stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
if listed.returncode != 0:
    print(listed.stderr[-4000:]); print("MIRI FAILED: the tests do not build"); sys.exit(1)
tests = [l[:-len(": test")] for l in listed.stdout.splitlines() if l.endswith(": test")]
groups = [tests[k::jobs] for k in range(jobs)]
def run(k):
    res = []
    for t in groups[k]:
        s = time.time()
        try:
            p = subprocess.run(base + ["--exact", t], cwd=root, env=env_for(k), stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True, timeout=limit)
            out = p.stdout
            if "1 ignored" in out:
                state = "ignored"
            elif p.returncode == 0:
                state = "ok"
            elif "Undefined Behavior" in out:
                state = "UNDEFINED BEHAVIOUR"
            else:
                state = "failed"
        except subprocess.TimeoutExpired:
            state, out = "timeout", ""
        res.append((t, state, time.time() - s, out))
    return res
# each directory built once before the groups run (the first one listed)
for k in range(1, jobs):
    subprocess.run(base + ["--list"], cwd=root, env=env_for(k), stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
failed = []
with cf.ThreadPoolExecutor(jobs) as ex:
    # each group as soon as it ends
    for fut in cf.as_completed([ex.submit(run, k) for k in range(jobs)]):
        for t, state, d, out in fut.result():
            print("%-72s %-8s %6.1f s" % (t, state, d))
            if state not in ("ok", "ignored"):
                failed.append(t)
                print(out[-3000:])
print("MIRI %s: %d tests, %d failed %s, %.1f min" % ("OK" if not failed else "FAILED", len(tests), len(failed), failed, (time.time() - t0) / 60))
sys.exit(1 if failed else 0)
