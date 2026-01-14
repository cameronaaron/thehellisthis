# Multi-stage build: compile with Rust toolchain, run on slim runtime
FROM rust:1.81-slim AS builder

# Install minimal build deps (openssl for rustls build)
RUN apt-get update && apt-get install -y --no-install-recommends \
    pkg-config \
    libssl-dev \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Pre-copy manifests for incremental cache (if Cargo.lock exists, it will be copied)
COPY Cargo.toml Cargo.lock* ./
COPY src ./src
COPY index.html ./index.html
COPY robots.txt ./robots.txt

# Build release binary
RUN cargo build --release

# Runtime image
FROM debian:bookworm-slim AS runtime
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates \
  && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/infinite-chat /app/infinite-chat
COPY index.html ./index.html
COPY robots.txt ./robots.txt

ENV PORT=3000
ENV RUST_LOG=info
EXPOSE 3000

CMD ["./infinite-chat"]
