# Releasing Scratchpad

Releases are built, signed and published by GitHub Actions (`.github/workflows/release.yml`) when
a version tag is pushed.

## Cutting a release

1. Make sure `master` is green in CI.
2. Add user-facing changes under `## [Unreleased]` in `CHANGELOG.md` as you go (*Added*,
   *Changed*, *Fixed*, *Security*, …).
3. Run the release script on an up-to-date, clean `master`:

   ```sh
   scripts/release.sh 1.2.0        # or 1.3.0-rc.1 for a prerelease
   git push origin master v1.2.0
   ```

   The script moves the *Unreleased* entries under `## [1.2.0] - <date>`, updates the
   comparison links, checks that the workspace builds, commits `Release v1.2.0` and creates an
   annotated tag. It doesn't push.

   The tag is the version: builds take it from `git describe` (`crates/version.rs`,
   `scripts/version.sh`), so `Cargo.toml` stays at `0.0.0`. A commit after a tag builds as
   e.g. `1.2.0+3.gabc1234`, and a build without git or tags as `0.0.0`. Set
   `SCRATCHPAD_VERSION` to override it, as the release workflow and `docker build
   --build-arg SCRATCHPAD_VERSION=…` do.
4. Watch the *Release* workflow. When it finishes:
   * the GitHub release `Scratchpad 1.2.0` has every artifact, `SHA256SUMS` (+ `.asc`) and the
     changelog section as release notes;
   * `ghcr.io/scalexy/scratchpad:1.2.0`, `:1.2` and `:latest` are pushed and signed;
   * the Store uploads are waiting for approval (if configured, see
     [windows-store.md](windows-store.md) and [mac-app-store.md](mac-app-store.md)).

Prereleases (any version with a `-` suffix) are marked as prereleases on GitHub, don't move the
`latest` container tag and are never sent to either store.

## What the workflow does

| Job | |
| --- | --- |
| `prepare` | Takes the version from the tag (fails unless it's `vX.Y.Z` or `vX.Y.Z-suffix`) and passes it to every job as `SCRATCHPAD_VERSION`; extracts the release notes from `CHANGELOG.md` |
| `build` (8 targets) | Builds, runs the test suite and a smoke test (SMTP capture, health check, web API, export), packages, signs and attests |
| `msix` | Builds the Microsoft Store `.msixbundle`/`.msixupload` from the Windows builds |
| `mac-app-store` | Builds the sandboxed universal Mac App Store `.pkg` from the macOS builds |
| `release` | Creates the GitHub release with checksums (GPG-signed when a key is configured) |
| `store` | Uploads to Partner Center (optional, gated by the `microsoft-store` environment) |
| `mac-app-store-upload` | Uploads to App Store Connect for TestFlight and review (optional, gated by the `mac-app-store` environment) |
| `container` | Cross-compiles and pushes the multi-arch image (amd64, arm64, 386) to `ghcr.io`, attests it and signs it with cosign |

A manual run (*Actions → Release → Run workflow*) builds and uploads everything as workflow
artifacts without publishing, which is useful to test packaging changes.

## Artifacts

| Platform | Files |
| --- | --- |
| Windows x64 / x86 / arm64 | `scratchpad-<v>-windows-<arch>-setup.exe` (installer), `scratchpad-<v>-windows-<arch>.zip` |
| macOS x64 / arm64 | `scratchpad-<v>-macos-<arch>.dmg` (notarized when configured), `scratchpad-<v>-macos-<arch>-cli.tar.gz` |
| Linux x64 / x86 / arm64 | `scratchpad-<v>-linux-<arch>.deb` (desktop app + `scratchpad` systemd service), `scratchpad-<v>-linux-<arch>.tar.gz` (+ `.asc`) |
| Container | `ghcr.io/scalexy/scratchpad:<v>` |
| Microsoft Store | `msstore-package` workflow artifact |
| Mac App Store | `mac-app-store-package` workflow artifact |

## Signing secrets

All are optional; without them the artifacts are unsigned (and the workflow says so).

| Secret | Used for |
| --- | --- |
| `APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_P12_BASE64` (base64 .p12), `APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_PASSWORD`, `APPLE_DEVELOPER_ACCOUNT_DEVELOPER_ID_APPLICATION_CERTIFICATE_IDENTITY` | Developer ID signing |
| `APPLE_DEVELOPER_ACCOUNT_OWNER_APPLE_ID`, `APPLE_DEVELOPER_ACCOUNT_TEAM_ID`, `APPLE_DEVELOPER_ACCOUNT_OWNER_APPLE_ID_APP_PASSWORD` | Notarization |
| `WINDOWS_CERTIFICATE` (base64 .pfx), `WINDOWS_CERTIFICATE_PASSWORD` | Authenticode with a certificate file |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERTIFICATE_PROFILE` | Authenticode with Azure Trusted Signing (instead of a .pfx) |
| `GPG_PRIVATE_KEY`, `GPG_PASSPHRASE` | `.deb`/tarball and `SHA256SUMS` signatures |

The macOS certificate must be a **Developer ID Application** certificate (only the Account
Holder can create one: *Certificates → + → Developer ID Application*, "G2 Sub-CA"). An *Apple
Distribution* certificate signs fine, but notarization rejects it with status *Invalid*; it is
only for the Mac App Store package. `package-macos.sh` refuses other certificates and prints
Apple's log if notarization fails. To read the log of an earlier submission:
`xcrun notarytool log <id> --apple-id … --team-id … --password …`.

Store secrets and variables are described in [windows-store.md](windows-store.md) and
[mac-app-store.md](mac-app-store.md).

## Container registry (ghcr.io)

The `container` job pushes `ghcr.io/scalexy/scratchpad` with the workflow's `GITHUB_TOKEN`; no
secrets are needed. Tags: `<version>` always, `<major>.<minor>` and `latest` for releases only.

The first release creates the package as **private**. Once, after it:

1. Open *ScaleXY → Packages → scratchpad → Package settings*.
2. *Change visibility* → **Public**, so `docker pull` works without logging in. (If it's not
   offered, allow public packages under the organization's *Settings → Packages*.)
3. Check that *Manage Actions access* lists `ScaleXY/scratchpad` with **Write**. The image's
   `org.opencontainers.image.source` label connects it to the repository on the first push; add
   it by hand if the package was created some other way.

To test the image locally: `docker buildx build --platform linux/amd64,linux/arm64,linux/386 .`

## Verifying a release

```sh
sha256sum -c SHA256SUMS --ignore-missing
gpg --verify SHA256SUMS.asc SHA256SUMS
gh attestation verify scratchpad-1.2.0-linux-x64.tar.gz --repo ScaleXY/scratchpad
gh attestation verify oci://ghcr.io/scalexy/scratchpad:1.2.0 --repo ScaleXY/scratchpad
cosign verify ghcr.io/scalexy/scratchpad:1.2.0 \
  --certificate-identity-regexp 'https://github.com/ScaleXY/scratchpad/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

## When something goes wrong

* **A build job failed:** fix the problem on `main`, delete the tag
  (`git push --delete origin v1.2.0 && git tag -d v1.2.0`), delete the draft or partial release if
  one was created, and release again. Re-running failed jobs is fine for flaky infrastructure.
* **A bad release went out:** publish a patch release (`1.2.1`). Don't reuse tags. In the Store,
  submit the patch; Partner Center can also halt a gradual rollout
  (`msstore submission rollout halt <product id>`).
