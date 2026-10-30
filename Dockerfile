# Base images; arguments used in FROM must precede the first stage
ARG NODE_VERSION=22
ARG RUST_VERSION=1.85

# Frontend Stage
FROM node:${NODE_VERSION}-bookworm-slim AS frontend
WORKDIR /app

# The web UI lands in /ui. A tree without frontend/ builds an image without
# the UI (the binary answers 404 at /); a frontend that fails to build fails
# the image.
RUN --mount=type=bind,source=.,target=/context <<EOF
set -e
mkdir -p /ui
if [ ! -f /context/frontend/package.json ]; then
    echo "No frontend/package.json: building the image without the web UI" >&2
    exit 0
fi
cp -r /context/frontend /app/frontend
cd /app/frontend
corepack enable
pnpm install --frozen-lockfile
pnpm build
cp -r .output/public/. /ui/
EOF


# Building Stage
FROM rust:${RUST_VERSION}-slim-bookworm AS build
WORKDIR /app
COPY LICENSE LICENSE

RUN --mount=type=bind,source=doc,target=doc \
    --mount=type=bind,source=src,target=src \
    --mount=type=bind,source=crates,target=crates \
    --mount=type=bind,source=Cargo.toml,target=Cargo.toml \
    --mount=type=bind,source=Cargo.lock,target=Cargo.lock \
    --mount=type=bind,from=frontend,source=/ui,target=frontend/.output/public \
    <<EOF
set -e
apt-get -y update && apt-get install -y --no-install-recommends pkg-config libssl-dev
cargo build --locked --release
cp target/release/bifrost /bifrost
EOF


# Final Stage
FROM debian:bookworm-slim AS final

COPY --from=build /bifrost /app/bifrost

RUN apt-get -y update && apt-get install -y --no-install-recommends libssl3 ca-certificates

WORKDIR /app

CMD ["/app/bifrost"]
