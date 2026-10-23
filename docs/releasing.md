# Releasing Scratchpad

Releases are built, signed and published by GitHub Actions (`.github/workflows/release.yml`) when
a version tag is pushed.

## Cutting a release

1. Make sure `main` is green in CI.
2. Add user-facing changes under `## [Unreleased]` in `CHANGELOG.md` as you go (*Added*,
   *Changed*, *Fixed*, *Security*, …).
3. Run the release script on an up-to-date, clean `main`:

   ```sh
   scripts/release.sh 1.2.0        # or 1.3.0-rc.1 for a prerelease
   git push origin main v1.2.0
   ```

   The script bumps the workspace version (`Cargo.toml`, `Cargo.lock`), moves the *Unreleased*
   entries under `## [1.2.0] - <date>`, updates the comparison links, checks that the workspace
   builds, commits `Release v1.2.0` and creates an annotated tag. It doesn't push.
4. Watch the *Release* workflow. When it finishes:
   * the GitHub release `Scratchpad 1.2.0` has every artifact, `SHA256SUMS` (+ `.asc`) and the
     changelog section as release notes;
   * `ghcr.io/scalexy/scratchpad:1.2.0`, `:1.2` and `:latest` are pushed and signed;
   * the Store submission is waiting for approval (if configured, see
     [windows-store.md](windows-store.md)).

Prereleases (any version with a `-` suffix) are marked as prereleases on GitHub, don't move the
`latest` container tag and are never sent to the Store.

## What the workflow does

| Job | |
| --- | --- |
| `prepare` | Fails if the tag doesn't equal `v` + the workspace version; extracts the release notes from `CHANGELOG.md` |
| `build` (8 targets) | Builds, runs the test suite and a smoke test (SMTP capture, health check, web API, export), packages, signs and attests |
| `msix` | Builds the Microsoft Store `.msixbundle`/`.msixupload` from the Windows builds |
| `release` | Creates the GitHub release with checksums (GPG-signed when a key is configured) |
| `store` | Uploads to Partner Center (optional, gated by the `microsoft-store` environment) |
| `container` | Builds and pushes the multi-arch image (amd64, arm64, 386) and signs it with cosign |

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

## Signing secrets

All are optional; without them the artifacts are unsigned (and the workflow says so).

| Secret | Used for |
| --- | --- |
| `MACOS_CERTIFICATE` (base64 .p12), `MACOS_CERTIFICATE_PASSWORD`, `MACOS_SIGNING_IDENTITY` | Developer ID signing |
| `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` | Notarization |
| `WINDOWS_CERTIFICATE` (base64 .pfx), `WINDOWS_CERTIFICATE_PASSWORD` | Authenticode with a certificate file |
| `AZURE_TENANT_ID`, `AZURE_CLIENT_ID`, `AZURE_CLIENT_SECRET`, `AZURE_SIGNING_ENDPOINT`, `AZURE_SIGNING_ACCOUNT`, `AZURE_CERTIFICATE_PROFILE` | Authenticode with Azure Trusted Signing (instead of a .pfx) |
| `GPG_PRIVATE_KEY`, `GPG_PASSPHRASE` | `.deb`/tarball and `SHA256SUMS` signatures |

Microsoft Store secrets and variables are described in [windows-store.md](windows-store.md).

## Verifying a release

```sh
sha256sum -c SHA256SUMS --ignore-missing
gpg --verify SHA256SUMS.asc SHA256SUMS
gh attestation verify scratchpad-1.2.0-linux-x64.tar.gz --repo scalexy/scratchpad
cosign verify ghcr.io/scalexy/scratchpad:1.2.0 \
  --certificate-identity-regexp 'https://github.com/scalexy/scratchpad/' \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com
```

## When something goes wrong

* **A build job failed:** fix the problem on `main`, delete the tag
  (`git push --delete origin v1.2.0 && git tag -d v1.2.0`), delete the draft or partial release if
  one was created, and release again. Re-running failed jobs is fine for flaky infrastructure.
* **A bad release went out:** publish a patch release (`1.2.1`). Don't reuse tags. In the Store,
  submit the patch; Partner Center can also halt a gradual rollout
  (`msstore submission rollout halt <product id>`).
