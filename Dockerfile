FROM rust:1.88-alpine3.21 AS builder

RUN apk add --no-cache musl-dev build-base protoc openssl-dev openssl-libs-static curl

WORKDIR /build

COPY Cargo.toml Cargo.lock ./
COPY crates crates
COPY src src

RUN cargo build --release


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
