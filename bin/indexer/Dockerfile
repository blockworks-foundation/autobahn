FROM rust:1.76.0 as base
RUN cargo install cargo-chef@0.1.62 --locked
RUN rustup component add rustfmt
RUN apt-get update && apt-get install -y clang cmake ssh
WORKDIR /app

FROM base AS plan
COPY . .
WORKDIR /app
RUN cargo chef prepare --recipe-path recipe.json

FROM base as build
COPY --from=plan /app/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json
COPY . .
RUN cargo build --release --bin indexer

FROM debian:bookworm-slim as run
RUN apt-get update && apt-get -y install ca-certificates libc6 libssl3 libssl-dev openssl

COPY --from=build /app/target/release/indexer /usr/local/bin/
COPY --from=build /app/bin/indexer/template-config.toml /usr/local/bin/template-config.toml

RUN adduser --system --group --no-create-home mangouser
USER mangouser
