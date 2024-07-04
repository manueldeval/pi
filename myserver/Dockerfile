FROM rust:1.79 as builder
WORKDIR /usr/src/myserver
COPY . .
RUN cargo build --release

# FROM debian:bullseye-slim
# RUN apt update
# RUN apt install -y libc6
# COPY --from=builder /usr/local/cargo/bin/myserver /usr/local/bin/myserver
# CMD ["myserver"]

FROM debian:bookworm-slim
RUN apt-get update && apt install -y openssl
WORKDIR /app
COPY --from=builder /usr/src/myserver/target/release/myserver /app/

CMD ["./myserver"]