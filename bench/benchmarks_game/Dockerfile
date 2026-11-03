# The compilers of the Benchmarks Game programs we compare with, and what
# iscc needs to build programs (DECISIONS D142). Their programs are built
# and run only inside this image, without network; nothing is installed on
# the host. Versions: those of the Benchmarks Game command lines where
# Debian trixie allows (gcc and g++ 14.2, Rust 1.84.1), else Debian's (Go
# 1.24 instead of 1.23.1).
FROM debian:trixie
RUN echo "deb http://deb.debian.org/debian trixie-backports main" > /etc/apt/sources.list.d/backports.list \
    && apt-get update \
    && apt-get install -y --no-install-recommends gcc g++ libgomp1 golang-go python3 ca-certificates curl \
       libapr1-dev libboost-dev libgmp-dev libpcre2-dev pkg-config m4 \
    && apt-get install -y --no-install-recommends -t trixie-backports libllvm21 \
    && rm -rf /var/lib/apt/lists/*
# khash.h of klib, for the C program of k-nucleotide (its `-IInclude`)
RUN mkdir -p /opt/klib && curl -sSfL https://raw.githubusercontent.com/attractivechaos/klib/master/khash.h -o /opt/klib/khash.h \
    && sha256sum /opt/klib/khash.h
# Rust, the version of their command lines; rayon fetched and built now, so
# that the programs build offline
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/usr/local/bin:/usr/bin:/bin
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain 1.84.1 --no-modify-path
COPY rust /opt/rust-bench
RUN cd /opt/rust-bench && cargo fetch && chmod -R a+rwX /opt/rust-bench /opt/cargo
# the Go programs build in a module (none of them needs another one now)
ENV GOMODCACHE=/opt/gomod
COPY go /opt/go-bench
RUN mkdir -p /opt/gomod && chmod -R a+rwX /opt/go-bench /opt/gomod
# iscc, built on the host as usual (its runtime is inside it)
COPY iscc /usr/local/bin/iscc
WORKDIR /work
