FROM rust:1-alpine AS builder
RUN apk add --no-cache musl-dev
WORKDIR /app
COPY Cargo.toml ./
COPY src ./src
RUN cargo build --release

FROM alpine:3.20
RUN adduser -D -u 10001 rseek
COPY --from=builder /app/target/release/rseek /usr/local/bin/rseek
USER rseek
EXPOSE 8080
ENTRYPOINT ["rseek"]
CMD ["serve", "--host", "0.0.0.0", "--port", "8080"]
