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

# Configure git + cargo registry for private Forgejo deps
ARG GIT_AUTH_TOKEN
ARG PLUGIN_PASSWORD
RUN TOKEN="${GIT_AUTH_TOKEN:-$PLUGIN_PASSWORD}" && \
    git config --global url."http://x-access-token:${TOKEN}@100.92.54.45:3002/".insteadOf "http://100.92.54.45:3002/" && \
    printf '[registries.forgejo]\nindex = "sparse+https://repo.indexarr.net/api/packages/indexarr/cargo/"\ncredential-provider = "cargo:token"\n\n[registry]\ndefault = "forgejo"\n' > $CARGO_HOME/config.toml && \
    printf '[registries.forgejo]\ntoken = "Bearer %s"\n' "$TOKEN" > $CARGO_HOME/credentials.toml
RUN sed -i '/^\[patch\./,/^$/d' Cargo.toml

# RELEASE_OPTIMIZED=true enables fat LTO + single codegen-unit (slow but smaller binary)
ARG RELEASE_OPTIMIZED=false

# Build Rust binary (build.rs skips ng build since dist already exists)
RUN if [ "$RELEASE_OPTIMIZED" = "true" ]; then \
      export CARGO_PROFILE_RELEASE_LTO=fat \
             CARGO_PROFILE_RELEASE_CODEGEN_UNITS=1 \
             CARGO_PROFILE_RELEASE_STRIP=symbols; \
    fi && \
    cargo build --release --features webdav


FROM lscr.io/linuxserver/baseimage-alpine:3.21

RUN apk add --no-cache \
        ca-certificates \
        curl \
        7zip

COPY --from=builder /build/target/release/rustnzb /usr/local/bin/rustnzb

# s6 init: create directories and fix permissions
COPY root/ /

EXPOSE 9090

VOLUME ["/config", "/data", "/downloads"]
