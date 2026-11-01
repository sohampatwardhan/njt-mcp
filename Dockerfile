# Multi-arch image. Build with:
#   docker buildx build --platform linux/amd64,linux/arm64 -t njt-mcp .
# A plain `docker build` produces the architecture of the machine it runs on.

FROM rust:1.99-bookworm AS build
RUN apt-get update \
    && apt-get install -y --no-install-recommends protobuf-compiler \
    && rm -rf /var/lib/apt/lists/*
WORKDIR /src
COPY Cargo.toml Cargo.lock build.rs ./
COPY proto ./proto
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates \
    && rm -rf /var/lib/apt/lists/* \
    && useradd --uid 10001 --create-home njt
COPY --from=build /src/target/release/njt-mcp /usr/local/bin/njt-mcp
USER 10001
EXPOSE 8092
ENTRYPOINT ["njt-mcp"]
CMD ["http", "--bind", "0.0.0.0:8092"]
