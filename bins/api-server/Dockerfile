# syntax=docker/dockerfile:1.7
# Build context: the repository root.

FROM rust:1.100.0-slim-trixie@sha256:e9d95b74296f7126a83ae3d041aeaba1ff4c478f13abc5cc034fcf7c131c3d4a AS build
WORKDIR /src
COPY . .
RUN cargo build --release --locked -p api-server && cp target/release/api-server /service

FROM gcr.io/distroless/cc-debian13:nonroot@sha256:cb87acc574abe719b6e5c547384defa2f7b1f24af9605d3f618cf852495d8be2
COPY --from=build /service /service
EXPOSE 8080
ENTRYPOINT ["/service"]
