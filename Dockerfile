FROM nvidia/cuda:12.8.0-devel-bookworm AS builder

RUN apt-get update && \
    apt-get install -y --no-install-recommends curl build-essential pkg-config && \
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y && \
    rm -rf /var/lib/apt/lists/*
ENV PATH="/root/.cargo/bin:${PATH}"

WORKDIR /build
COPY . .
RUN cargo build --release -p speake-rs-daemon --features http,cuda

FROM nvidia/cuda:12.8.0-runtime-bookworm

RUN apt-get update && \
    apt-get install -y --no-install-recommends ffmpeg ca-certificates && \
    rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/speake-rs-daemon /usr/local/bin/speake-rs-daemon

EXPOSE 9000
ENTRYPOINT ["speake-rs-daemon", "--http", "0.0.0.0:9000"]
