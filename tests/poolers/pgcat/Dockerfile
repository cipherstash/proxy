# syntax=docker/dockerfile:1
# Official v1.2.0 source and lockfile. Build locally because the release's GHCR
# image is not publicly pullable; do not substitute a newer PgCat version.
FROM rust:1.79.0-slim-bookworm AS builder
RUN apt-get update && apt-get install -y --no-install-recommends build-essential ca-certificates \
    && rm -rf /var/lib/apt/lists/*
ADD --checksum=sha256:c742dbc232d232b6af4ddc0eac51578c9903d9effd0ffb75e86e7e011a76a42b \
    https://codeload.github.com/postgresml/pgcat/tar.gz/4a7a6a8e7a78354b889002a4db118a8e2f2d6d79 /tmp/pgcat.tar.gz
WORKDIR /app
RUN tar -xzf /tmp/pgcat.tar.gz --strip-components=1 -C /app \
    && rm /tmp/pgcat.tar.gz \
    && cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y --no-install-recommends postgresql-client \
    && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/pgcat /usr/bin/pgcat
ENV RUST_LOG=info
CMD ["pgcat"]
STOPSIGNAL SIGINT
