"""The programs of the tests under Valgrind's memcheck (D578): each test
file of `iscc` run with `ISC_TEST_MEMCHECK=1` (`tests/common`: its programs
linked with glibc for AVX2, run under Valgrind on glibc's allocator with 2
threads), one file at a time, 2 tests at a time: a read or a write outside
the blocks of the heap, or of memory never written, fails its test.
Valgrind takes 3 to 4 GB for some programs, more than 10 for one: each file
runs in a scope of systemd whose memory is capped (MEMORY_MAX, no swap), so
that a program over it is killed alone (its test fails) and the machine
never runs short (2026-11-09: 4 tests at once with 16 threads each, three
killed by the kernel for want of memory of the whole machine); the files
stop when the memory available falls under MIN_FREE_GB.
    python3 tools/memcheck.py [FILE...]       (default: every test file)
One line a file, then `MEMCHECK OK` or `MEMCHECK FAILED`."""
import os, subprocess, sys, time
root = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
MIN_FREE_GB = 8
MEMORY_MAX = "10G"
files = sys.argv[1:] or sorted(f[:-3] for f in os.listdir(os.path.join(root, "crates/iscc/tests")) if f.endswith(".rs"))
def available_gb():
    for l in open("/proc/meminfo"):
        if l.startswith("MemAvailable:"):
            return int(l.split()[1]) / (1 << 20)
    return 0
env = dict(os.environ, ISC_TEST_MEMCHECK="1")
t0 = time.time()
failed = []
for f in files:
    if available_gb() < MIN_FREE_GB:
        print("stopped before %s: %.1f GB available" % (f, available_gb()), flush=True)
        failed.append(f)
        break
    s = time.time()
    scope = ["systemd-run", "--user", "--scope", "-q", "-p", "MemoryMax=" + MEMORY_MAX, "-p", "MemorySwapMax=0"]
    p = subprocess.run(scope + ["cargo", "test", "-q", "-p", "iscc", "--test", f, "--", "--test-threads=2"], cwd=root, env=env, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
    res = [l for l in p.stdout.splitlines() if l.startswith("test result")]
    print("%-24s %6.0f s  %s" % (f, time.time() - s, res[-1] if res else "code %d" % p.returncode), flush=True)
    if p.returncode != 0:
        failed.append(f)
        print(p.stdout[-6000:], flush=True)
print("MEMCHECK %s: %d files, %d failed %s, %.1f min" % ("OK" if not failed else "FAILED", len(files), len(failed), failed, (time.time() - t0) / 60))
sys.exit(1 if failed else 0)
