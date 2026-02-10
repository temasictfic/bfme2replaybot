# Build stage
FROM rust:1.93.0-slim-trixie AS builder

# Install build dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    gcc g++ libc6-dev make && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

# Copy manifests
COPY Cargo.toml Cargo.lock* ./

# Create dummy main.rs to build dependencies
RUN mkdir -p src && echo "fn main() {}" > src/main.rs

# Build dependencies (this layer will be cached)
RUN cargo build --release --locked && rm -rf src

# Copy actual source code
COPY src ./src
COPY assets ./assets

# Build the application
RUN touch src/main.rs && cargo build --release --locked

# Strip the binary for smaller size
RUN strip /app/target/release/dcreplaybot

# Runtime stage
FROM debian:trixie-slim

# Install runtime dependencies
RUN apt-get update && apt-get install -y --no-install-recommends \
    ca-certificates && \
    rm -rf /var/lib/apt/lists/*

# Non-root user for security
RUN useradd -m -u 1000 botuser

WORKDIR /app

# Copy the binary
COPY --from=builder /app/target/release/dcreplaybot /app/dcreplaybot

# Copy assets (maps and fonts)
COPY --from=builder /app/assets /app/assets

# Set environment variables
ENV RUST_LOG=info
ENV ASSETS_PATH=/app/assets

EXPOSE 8000

USER botuser

# Run the bot
CMD ["./dcreplaybot"]
