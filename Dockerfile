FROM rust:1.97.1-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends cmake pkg-config libopus-dev \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /build
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates libopus0 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 vozen \
    && useradd --uid 10001 --gid vozen --no-create-home --shell /usr/sbin/nologin vozen
WORKDIR /app
COPY --from=build /build/target/release/vozen-music /usr/local/bin/vozen-music
COPY catalog.json ./catalog.json
COPY media ./media
COPY licenses ./licenses
USER 10001:10001
ENTRYPOINT ["vozen-music"]
