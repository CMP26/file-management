FROM node:22-bookworm-slim AS frontend-builder

WORKDIR /app/frontend
COPY frontend/package.json frontend/package-lock.json ./
RUN npm ci
COPY frontend/index.html frontend/vite.config.js ./
COPY frontend/src ./src
RUN npm run build

FROM rust:1.91-bookworm AS builder

RUN apt-get update && apt-get install -y --no-install-recommends \
        ffmpeg \
        pkg-config \
        libssl-dev \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY Cargo.toml ./
COPY Cargo.lock ./
COPY frontend ./frontend
COPY --from=frontend-builder /app/frontend/dist ./frontend/dist
COPY migrations ./migrations
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y --no-install-recommends \
        ffmpeg \
        curl \
        ca-certificates \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app
COPY --from=builder /app/target/release/nexalearn-backend /usr/local/bin/nexalearn-backend
EXPOSE 8080
CMD ["nexalearn-backend"]
