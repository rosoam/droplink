FROM rust:1.92-slim AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
RUN cargo build -p droplink-relay --release

FROM debian:bookworm-slim
RUN useradd --system --uid 10001 droplink
COPY --from=builder /app/target/release/droplink-relay /usr/local/bin/droplink-relay
USER droplink
EXPOSE 8080
CMD ["droplink-relay"]
