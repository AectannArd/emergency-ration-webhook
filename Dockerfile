# Multi-stage build for the capacity admission webhook (referenced by
# deploy/kustomize/webhook/deployment.yaml). Produces a small static image on a distroless base.
# 4-space indent per .editorconfig.

# ---- Build stage ----
FROM rust:1.89-bookworm AS builder
WORKDIR /usr/src/capacity-admission-webhook

# Build dependencies first (cached layer). Stub ALL [[bin]] entry points so the
# dependency graph compiles without the real sources: the webhook (src/main.rs),
# erw-verify (src/bin/erw-verify/main.rs) AND the equalizer
# (src/bin/capacity-equalizer/main.rs) — every [[bin]] in Cargo.toml must exist
# for `cargo build --release` to succeed (CI failure catalog Layer 9).
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src/bin/erw-verify src/bin/capacity-equalizer && \
    echo "fn main() {}" > src/main.rs && \
    echo "" > src/lib.rs && \
    echo "fn main() {}" > src/bin/erw-verify/main.rs && \
    echo "fn main() {}" > src/bin/erw-verify/build.rs && \
    echo "fn main() {}" > src/bin/capacity-equalizer/main.rs && \
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
