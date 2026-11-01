FROM rust:1.97.1-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake pkg-config libopus-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
COPY web ./web
COPY assets/brand/v4/vozen-music-avatar.png ./assets/brand/v4/vozen-music-avatar.png
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libopus0 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 vozen \
    && useradd --uid 10001 --gid vozen --no-create-home --shell /usr/sbin/nologin vozen
WORKDIR /app
RUN mkdir -p /app/data && chown 10001:10001 /app/data
COPY --from=build /build/target/release/vozen-music /usr/local/bin/vozen-music
COPY catalog.json ./catalog.json
COPY media ./media
COPY licenses ./licenses
COPY covers ./covers
USER 10001:10001
ENTRYPOINT ["vozen-music"]
