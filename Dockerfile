FROM rust:1.86-alpine3.21 AS build-env

WORKDIR /build

# the rust container is literally incomplete lol
# https://stackoverflow.com/a/74309414
RUN apk add --no-cache pcc-libs-dev musl-dev pkgconf

# for layer caching, first only build the deps, so that changes to literally anything else don't invalidate the cache
RUN mkdir src
RUN echo 'fn main() {}' > src/main.rs
COPY Cargo.toml Cargo.lock ./
RUN cargo build --release

# copy in the real source
RUN rm src/*.rs
COPY src src
COPY build.rs ./

# this builds a release binary
RUN cargo build --release

FROM alpine:3.21

COPY --from=build-env /build/target/release/containerspy /usr/bin/containerspy

# for mounting config.json into
RUN mkdir /etc/containerspy

ENTRYPOINT ["containerspy"]