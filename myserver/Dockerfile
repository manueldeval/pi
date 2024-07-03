FROM rust:1.79 as builder
WORKDIR /usr/src/myserver
COPY . .
RUN cargo install --path .

FROM debian:bullseye-slim
COPY --from=builder /usr/local/cargo/bin/myserver /usr/local/bin/myserver
CMD ["myserver"]