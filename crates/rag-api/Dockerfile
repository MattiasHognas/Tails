# Build stage
FROM rust:1.90.0-slim AS builder

WORKDIR /build

# Copy pre-built binary
# This avoids SSL issues in CI environments with MITM proxies
COPY target/release/rag-api /build/rag-api

# Runtime stage
FROM ubuntu:24.04

# Install required runtime dependencies
RUN apt-get update && \
    apt-get install -y --no-install-recommends \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy the binary from builder
COPY --from=builder /build/rag-api /usr/local/bin/rag-api

# Switch to non-root user (ubuntu user with UID 1000)
USER ubuntu

WORKDIR /app

# Expose the default port
EXPOSE 5191

# Set default environment variables (can be overridden at runtime)
ENV RAG_TOPK_DEFAULT=16
ENV RAG_TOPK_MAX=32
ENV RAG_SEARCH_CANDIDATES=64

ENTRYPOINT ["rag-api"]
