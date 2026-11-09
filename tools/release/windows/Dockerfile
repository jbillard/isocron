# The environment that builds iscc for Windows (D248) on Linux: MinGW-w64
# (its gcc builds LLVM, LLD and iscc.exe; its start files and libraries are
# embedded in iscc), CMake and Ninja (a native tablegen is built first), Rust
# by rustup with the targets x86_64-pc-windows-gnu and x86_64-pc-windows-msvc
# (the runtimes iscc.exe embeds, D542).
FROM debian:trixie
RUN apt-get update \
    && apt-get install -y --no-install-recommends g++ make python3 ca-certificates curl xz-utils cmake ninja-build file \
       g++-mingw-w64-x86-64-posix gcc-mingw-w64-x86-64-posix binutils-mingw-w64-x86-64 mingw-w64-x86-64-dev \
    && rm -rf /var/lib/apt/lists/*
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:/usr/local/bin:/usr/bin:/bin
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path \
    && rustup target add x86_64-pc-windows-gnu x86_64-pc-windows-msvc wasm32-wasip1 wasm32-wasip1-threads \
    && chmod -R a+rwX /opt/rustup /opt/cargo
WORKDIR /work
