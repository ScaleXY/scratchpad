# Headless Scratchpad (SMTP + web interface) for servers and CI.
#   docker build -t scratchpad .
#   docker run -p 2525:2525 -p 8025:8025 -e SCRATCHPAD_WEB_AUTH=admin:secret scratchpad
# Multi-arch (amd64, arm64, 386): the binary is cross-compiled on the build machine's
# architecture instead of compiling under emulation.
#   docker buildx build --platform linux/amd64,linux/arm64,linux/386 .
# Keep the Rust version at or above workspace.package.rust-version.
FROM --platform=$BUILDPLATFORM rust:1.98-slim-bookworm AS build
ARG TARGETARCH
WORKDIR /src
RUN case "$TARGETARCH" in \
      amd64) target=x86_64-unknown-linux-gnu; prefix=x86_64-linux-gnu; packages="gcc-x86-64-linux-gnu libc6-dev-amd64-cross" ;; \
      arm64) target=aarch64-unknown-linux-gnu; prefix=aarch64-linux-gnu; packages="gcc-aarch64-linux-gnu libc6-dev-arm64-cross" ;; \
      386) target=i686-unknown-linux-gnu; prefix=i686-linux-gnu; packages="gcc-i686-linux-gnu libc6-dev-i386-cross" ;; \
      *) echo "unsupported architecture $TARGETARCH" >&2; exit 1 ;; \
    esac \
 && if ! command -v "$prefix-gcc" >/dev/null; then \
      apt-get update && apt-get install -y --no-install-recommends $packages \
      && rm -rf /var/lib/apt/lists/*; \
    fi \
 && rustup target add "$target" \
 && echo "$target" > /target && echo "$prefix" > /prefix
# .git isn't copied, so the version comes from here (the release workflow sets it from the tag).
ARG SCRATCHPAD_VERSION=0.0.0
COPY . .
RUN target="$(cat /target)" prefix="$(cat /prefix)" \
 && upper="$(echo "$target" | tr a-z- A-Z_)" \
 && export "CARGO_TARGET_${upper}_LINKER=$prefix-gcc" \
 && cargo build --release --locked --target "$target" -p scratchpad --no-default-features \
 && "$prefix-strip" "target/$target/release/scratchpad" \
 && cp "target/$target/release/scratchpad" /scratchpad

FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="Scratchpad" \
      org.opencontainers.image.description="Email testing: SMTP capture server and web interface (headless)" \
      org.opencontainers.image.source="https://github.com/ScaleXY/scratchpad" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home /data --uid 10001 scratchpad \
 && mkdir /data && chown scratchpad /data
COPY --from=build /scratchpad /usr/local/bin/scratchpad
USER scratchpad
ENV SCRATCHPAD_HEADLESS=true \
    SCRATCHPAD_DATA_DIR=/data \
    SCRATCHPAD_SMTP_HOST=0.0.0.0 \
    SCRATCHPAD_WEB_HOST=0.0.0.0
VOLUME /data
EXPOSE 2525 8025
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["scratchpad", "--healthcheck"]
ENTRYPOINT ["scratchpad", "--headless"]
