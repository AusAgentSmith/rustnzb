FROM rust:1.88-alpine3.21 AS builder

RUN apk add --no-cache musl-dev build-base protoc openssl-dev openssl-libs-static curl nodejs npm git

WORKDIR /build

# Install Angular dependencies first (cached layer)
COPY frontend/package.json frontend/package-lock.json frontend/
RUN cd frontend && npm ci

# Copy frontend source and build
COPY frontend frontend
RUN cd frontend && npx ng build --configuration=production

# Copy Rust source
COPY Cargo.toml Cargo.lock build.rs ./
COPY src src
COPY tests tests

# Configure git for private deps and strip local [patch] overrides
ARG GITHUB_TOKEN
RUN git config --global url."https://x-access-token:${GITHUB_TOKEN}@github.com/".insteadOf "https://github.com/"
RUN sed -i '/^\[patch\./,/^$/d' Cargo.toml

# Build Rust binary (build.rs skips ng build since dist already exists)
RUN cargo build --release


FROM lscr.io/linuxserver/baseimage-alpine:3.23

RUN apk add --no-cache \
        ca-certificates \
        curl \
        7zip

COPY --from=builder /build/target/release/rustnzb /usr/local/bin/rustnzb

# s6 init: create directories and fix permissions
COPY root/ /

EXPOSE 9090

VOLUME ["/config", "/data", "/downloads"]
