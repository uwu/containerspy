FROM rust:1.86-slim AS build-env

WORKDIR /build

# since this is just a build env, simply copy everything in, no need to be picky
COPY . .

# this builds a release binary and leaves the binary in /usr/local/cargo/bin/myapp
RUN cargo install --path .

FROM alpine:3.21

COPY --from=build-env /usr/local/cargo/bin/containerspy /usr/bin/containerspy

# for mounting config.json into
RUN mkdir /etc/containerspy

ENTRYPOINT ["containerspy"]