# The environment that builds iscc for Linux (D248): LLVM and LLD as static
# libraries, then iscc linked with them, so that iscc needs nothing else.
# manylinux_2_28 (AlmaLinux 8, glibc 2.28): the iscc it builds runs on any
# Linux of x86-64 with a glibc of at least 2.28. CMake and Ninja from PyPI,
# Rust by rustup (and its WebAssembly targets, whose C library iscc embeds).
FROM quay.io/pypa/manylinux_2_28_x86_64
RUN /opt/python/cp312-cp312/bin/pip install --no-cache-dir cmake ninja \
    && ln -sf /opt/python/cp312-cp312/bin/cmake /usr/local/bin/cmake \
    && ln -sf /opt/python/cp312-cp312/bin/ninja /usr/local/bin/ninja
ENV RUSTUP_HOME=/opt/rustup CARGO_HOME=/opt/cargo PATH=/opt/cargo/bin:$PATH
RUN curl -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain stable --no-modify-path \
    && rustup target add wasm32-wasip1 wasm32-wasip1-threads x86_64-pc-windows-gnu x86_64-pc-windows-msvc \
    && chmod -R a+rwX /opt/rustup /opt/cargo
WORKDIR /work
