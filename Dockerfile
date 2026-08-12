FROM rust:1.97-bookworm AS builder

WORKDIR /app
COPY . .
RUN cargo build --release -p lungyam-cli

FROM debian:bookworm-slim

RUN useradd --system --uid 10001 --create-home lungyam
COPY --from=builder /app/target/release/lungyam /usr/local/bin/lungyam
COPY config/lungyam.yaml /etc/lungyam/lungyam.yaml

USER lungyam
EXPOSE 8080

ENTRYPOINT ["lungyam"]
CMD ["--config", "/etc/lungyam/lungyam.yaml"]
