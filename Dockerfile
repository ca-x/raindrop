# syntax=docker/dockerfile:1.7

FROM node:26.4.0-bookworm-slim AS web-builder

WORKDIR /src/web
ENV NPM_CONFIG_REGISTRY=https://registry.npmjs.org/ \
    NPM_CONFIG_REPLACE_REGISTRY_HOST=always
COPY web/package.json web/package-lock.json ./
RUN npm install --global npm@12.0.1 --ignore-scripts
RUN --mount=type=cache,target=/root/.npm,sharing=locked \
    npm ci --ignore-scripts
COPY web/ ./
RUN npm run build

FROM rust:1.94.0-bookworm AS rust-builder

WORKDIR /src
COPY . ./
COPY --from=web-builder /src/web/dist /src/web/dist
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/src/target,sharing=locked \
    cargo build --release --locked && \
    cp target/release/raindrop /tmp/raindrop

FROM debian:bookworm-slim AS runtime

ARG VERSION=dev
ARG BUILD_TIME=unknown
ARG GIT_COMMIT=unknown

LABEL org.opencontainers.image.title="Raindrop" \
      org.opencontainers.image.description="A self-hosted, multi-user RSS reader" \
      org.opencontainers.image.source="https://github.com/czyt/raindrop" \
      org.opencontainers.image.licenses="MIT" \
      org.opencontainers.image.version="${VERSION}" \
      org.opencontainers.image.created="${BUILD_TIME}" \
      org.opencontainers.image.revision="${GIT_COMMIT}"

RUN apt-get update && \
    apt-get install --yes --no-install-recommends ca-certificates curl && \
    rm -rf /var/lib/apt/lists/* && \
    groupadd --gid 10001 raindrop && \
    useradd --uid 10001 --gid 10001 --no-create-home \
      --home-dir /nonexistent --shell /usr/sbin/nologin raindrop && \
    install -d -o 10001 -g 10001 -m 0700 /data

COPY --from=rust-builder --chown=10001:10001 /tmp/raindrop /usr/local/bin/raindrop

ENV RAINDROP_DATA_DIR=/data \
    RAINDROP_BIND=0.0.0.0:8080

WORKDIR /
VOLUME ["/data"]
EXPOSE 8080
STOPSIGNAL SIGTERM
USER 10001:10001
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s --retries=3 \
  CMD ["curl", "--fail", "--silent", "--show-error", "http://127.0.0.1:8080/api/v1/health/live"]
ENTRYPOINT ["/usr/local/bin/raindrop"]
