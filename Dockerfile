# Headless Scratchpad (SMTP + web interface) for servers and CI.
#   docker build -t scratchpad .
#   docker run -p 2525:2525 -p 8025:8025 -e SCRATCHPAD_WEB_AUTH=admin:secret scratchpad
# Keep the Rust version at or above workspace.package.rust-version.
FROM rust:1.98-slim-bookworm AS build
WORKDIR /src
# .git isn't copied, so the version comes from here (the release workflow sets it from the tag).
ARG SCRATCHPAD_VERSION=0.0.0
COPY . .
RUN cargo build --release --locked -p scratchpad --no-default-features \
 && strip target/release/scratchpad

FROM debian:bookworm-slim
LABEL org.opencontainers.image.title="Scratchpad" \
      org.opencontainers.image.description="Email testing: SMTP capture server and web interface (headless)" \
      org.opencontainers.image.source="https://github.com/scalexy/scratchpad" \
      org.opencontainers.image.licenses="MIT"
RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --home /data --uid 10001 scratchpad \
 && mkdir /data && chown scratchpad /data
COPY --from=build /src/target/release/scratchpad /usr/local/bin/scratchpad
USER scratchpad
ENV SCRATCHPAD_HEADLESS=true \
    SCRATCHPAD_DATA_DIR=/data \
    SCRATCHPAD_SMTP_HOST=0.0.0.0 \
    SCRATCHPAD_WEB_HOST=0.0.0.0
VOLUME /data
EXPOSE 2525 8025
HEALTHCHECK --interval=30s --timeout=5s --start-period=10s CMD ["scratchpad", "--healthcheck"]
ENTRYPOINT ["scratchpad", "--headless"]
