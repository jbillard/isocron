## The programs

The fastest of each language on the site of the Benchmarks Game (2026-11-02 and 2026-11-03):

| benchmark | C | C++ | Rust | Go |
| --- | --- | --- | --- | --- |
| binarytrees | gcc-2 | gpp-7 | rust-5 | go-2 |
| fannkuchredux | gcc-6 | gpp-6 | rust-6 | go-3 |
| fasta | gcc-3 | gpp-9 | rust-7 | go-2 |
| knucleotide | gcc | gpp-2 | rust-7 | go-7 |
| mandelbrot | gcc-6 | gpp-4 | rust-4 | go-4 |
| nbody | gcc-9 | gpp-0 | rust-9 | go-3 |
| pidigits | gcc-2 | gpp-4 | rust-4 | go-4 |
| regexredux | gcc-5 | gpp-6 | rust-7 | go-3 |
| revcomp | gcc-7 | gpp-2 | rust | go-6 |
| spectralnorm | gcc-6 | gpp-6 | rust-6 | go-4 |

## Protocol

* Machine: Intel Xeon E-2388G (8 cores, 2 hardware threads per core),
  `bench/machine.sh apply parallel`: governor `performance`, turbo off, the
  frequency fixed, every other process on the sibling threads.
* Every program runs in the Docker image `isocron-bg` without network
  (`bench/benchmarks_game/run.sh measure 4 12 native`), in the slice
  `isc-bench.slice`, on 4 CPUs (the first hardware thread of 4 physical
  cores, as the 4 cores of the machine of the Benchmarks Game), with 4
  threads (`OMP_NUM_THREADS`, `RAYON_NUM_THREADS`, `GOMAXPROCS`,
  `ISC_THREADS`; C++ #4 of mandelbrot starts one thread per CPU of the
  machine).
* Mode `native`: the options of the command lines of the Benchmarks Game
  with the CPU of this machine (`-march=native`, `target-cpu=native`,
  `GOAMD64=v3`, Isocron by default); gcc and g++ 14.2.0, rustc 1.84.1, go
  1.24.4 (Debian trixie; the Benchmarks Game has go 1.23.1).
* Sizes: binary-trees 21, fannkuch-redux 12, fasta 25 000 000, mandelbrot
  16 000, n-body 50 000 000, pidigits 10 000, spectral-norm 5 500;
  k-nucleotide and reverse-complement read the output of fasta 25 000 000,
  regex-redux that of fasta 5 000 000 (made by the fasta of Isocron, whose
  output is the reference's byte for byte).
* Every output is checked first (`run.sh check`): against the reference
  output of the Benchmarks Game, or as `bench/benchmarks_game/README.md`
  says for the two programs that cannot be (C++ #4 of mandelbrot, Go #4 of
  pidigits).

## Choices forced by the image

* Go of regex-redux: #5 (the fastest) and #4 bind PCRE1, which Debian
  trixie no longer has; #3, with the `regexp` of Go, is measured.
* The Rust programs are built with Cargo and one set of dependencies, at
  the versions that build with Rust 1.84.1 (`bench/benchmarks_game/rust`);
  the Benchmarks Game builds them with `rustc` against crates whose versions
  it does not give.

## Reading

Isocron is the fastest on mandelbrot, within a factor of 2 on
fannkuch-redux and fasta, between 2 and 4 on six programs, and 7.5 times
slower on reverse-complement. The causes found so far, and those left to
find, are in TODO ("Performance") and DECISIONS (D164 to D176).
