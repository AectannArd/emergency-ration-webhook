# Multi-stage build for the capacity admission webhook (referenced by
# deploy/deployment.yaml). Produces a small static image on a distroless base.
# 4-space indent per .editorconfig.

# ---- Build stage ----
FROM rust:1.89-bookworm AS builder
WORKDIR /usr/src/capacity-admission-webhook

# Build dependencies first (cached layer).
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/bin/erw-verify && \
    echo "fn main() {}" > src/main.rs && \
    echo "" > src/lib.rs && \
    echo "fn main() {}" > src/bin/erw-verify/main.rs && \
    cargo build --release && \
    rm -rf src

# Build the actual binary.
COPY . .
RUN touch src/main.rs src/lib.rs && cargo build --release --bin capacity-admission-webhook

# ---- Runtime stage ----
FROM gcr.io/distroless/cc-debian12:nonroot
COPY --from=builder \
    /usr/src/capacity-admission-webhook/target/release/capacity-admission-webhook \
    /usr/local/bin/capacity-admission-webhook

# HTTPS admission (8443) + plaintext metrics/probe (9090).
EXPOSE 8443 9090
USER nonroot:nonroot
ENTRYPOINT ["/usr/local/bin/capacity-admission-webhook"]
