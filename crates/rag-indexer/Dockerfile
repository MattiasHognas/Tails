# Build stage
FROM rust:1.90.0-slim AS builder

WORKDIR /build

# Copy pre-built binary
# This avoids SSL issues in CI environments with MITM proxies
COPY target/release/rag-indexer /build/rag-indexer

# Runtime stage
FROM ubuntu:24.04

# Install required runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /build/rag-indexer /usr/local/bin/rag-indexer

# Create data directory for watermark and set ownership
RUN mkdir -p /data && chown ubuntu:ubuntu /data

# Switch to non-root user (ubuntu user with UID 1000)
USER ubuntu

WORKDIR /app

# Set default environment variables (can be overridden at runtime)
ENV INDEXER_WATERMARK=/data/watermark.json
ENV INDEXER_LOOKBACK_MINUTES=90

ENTRYPOINT ["rag-indexer"]
