FROM rust:bookworm AS builder

WORKDIR /build
COPY . .
RUN cargo build --release -p speake-rs-daemon --features http,cuda

FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends ffmpeg ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/speake-rs-daemon /usr/local/bin/speake-rs-daemon

EXPOSE 9000
ENTRYPOINT ["speake-rs-daemon", "--http", "0.0.0.0:9000"]
