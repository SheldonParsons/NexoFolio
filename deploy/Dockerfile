ARG RUST_IMAGE=rust:1.94.0-bookworm
ARG RUNTIME_IMAGE=debian:bookworm-slim
FROM ${RUST_IMAGE} AS build
ENV CARGO_HTTP_MULTIPLEXING=false CARGO_HTTP_TIMEOUT=30
WORKDIR /app
COPY Cargo.toml Cargo.lock rust-toolchain.toml ./
COPY apps ./apps
COPY crates ./crates
COPY migrations ./migrations
COPY tests ./tests
COPY contracts ./contracts
RUN cargo build --locked --release -p nexofolio-backend --bins

FROM ${RUNTIME_IMAGE} AS runtime
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates curl libgcc-s1 \
    && rm -rf /var/lib/apt/lists/* \
    && groupadd --gid 10001 nexofolio \
    && useradd --uid 10001 --gid 10001 --no-create-home nexofolio \
    && mkdir -p /var/lib/nexofolio/blobs \
    && chown -R 10001:10001 /var/lib/nexofolio
COPY --from=build /app/target/release/nexofolio-api /usr/local/bin/
COPY --from=build /app/target/release/nexofolio-worker /usr/local/bin/
COPY --from=build /app/target/release/nexofolio-admin /usr/local/bin/
USER 10001:10001
ENV NEXOFOLIO_BIND_ADDR=0.0.0.0:8080 NEXOFOLIO_BLOB_ROOT=/var/lib/nexofolio/blobs
EXPOSE 8080
CMD ["nexofolio-api"]
