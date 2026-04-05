FROM rust:1.85 AS builder
WORKDIR /app
COPY . .
RUN cargo build --release -p chorus-server

FROM debian:bookworm-slim
RUN apt-get update && apt-get install -y ca-certificates && rm -rf /var/lib/apt/lists/*
COPY --from=builder /app/target/release/chorus-server /usr/local/bin/
EXPOSE 3000
CMD ["chorus-server"]
