# Build stage - using Alpine for smaller image
FROM rust:1.85-alpine AS builder

# Install build dependencies
RUN apk add --no-cache musl-dev g++

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

# Runtime stage - minimal Alpine image
FROM alpine:3.19

# Install runtime dependencies
RUN apk add --no-cache ca-certificates

# Non-root user for security
RUN adduser -D -u 1000 botuser

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
