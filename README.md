# Scratchpad

Email testing and debugging for developers, by [ScaleXY](https://scalexy.com). Your applications
send email to Scratchpad's local SMTP server instead of real inboxes. You then inspect the captured
messages in a native desktop app (built with [iced](https://iced.rs)) or in a web interface.
Scratchpad can also run **headless** on a server and be used entirely from the browser.

```
scratchpad                 # desktop app (+ web interface on http://127.0.0.1:8025)
scratchpad --headless      # SMTP server + web interface only, for servers and containers
```

## Features

| Feature | Details |
| --- | --- |
| Local SMTP server that captures all mail | ✅ port `2525` by default, `AUTH PLAIN/LOGIN`, `PIPELINING`, `8BITMIME`, `SMTPUTF8`, `SIZE`, optional `STARTTLS` |
| Unlimited mailboxes, created automatically from the SMTP username | ✅ any password is accepted; mail without credentials goes to **Default** |
| Sender, recipients, CC and BCC | ✅ BCC is derived from SMTP envelope recipients missing from To/Cc |
| HTML, plain text, HTML source, raw message and headers | ✅ sandboxed HTML preview with **multi-device previews** (see below) |
| Attachments, including inline `cid:` images | ✅ open or save; inline images render in the HTML preview |
| Link checker (HEAD, falling back to GET), can be turned off | ✅ |
| SpamAssassin spam report, can be turned off | ✅ via `spamd` (`127.0.0.1:783`); built-in heuristics when SpamAssassin is not installed |
| Laravel view inspector (`beyondcode/helo-laravel`) | ✅ decodes the `X-HELO-View`, `X-HELO-View-File` and `X-HELO-View-Data` headers |
| Forward emails via a custom SMTP server | ✅ none / STARTTLS / TLS, with optional sender override |
| Share single emails or whole inboxes, list and revoke shares | ✅ **cloud shares** uploaded to your own S3-compatible bucket, or live links served by your Scratchpad web server (see below) |
| Remote mailboxes for live and staging apps | ✅ poll an **IMAP account**, or another **Scratchpad instance** running headless on your server |
| Desktop notifications for new mail | ✅ desktop app and web interface |
| Keyboard navigation | ✅ `↑`/`↓` (or `j`/`k`) to move, `Backspace`/`Delete` to delete, `1`–`9` to switch tabs (desktop) |
| Bulk delete emails and whole inboxes | ✅ multi-select, empty mailbox, delete mailbox |
| Search | ✅ subject, sender, recipients and preview |
| Export | ✅ any email (or a selection) as **one self-contained `.html` file** with the viewer built in |

Good to know:

* **Sharing uses your own storage.** Cloud shares go to an S3-compatible bucket you configure;
  live links point at your own web interface (set *Public URL* when Scratchpad runs behind a domain).
* **The desktop app's preview is a text rendering.** iced has no HTML engine. *Open in browser*
  shows the real HTML with all device previews from an exported file, so it works offline.
* **The view inspector reads the headers of the `beyondcode/helo-laravel` package.** Install it in
  your Laravel app (`composer require --dev beyondcode/helo-laravel`) to see the view behind each
  mailable.

## Design

Both UIs share one design system: a neutral light theme and a **pure black (`#000000`) dark
theme**, the [Inter](https://rsms.me/inter/) typeface and [Lucide](https://lucide.dev) icons.
Both are bundled, so the app looks the same on every platform. Choose *System*, *Light* or *Dark*
from the switcher at the bottom of the sidebar. The web UI stores its choice per browser.

To render screenshots of the desktop UI without a display (useful for docs and visual review):

```sh
cargo test -p scratchpad -- --ignored gui_snapshots   # writes target/gui-snapshots/*.png
```

## Multi-device previews

The preview tab in the web interface, the desktop app and exported files has a device switcher:
**Fit** (full width), **Desktop** 1280×800, **Laptop** 1024×640, **Tablet** 768×1024, **Mobile**
390×844 and **All** (mobile, tablet and desktop side by side). Tablet and mobile rotate to
landscape. Each device renders at its real viewport size and is scaled down to fit, so line breaks
and media queries behave as they would on the device.

## Export and cloud sharing

**Export.** *Export .html* (toolbar, or *Export* on a selection) produces one self-contained file:
the read-only viewer UI (both themes, device previews, text, HTML source, headers, attachments,
links, spam report, view inspector, raw source), fonts, icons, inline images and attachments, all
embedded. The email itself is rendered in a sandboxed frame without scripts. Attachments over
10 MB (25 MB in total) are listed but not embedded. Leave out *technical details* to drop headers,
the raw source, Bcc and the reports.

**Cloud shares.** Add a bucket under *Cloud sharing* (AWS S3, Cloudflare R2, MinIO, Backblaze B2,
Wasabi, …):

| Field | Notes |
| --- | --- |
| S3 endpoint | e.g. `https://s3.eu-west-1.amazonaws.com`, `https://<account>.r2.cloudflarestorage.com` |
| Region, Bucket | region defaults to `auto` |
| Access key, Secret key | the secret is never sent back to the UI |
| Vanity/CDN domain | optional; public links use it (CloudFront, Cloudflare, …) |
| Links | **Public**, **Private** (presigned, expiring) or **Both** (chosen per share) |
| Path-style URLs | on by default; turn off for AWS virtual-hosted buckets |

Sharing exports the selected emails and uploads the file:

* public: `<cdn-or-endpoint>/email/shares/<uuid>.html`
* private: `email/private/<uuid>.html`, opened through a presigned link that expires after 1 hour,
  1 day or 7 days (the SigV4 maximum). *New link* issues a fresh presigned link without uploading
  again.

Private shares use a separate prefix so a bucket policy (or CDN rule) can make `email/shares/`
public while everything else stays private. *Test connection* uploads, opens and deletes test
objects, and warns when private objects are publicly readable. The list of shares (title, emails,
bucket, link, expiry, size) is stored locally in `scratchpad.sqlite3`; deleting a share deletes the
uploaded file. Live links from the Scratchpad server are still available as a destination.

**Credentials.** Each S3 config is stored either

* **on this device only**: `credentials.json` in the data directory (owner-only permissions), works
  fully offline; or
* **synced**: in a vault encrypted with a master password (Argon2id key derivation,
  XChaCha20-Poly1305) and synced to the cloud backend. Only the encrypted vault leaves the device.
  Unlock it once per session; headless servers can pass `--master-password` /
  `SCRATCHPAD_MASTER_PASSWORD`.

The cloud backend is a stub for now: it mirrors the encrypted vault to `cloud-sync-stub/` in the
data directory, so *Sync now* and *Restore from cloud* can already be exercised. The real service
plugs in by implementing `scratchpad_core::vault::SyncBackend`.

## Pointing your app at Scratchpad

| Setting | Value |
| --- | --- |
| Host | `127.0.0.1` (`host.docker.internal` from Docker, `10.0.2.2` from Vagrant/Homestead) |
| Port | `2525` |
| Username | the mailbox name, e.g. `My-Project` (optional) |
| Password | anything |
| Encryption | none |

Laravel `.env`:

```dotenv
MAIL_MAILER=smtp
MAIL_HOST=127.0.0.1
MAIL_PORT=2525
MAIL_USERNAME=Inbox-Name
MAIL_PASSWORD=null
MAIL_ENCRYPTION=null
```

The *Setup* page in both UIs has ready-to-copy snippets for Laravel, Symfony, WordPress, Nodemailer,
Rails and Django, plus a **Send test email** button.

## Running headless (servers, CI, containers)

```sh
scratchpad --headless \
  --smtp-host 0.0.0.0 --smtp-port 2525 \
  --web-host 0.0.0.0 --web-port 8025 \
  --web-auth admin:change-me \
  --public-url https://scratchpad.example.com
```

Every flag can also be set through an environment variable:

| Flag | Environment | Default |
| --- | --- | --- |
| `--headless` | `SCRATCHPAD_HEADLESS` | off |
| `--data-dir` | `SCRATCHPAD_DATA_DIR` | OS data dir (`~/Library/Application Support/com.scalexy.scratchpad`, `~/.local/share/scratchpad`, `%APPDATA%\ScaleXY\Scratchpad\data`) |
| `--smtp-host` / `--smtp-port` | `SCRATCHPAD_SMTP_HOST` / `SCRATCHPAD_SMTP_PORT` | `127.0.0.1` / `2525` |
| `--web-host` / `--web-port` | `SCRATCHPAD_WEB_HOST` / `SCRATCHPAD_WEB_PORT` | `127.0.0.1` / `8025` |
| `--web-auth user:pass` | `SCRATCHPAD_WEB_AUTH` | none (a warning is printed when listening publicly without it) |
| `--public-url` | `SCRATCHPAD_PUBLIC_URL` | the web server's URL |
| `--master-password` | `SCRATCHPAD_MASTER_PASSWORD` | none (synced S3 configs stay locked until unlocked in the UI) |
| `--log` | `SCRATCHPAD_LOG` | `info` |
| `--log-format text\|json` | `SCRATCHPAD_LOG_FORMAT` | `text` (logs go to stderr; the desktop app also writes `logs/scratchpad.log` in the data directory) |
| `--healthcheck` | | exits 0 when the web interface of a running instance answers (used by the container image) |

Flags override the saved settings for that run only. Everything else, including forwarding, checks
and retention, is set in the *Settings* page and stored in `settings.json` in the data directory.
Emails live in `scratchpad.sqlite3` in the same directory.

**Docker:**

```sh
docker run -d -p 2525:2525 -p 8025:8025 -v scratchpad-data:/data \
  -e SCRATCHPAD_WEB_AUTH=admin:change-me ghcr.io/scalexy/scratchpad
```

**systemd (Ubuntu `.deb`):** the package installs a disabled `scratchpad` service that runs headless as
the `scratchpad` user. Configure it in `/etc/default/scratchpad`, then run
`sudo systemctl enable --now scratchpad`.

**Testing staging servers from your desktop:** run Scratchpad headless next to your staging app. In
the desktop app, add a *Remote mailbox* of type *Scratchpad server* that points at its URL. New mail
then shows up locally.

### Web API

The web interface uses a JSON API under `/api`, which you can also script against (for example in
end-to-end tests):

| Method & path | Purpose |
| --- | --- |
| `GET /api/status` | SMTP/web status |
| `GET /api/mailboxes` · `DELETE /api/mailboxes/{name}` · `POST /api/mailboxes/{name}/clear` | mailboxes |
| `GET /api/emails?mailbox=&q=&limit=&offset=` · `GET /api/mailboxes/{name}/emails?q=&limit=&offset=` | list and search emails (`X-Total-Count` header when paging) |
| `GET /api/emails/{id}` · `DELETE /api/emails/{id}` · `POST /api/emails/delete` · `POST /api/emails/read` | email detail, delete, mark read |
| `GET /api/emails/{id}/html` · `/raw[?download=true]` · `/attachments/{n}` · `/view-data` | content |
| `POST /api/emails/{id}/forward` `{"to":["a@b.c"]}` · `POST /api/emails/{id}/recheck` | forward, re-run checks |
| `GET /api/emails/{id}/export` · `GET /api/export?ids=a,b&attachments=&technical=&device=` | single-file `.html` export |
| `GET/POST /api/shares` · `DELETE /api/shares/{token}` | live server links |
| `GET/POST /api/cloud/shares` · `POST /api/cloud/shares/{id}/renew` · `DELETE /api/cloud/shares/{id}[?keep_object=true]` | cloud shares |
| `GET /api/cloud` · `POST /api/cloud/configs` · `POST /api/cloud/configs/test` · `DELETE /api/cloud/configs/{id}` | S3 configs (secrets are write-only) |
| `POST /api/cloud/vault/{create,unlock,lock,password,sync,restore}` · `DELETE /api/cloud/vault` | credential vault |
| `GET/POST /api/remotes` · `PUT/DELETE /api/remotes/{id}` · `POST /api/remotes/poll` | remote mailboxes |
| `GET/PUT /api/settings` · `POST /api/test-email` | settings, test email (passwords are write-only: returned empty with a `*_set` flag; omit or send `null` to keep them) |
| `GET /api/events` | live updates (Server-Sent Events) |

## Supported platforms

| OS | Architectures | Artifacts | Signing |
| --- | --- | --- | --- |
| Windows 10/11 | x86 (32-bit), x64, arm64 | `*-setup.exe` (Inno Setup), `.zip`, Microsoft Store (MSIX bundle) | Authenticode via `signtool` (PFX) or Azure Trusted Signing; the Store signs Store packages |
| macOS 11+ | x64, arm64 | `.dmg`, `.app` zip, CLI `.tar.gz`, Mac App Store (sandboxed universal `.pkg`) | Developer ID + hardened runtime + notarization (ad-hoc without a certificate); Apple re-signs Store packages |
| Ubuntu 22.04+ | x86 (32-bit), x64, arm64 | `.deb`, `.tar.gz` | detached GPG signatures (`.asc`) |

Every release artifact also gets a Sigstore build-provenance attestation
(`gh attestation verify <file> -R scalexy/scratchpad`). The `SHA256SUMS` file is GPG signed and the
container image is signed with cosign.

On Linux the GUI loads X11/Wayland, xkbcommon and the GPU drivers at runtime, so the binary links
only against glibc. Headless mode has no runtime dependencies beyond glibc. Save and export dialogs use the
XDG desktop portal (`xdg-desktop-portal`, installed on standard Ubuntu desktops and recommended
by the `.deb`).

## Building

Requires Rust 1.90 or newer.

```sh
cargo build --release                                          # desktop + headless
cargo build --release -p scratchpad --no-default-features      # headless only (no GUI dependencies)
cargo test --workspace
```

Tests that need real services (S3, IMAP/SMTP relay, SpamAssassin) are `#[ignore]`d and read
`SCRATCHPAD_TEST_*` variables; CI runs them against MinIO, GreenMail and SpamAssassin containers
(see the `integration` job in `.github/workflows/ci.yml`). `cargo deny check` runs the
licence and advisory checks configured in `deny.toml`.

Cross-target builds and packages:

```sh
rustup target add x86_64-apple-darwin
scripts/package-macos.sh x86_64-apple-darwin dist      # .app, .dmg (signs if MACOS_* is set)
scripts/package-linux.sh i686-unknown-linux-gnu dist   # needs gcc-multilib; .deb + .tar.gz
pwsh scripts/package-windows.ps1 -Target aarch64-pc-windows-msvc   # needs clang for ring
scripts/smoke-test.sh target/release/scratchpad        # end-to-end check of a built binary
```

## Releases

The git tag is the version of every build. `scripts/release.sh 1.2.0` moves the changelog entries and tags the release;
pushing the tag runs `.github/workflows/release.yml`. It builds, tests, smoke-tests, signs and
packages all eight targets on native GitHub runners, then publishes a GitHub release (notes from
`CHANGELOG.md`, checksums, provenance attestations), a signed multi-arch (`amd64`, `arm64`, `386`)
container image and, when configured, Microsoft Store and Mac App Store uploads. Without signing
secrets it still produces unsigned (ad-hoc on macOS) artifacts.

* [docs/releasing.md](docs/releasing.md): release process, secrets, verifying downloads
* [docs/windows-store.md](docs/windows-store.md): publishing to the Microsoft Store
* [docs/mac-app-store.md](docs/mac-app-store.md): publishing to the Mac App Store

## Third-party assets

* Inter, © The Inter Project Authors, SIL Open Font License 1.1 (`assets/fonts/LICENSE-Inter.txt`)
* Lucide icons, ISC License (`assets/icons/LICENSE`); regenerate with `scripts/build-icons.py`

## Project layout

```
crates/scratchpad-core   SMTP server, SQLite storage, MIME parsing, link checker, spam report,
                         forwarding, shares, remote mailboxes (IMAP + Scratchpad), S3 client,
                         credential vault and cloud shares
crates/scratchpad-web    axum REST API, SSE events, share pages, embedded single-page web app,
                         single-file HTML exporter (assets/export, assets/devices.js)
crates/scratchpad        the `scratchpad` binary: CLI, headless mode, iced desktop GUI (feature `gui`)
assets/                  shared fonts (Inter) and icons (Lucide)
packaging/               app icons, macOS bundle files, Windows installer, Linux desktop/systemd/deb files
scripts/                 packaging, signing and smoke-test scripts
```
