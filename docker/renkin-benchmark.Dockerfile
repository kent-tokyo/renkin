# Linux bounded-runtime image for the formal RENKIN comparison arm.
# Pin the built image digest in each formal run manifest before publication.
FROM rust:1.89-bookworm AS build
WORKDIR /src
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY src ./src
# Cargo validates explicit example paths while parsing the manifest, even when
# building only the CLI binary. Keep these sources in the build context so the
# bounded comparison image can be reproduced from a clean checkout.
COPY examples ./examples
RUN cargo build --locked --release --bin renkin

FROM debian:bookworm-slim
ARG VCS_REF=unknown
LABEL org.opencontainers.image.revision=$VCS_REF
COPY --from=build /src/target/release/renkin /usr/local/bin/renkin
ENTRYPOINT ["renkin"]
