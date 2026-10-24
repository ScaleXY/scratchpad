# Changelog

All notable changes to Scratchpad are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/). Release notes on GitHub are taken from this file.

## [Unreleased]

## [0.1.0] - 2026-10-17

### Added

- Local SMTP server that captures every email (`AUTH PLAIN/LOGIN`, `PIPELINING`, `8BITMIME`,
  `SMTPUTF8`, `SIZE`, optional `STARTTLS`); unlimited mailboxes named by the SMTP username.
- Desktop app (iced) and web interface with a black (`#000000`) dark theme and a light theme.
- HTML, text, HTML source, raw source, headers and attachments, including inline `cid:` images.
- Multi-device previews: Fit, Desktop, Laptop, Tablet, Mobile and all devices side by side.
- Link checker, spam report (SpamAssassin `spamd` with built-in fallback) and Laravel view
  inspector.
- Forwarding through any SMTP server.
- Export of one or more emails as a single self-contained `.html` file.
- Sharing: uploads to S3-compatible storage (public links or expiring presigned links) and live
  links served by Scratchpad, including live inbox links.
- Credential vault: S3 credentials stored on the device or encrypted with a master password
  (Argon2id, XChaCha20-Poly1305) for cloud sync (sync backend stubbed for now).
- Remote mailboxes: pull from IMAP accounts or other Scratchpad instances.
- Headless mode for servers and containers, with HTTP basic auth, a health check
  (`--healthcheck`) and JSON logs (`--log-format json`).
- Signed builds for Windows (x86, x64, arm64), macOS (x64, arm64) and Linux (x86, x64, arm64),
  a multi-arch container image, Microsoft Store packages (MSIX) and a sandboxed Mac App Store
  package.

[Unreleased]: https://github.com/scalexy/scratchpad/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/scalexy/scratchpad/releases/tag/v0.1.0
