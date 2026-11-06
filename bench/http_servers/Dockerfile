# Servers of HTTP written in C for production (nginx; lighttpd: h2o is not in
# Debian trixie), and a generator of load, to
# measure the server of Isocron against them (TODO, asked by the author on
# 2026-11-05). Built from the image of the Benchmarks Game (its `iscc`);
# run only inside, without network: the clients and the servers on the
# loopback of the container.
FROM isocron-bg
USER root
RUN apt-get update \
    && apt-get install -y --no-install-recommends nginx-light lighttpd wrk \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /work
