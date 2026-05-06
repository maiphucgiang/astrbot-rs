# AstrBot-rs Docker Image
# Multi-stage build for optimized production image

# =============================================================================
# Stage 1: Builder
# =============================================================================
FROM rust:1.80-slim-bookworm AS builder

WORKDIR /usr/src/astrbot

# Install build dependencies
RUN apt-get update && apt-get install -y \
    pkg-config \
    libssl-dev \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Copy workspace manifest files first (for layer caching)
COPY Cargo.toml Cargo.lock ./
COPY crates/astrbot-core/Cargo.toml crates/astrbot-core/
COPY crates/astrbot-platform/Cargo.toml crates/astrbot-platform/
COPY crates/astrbot-provider/Cargo.toml crates/astrbot-provider/
COPY crates/astrbot-plugin/Cargo.toml crates/astrbot-plugin/
COPY crates/astrbot-dashboard/Cargo.toml crates/astrbot-dashboard/
COPY crates/astrbot-cli/Cargo.toml crates/astrbot-cli/
COPY crates/astrbot-security/Cargo.toml crates/astrbot-security/
COPY crates/astrbot-persona/Cargo.toml crates/astrbot-persona/
COPY crates/astrbot-ux/Cargo.toml crates/astrbot-ux/
COPY crates/astrbot-feishu/Cargo.toml crates/astrbot-feishu/

# Create dummy lib.rs files to cache dependency compilation
RUN mkdir -p crates/astrbot-core/src && echo "pub fn dummy() {}" > crates/astrbot-core/src/lib.rs \
    && mkdir -p crates/astrbot-platform/src && echo "pub fn dummy() {}" > crates/astrbot-platform/src/lib.rs \
    && mkdir -p crates/astrbot-provider/src && echo "pub fn dummy() {}" > crates/astrbot-provider/src/lib.rs \
    && mkdir -p crates/astrbot-plugin/src && echo "pub fn dummy() {}" > crates/astrbot-plugin/src/lib.rs \
    && mkdir -p crates/astrbot-dashboard/src && echo "pub fn dummy() {}" > crates/astrbot-dashboard/src/lib.rs \
    && mkdir -p crates/astrbot-cli/src && echo "fn main() {}" > crates/astrbot-cli/src/main.rs \
    && mkdir -p crates/astrbot-security/src && echo "pub fn dummy() {}" > crates/astrbot-security/src/lib.rs \
    && mkdir -p crates/astrbot-persona/src && echo "pub fn dummy() {}" > crates/astrbot-persona/src/lib.rs \
    && mkdir -p crates/astrbot-ux/src && echo "pub fn dummy() {}" > crates/astrbot-ux/src/lib.rs \
    && mkdir -p crates/astrbot-feishu/src && echo "pub fn dummy() {}" > crates/astrbot-feishu/src/lib.rs

# Cache dependency build
RUN cargo build --release --bin astrbot || true

# =============================================================================
# Stage 2: Copy real source and build
# =============================================================================
# Remove dummy files
RUN rm -rf crates/*/src

# Copy full source
COPY crates/astrbot-core/src crates/astrbot-core/src
COPY crates/astrbot-platform/src crates/astrbot-platform/src
COPY crates/astrbot-provider/src crates/astrbot-provider/src
COPY crates/astrbot-plugin/src crates/astrbot-plugin/src
COPY crates/astrbot-dashboard/src crates/astrbot-dashboard/src
COPY crates/astrbot-cli/src crates/astrbot-cli/src
COPY crates/astrbot-security/src crates/astrbot-security/src
COPY crates/astrbot-persona/src crates/astrbot-persona/src
COPY crates/astrbot-ux/src crates/astrbot-ux/src
COPY crates/astrbot-feishu/src crates/astrbot-feishu/src

# Copy dashboard dist (pre-built frontend)
COPY crates/astrbot-dashboard/dashboard/dist crates/astrbot-dashboard/dashboard/dist

# Touch main.rs to force rebuild
RUN touch crates/astrbot-cli/src/main.rs

# Build release binary
RUN cargo build --release --bin astrbot

# =============================================================================
# Stage 3: Runtime
# =============================================================================
FROM debian:bookworm-slim AS runtime

LABEL maintainer="AstrBot Team <team@astrbot.rs>"
LABEL description="AstrBot - Multi-platform AI chatbot framework (Rust edition)"
LABEL version="0.1.0"

# Install runtime dependencies
RUN apt-get update && apt-get install -y \
    ca-certificates \
    libssl3 \
    && rm -rf /var/lib/apt/lists/* \
    && apt-get clean

# Create non-root user
RUN groupadd -r astrbot && useradd -r -g astrbot -m -s /bin/bash astrbot

WORKDIR /app

# Copy binary from builder
COPY --from=builder /usr/src/astrbot/target/release/astrbot /usr/local/bin/astrbot
RUN chmod +x /usr/local/bin/astrbot

# Copy dashboard dist for serving
COPY --from=builder /usr/src/astrbot/crates/astrbot-dashboard/dashboard/dist /app/dashboard/dist

# Create directories
RUN mkdir -p /app/data /app/plugins /app/logs \
    && chown -R astrbot:astrbot /app

# Write default config template
RUN cat > /app/config.json.default << 'EOF'
{
  "server_port": 6185,
  "default_provider_id": "default",
  "providers": [
    {
      "id": "default",
      "provider_type": "openai",
      "api_key": "YOUR_API_KEY_HERE",
      "base_url": "https://api.openai.com",
      "model": "gpt-4o-mini",
      "enabled": true
    }
  ],
  "plugins": {},
  "default_persona": "default",
  "personas": {}
}
EOF

USER astrbot

# Expose dashboard port
EXPOSE 6185

# Health check
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
    CMD astrbot --version || exit 1

# Default command
ENTRYPOINT ["astrbot"]
CMD ["run", "--config", "/app/config.json"]
