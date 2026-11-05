# The Benchmarks Game, in a Docker image of ours

The ten programs of [The Computer Language Benchmarks Game](https://benchmarksgame-team.pages.debian.net/benchmarksgame/)
written in Isocron (`examples/benchmarks_game/`, DECISIONS D160), checked
byte for byte against the reference outputs of the Benchmarks Game, and the
fastest programs of mandelbrot in C, C++, Rust and Go against them
(DECISIONS D142). Their code is built and run
only inside the image `isocron-bg`, without network; nothing is installed on
the host.

| Step | Command |
| --- | --- |
| download their sources (`programs.txt`) and the reference outputs and inputs of the ten programs (`references.txt`) into `upstream/` | `./fetch.sh` (`--offline`: the archive already in `build/`) |
| build the image (Debian trixie, gcc/g++ 14.2, Rust 1.84.1, Go 1.24, iscc of the host) | `./image.sh` |
| build every program in both modes into `bin/<mode>/` | `./run.sh build` |
| check the outputs: the ten Isocron programs against the references (`references.txt`: the argument and the input of each), the others at N = 200 against their reference and N = 16000 against Isocron | `./run.sh check` |
| the measurement with the protocol of SPEC section 14, after `bench/machine.sh apply parallel` (then `restore`): the container in `isc-bench.slice`, on T physical cores, T threads, the lowest of R runs (12); `results/benchmarks_game.md` | `./run.sh measure 4 12 native` |
| rough timing (lowest of 3, not the protocol of SPEC section 14) | `docker run --rm --network none --cpuset-cpus=0-3 --user "$(id -u):$(id -g)" -v "$PWD:/work" isocron-bg bash /work/quick_time.sh 4` |

`./image.sh` must run again after each `cargo build --release` of iscc.

## Modes

* `ivybridge`: the options of their command lines (their machine is an
  i5-3330: AVX without AVX2 or FMA); Isocron `--isa avx`.
* `native`: the same options with the CPU of this machine; Isocron by
  default. C and C++ add `-ffp-contract=off`: with AVX-512, `-mno-fma` no
  longer stops gcc from contracting into FMAs, which changes the output.
  C++ #4 adds `-mno-avx512f`: its AVX-512 branch returns 0 for a byte as
  soon as one of its 8 pixels escapes, so its output is wrong on an AVX-512
  machine (theirs has none).

## Known differences of their programs

* C++ #4 rounds its number of rows up to a multiple of the number of CPUs:
  at N = 200 its output differs on a machine of more than 8 CPUs.
  N = 16000, their size, is the check: there every program gives the same
  bytes as Isocron in both modes (`check_programs.sh` prints the
  difference at N = 200 and does not count it).
* Go #4 of pidigits writes no last line shorter than 10 digits (the
  Benchmarks Game runs it with N = 10000): it is checked at N = 10000
  against the Isocron program.
* Go #5 and #4 of regex-redux bind PCRE1, which Debian trixie no longer
  has: Go #3, with the `regexp` of Go, is built instead.
* Threads: C (OpenMP) reads `OMP_NUM_THREADS`, Rust (rayon)
  `RAYON_NUM_THREADS`, Go `GOMAXPROCS`, Isocron `ISC_THREADS`; C++ #4
  starts one thread per CPU of the machine whatever the cpuset.

## Not done yet

The mode `ivybridge` measured (the programs are built and checked in it),
and the lowest of 12 runs, as the Benchmarks Game reports them.
