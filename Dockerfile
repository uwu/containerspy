# muslrust applications don't link against libc, they contain musl, NEAT!
FROM clux/muslrust:stable AS chef
USER root

# needed for cross compilation from x86 to arm
#ARG BUILD_PLATFORM="x86_64" # set to "aarch64" for arm builds
#RUN rustup target add ${BUILD_PLATFORM}-unknown-linux-musl

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
ARG BUILD_PLATFORM="x86_64"
RUN cargo chef cook --release --target ${BUILD_PLATFORM}-unknown-linux-musl --recipe-path recipe.json

# build the application
COPY . .
RUN cargo build --release --target ${BUILD_PLATFORM}-unknown-linux-musl

FROM gcr.io/distroless/static

ARG BUILD_PLATFORM="x86_64"
COPY --from=builder /build/target/${BUILD_PLATFORM}-unknown-linux-musl/release/containerspy /usr/bin/containerspy

ENTRYPOINT ["containerspy"]
STOPSIGNAL SIGINT
