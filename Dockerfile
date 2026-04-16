# ── Stage 1: Build ────────────────────────────────────────────────────────────
# Full Rust toolchain. Gets thrown away after compilation.
FROM rust:1.86-bookworm AS builder

WORKDIR /app

RUN apt-get update && \
    apt-get install -y --no-install-recommends sqlite3 && \
    rm -rf /var/lib/apt/lists/*

# Layer cache trick: copy only dependency manifests and build a dummy binary
# first. Docker caches this layer. On future deployments, if Cargo.toml/lock
# haven't changed, this expensive step is skipped entirely.
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main() {}' > src/main.rs && \
    cargo build --release && \
    rm -f target/release/deps/serene_reactions*

# Now copy real source and migrations
COPY src ./src
COPY migrations ./migrations

# sqlx::query! macros verify SQL at compile time and need a real database.
# We create a temporary one here just for the build — it is NOT the production DB.
RUN mkdir -p /tmp/sqlx-check && \
    sqlite3 /tmp/sqlx-check/reactions.db < migrations/20260416112313_init.sql

ENV DATABASE_URL=sqlite:///tmp/sqlx-check/reactions.db

RUN cargo build --release


# ── Stage 2: Runtime ──────────────────────────────────────────────────────────
# Minimal image — just Debian + SQLite library + your binary.
# No compiler, no source code, no Rust toolchain.
FROM debian:bookworm-slim

RUN apt-get update && \
    apt-get install -y --no-install-recommends libsqlite3-0 ca-certificates && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /app/target/release/serene-reactions ./

# Create the data directory. In Dokploy you mount a persistent volume here
# (/app/data) so the SQLite file survives redeployments.
RUN mkdir -p data

ENV DATABASE_URL=sqlite:///app/data/reactions.db

EXPOSE 6969

CMD ["./serene-reactions"]
