# syntax=docker/dockerfile:1.7

FROM node:24-bookworm-slim AS frontend
WORKDIR /app/frontend

RUN corepack enable

COPY frontend/package.json frontend/pnpm-lock.yaml ./
RUN --mount=type=cache,id=pnpm-store,target=/root/.local/share/pnpm/store \
    pnpm install --frozen-lockfile

COPY frontend ./
RUN pnpm build

FROM rust:1-bookworm AS backend
WORKDIR /app

RUN apt-get update && \
    apt-get install -y --no-install-recommends libsqlite3-dev pkg-config && \
    rm -rf /var/lib/apt/lists/*

COPY Cargo.toml Cargo.lock ./
COPY migrations ./migrations
COPY src ./src
COPY --from=frontend /app/frontend/dist ./frontend/dist

RUN --mount=type=cache,id=cargo-registry,target=/usr/local/cargo/registry \
    --mount=type=cache,id=cargo-git,target=/usr/local/cargo/git \
    --mount=type=cache,id=tokenaltar-target,target=/app/target \
    cargo build --release && \
    cp /app/target/release/tokenaltar /usr/local/bin/tokenaltar

FROM debian:bookworm-slim AS runtime

RUN apt-get update && \
    apt-get install -y --no-install-recommends ca-certificates libsqlite3-0 && \
    rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=backend /usr/local/bin/tokenaltar /usr/local/bin/tokenaltar

ENV TOKENALTAR_BIND=0.0.0.0:8080
ENV TOKENALTAR_DATABASE_URL=sqlite:///data/tokenaltar.sqlite3

VOLUME ["/data"]
EXPOSE 8080

CMD ["tokenaltar"]
