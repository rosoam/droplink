FROM rust:1.78-slim AS builder
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
RUN mkdir src && echo 'fn main(){}' > src/main.rs && cargo build --release --locked
COPY src ./src
RUN touch src/main.rs && cargo build --release --locked

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
RUN useradd -r -s /bin/false relay
COPY --from=builder /app/target/release/droplink-relay /usr/local/bin/droplink-relay
ENV PORT=8080
EXPOSE 8080
USER relay
CMD ["droplink-relay"]
