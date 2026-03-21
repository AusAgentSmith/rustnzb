FROM rust:1-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev automake autoconf
WORKDIR /app

COPY Cargo.toml Cargo.lock ./
COPY src/ src/
RUN cargo build --release && \
    ./target/release/benchnzb --help > /dev/null

# Build par2cmdline-turbo (multi-threaded, SIMD-accelerated par2)
RUN git clone --depth 1 https://github.com/animetosho/par2cmdline-turbo.git /tmp/par2turbo && \
    cd /tmp/par2turbo && \
    aclocal && automake --add-missing && autoconf && \
    ./configure && make -j"$(nproc)" && \
    cp par2 /usr/local/bin/par2

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates curl p7zip-full libgomp1 \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/benchnzb /usr/local/bin/benchnzb
COPY --from=builder /usr/local/bin/par2 /usr/local/bin/par2
ENTRYPOINT ["benchnzb"]
