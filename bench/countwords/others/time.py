# Run in the container by others.py: each program on the input, its
# output checked (as normalize.py orders it), then 9 rounds interleaved,
# the minimum and the median (the original kept the minimum of 5)
import os, subprocess, time
INP = '/in/kjvbible_x10.txt'
def norm(b):
    rows = []
    for l in b.decode().splitlines():
        w, c = l.split()
        rows.append((-int(c), w))
    rows.sort()
    return rows
progs = [('grep', ['grep', 'foobar'], {}), ('grep C', ['grep', 'foobar'], {'LC_ALL': 'C'}),
         ('wc -w', ['wc', '-w'], {}), ('wc -w C', ['wc', '-w'], {'LC_ALL': 'C'})]
for n in sorted(os.listdir('/out')):
    progs.append((n, ['/out/' + n], {}))
progs.append(('isocron', ['/isc/countwords'], {'ISC_THREADS': '1'}))
ref = None
for n, cmd, env in progs[4:]:
    out = subprocess.run(cmd, stdin=open(INP, 'rb'), capture_output=True, env=dict(os.environ, **env)).stdout
    r = norm(out)
    if ref is None: ref = r
    print('%-30s %s' % (n, 'same output' if r == ref else 'OTHER OUTPUT'), flush=True)
res = {n: [] for n, _, _ in progs}
for k in range(9):
    for n, cmd, env in progs:
        with open(INP, 'rb') as f:
            t0 = time.perf_counter()
            subprocess.run(cmd, stdin=f, stdout=subprocess.DEVNULL, env=dict(os.environ, **env))
            res[n].append(time.perf_counter() - t0)
for n, _, _ in sorted(progs, key=lambda p: min(res[p[0]])):
    xs = sorted(res[n])
    print('%-30s min %.3f s  median %.3f s' % (n, xs[0], xs[4]), flush=True)
