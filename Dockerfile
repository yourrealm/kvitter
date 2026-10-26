# Forge builds this with kaniko: no BuildKit-only features (cache mounts etc.).

FROM node:24-alpine AS web
WORKDIR /src
RUN npm install -g pnpm@10.13.1
COPY package.json pnpm-lock.yaml ./
RUN pnpm install --frozen-lockfile
COPY web web
RUN mkdir -p static && pnpm build:web

FROM rust:1-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src src
COPY static static
COPY --from=web /src/static/composer.js static/composer.js
RUN cargo build --release --locked

FROM gcr.io/distroless/cc-debian12
COPY --from=build /src/target/release/kvitter /app/kvitter
ENV PORT=3000
EXPOSE 3000
ENTRYPOINT ["/app/kvitter"]
