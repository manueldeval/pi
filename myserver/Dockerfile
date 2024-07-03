FROM rust:1.79 as builder
WORKDIR /usr/src/myserver
COPY . .
RUN cargo install --path .

FROM debian:bullseye-slim
RUN apt update
RUN apt install -y libc6
COPY --from=builder /usr/local/cargo/bin/myserver /usr/local/bin/myserver
CMD ["myserver"]