FROM rust:1.97-bookworm@sha256:0e2bcaef56d041a486784e54104a81aebe0da44bd03019bd70bc0401e42e4a97 AS builder

WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY src ./src
RUN cargo build --release --locked

FROM debian:bookworm-slim@sha256:abd67ffcfa541b485a3dff59865ab629aa048a6c613e639d36e7456b0b229241

RUN groupadd --system --gid 10001 beacon \
    && useradd --system --uid 10001 --gid beacon --home-dir /nonexistent \
       --shell /usr/sbin/nologin beacon

COPY --from=builder /src/target/release/beacon /usr/local/bin/beacon

USER beacon:beacon
ENTRYPOINT ["/usr/local/bin/beacon"]
