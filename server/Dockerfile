FROM lukemathwalker/cargo-chef:latest-rust-1.86 AS chef
WORKDIR /app/server

FROM chef AS planner
COPY server/ ./
RUN cargo chef prepare --recipe-path recipe.json

FROM chef AS cacher
COPY --from=planner /app/server/recipe.json recipe.json
RUN cargo chef cook --release --recipe-path recipe.json

FROM chef AS builder
COPY server/ ./
COPY plugins/rust/nexis_wasm_plugin /app/plugins/rust/nexis_wasm_plugin
COPY examples/wasm-plugins/counter_rust_plugin /app/examples/wasm-plugins/counter_rust_plugin
COPY --from=cacher /app/server/target /app/server/target
RUN cargo build --release -p transport_ws
RUN rustup target add wasm32-unknown-unknown
RUN cargo build --manifest-path /app/examples/wasm-plugins/counter_rust_plugin/Cargo.toml --target wasm32-unknown-unknown --release

FROM debian:bookworm-slim
RUN apt-get update \
  && apt-get install -y --no-install-recommends ca-certificates netcat-openbsd \
  && rm -rf /var/lib/apt/lists/* \
  && useradd -m -u 10001 nexis
WORKDIR /app
COPY --from=builder /app/server/target/release/transport_ws /usr/local/bin/transport_ws
COPY --from=builder /app/examples/wasm-plugins/counter_rust_plugin/target/wasm32-unknown-unknown/release/nexis_counter_plugin.wasm /app/plugins/nexis_counter_plugin.wasm
USER nexis
EXPOSE 4000
CMD ["transport_ws"]
