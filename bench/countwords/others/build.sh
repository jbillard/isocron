#!/bin/sh
# Run in the container `isocron-bg` by others.py: the programs of the
# original benchmark (benhoyt/countwords at d8c5cf3, in /src, read only)
# built with the options of its test.sh, into /out; Zig 0.7.1, the version
# of the original measures, in /zig.
set -e
mkdir -p /tmp/w && cp -r /src/. /tmp/w && cd /tmp/w
gcc -O2 simple.c -o /out/simple-c
gcc -O2 optimized.c -o /out/optimized-c
g++ -O2 simple.cpp -o /out/simple-cpp
g++ -O2 -DNDEBUG -std=c++17 optimized.cpp -o /out/optimized-cpp
go build -o /out/simple-go simple.go
go build -o /out/optimized-go optimized.go
for r in simple optimized optimized-unsafe optimized-trie optimized-customhashmap; do
    cargo build -q --release --manifest-path rust/$r/Cargo.toml
    cp rust/$r/target/release/countwords /out/rust-$r
done
/zig/zig-linux-x86_64-0.7.1/zig build-exe -OReleaseFast --name simple-zig simple.zig && cp simple-zig /out/
/zig/zig-linux-x86_64-0.7.1/zig build-exe -OReleaseFast --name optimized-zig optimized.zig && cp optimized-zig /out/
