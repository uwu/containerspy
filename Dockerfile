# muslrust applications don't link against libc, they contain musl, NEAT!
FROM clux/muslrust:stable AS chef
USER root
RUN cargo install cargo-chef
WORKDIR /build

# use chef to plan the most efficent way to build the project from our files
FROM chef AS planner
COPY . .
RUN cargo chef prepare --recipe-path recipe.json

# use chef to build the deps
FROM chef AS builder
COPY --from=planner /build/recipe.json recipe.json

# build deps (cached by docker)
RUN cargo chef cook --release --target x86_64-unknown-linux-musl --recipe-path recipe.json

# build the application
COPY . .
RUN cargo build --release --target x86_64-unknown-linux-musl

FROM gcr.io/distroless/static

COPY --from=builder /build/target/x86_64-unknown-linux-musl/release/containerspy /usr/bin/containerspy

ENTRYPOINT ["containerspy"]
STOPSIGNAL SIGINT
