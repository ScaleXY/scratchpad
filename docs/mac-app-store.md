# Publishing Scratchpad to the Mac App Store

The direct download (`.dmg`, Developer ID signed and notarized) is built by
`scripts/package-macos.sh` and described in [releasing.md](releasing.md). The Mac App Store build
is different:

| | Direct download (`.dmg`) | Mac App Store (`.pkg`) |
| --- | --- | --- |
| Binary | one per architecture | universal (arm64 + x64) |
| Signing | Developer ID Application, notarized | Apple Distribution + Mac Installer Distribution; Apple re-signs |
| Entitlements | `packaging/macos/entitlements.plist` (hardened runtime) | `packaging/macos/entitlements-appstore.plist` (App Sandbox) |
| Data folder | `~/Library/Application Support/com.scalexy.scratchpad` | `~/Library/Containers/com.scalexy.scratchpad/Data/Library/Application Support/com.scalexy.scratchpad` |
| Updates | download a new release | the App Store |

What's in the repository:

| File | Purpose |
| --- | --- |
| `scripts/package-macos-appstore.sh` | Builds the sandboxed universal app and the signed `.pkg`; `--upload` sends it to App Store Connect |
| `packaging/macos/entitlements-appstore.plist` | App Sandbox entitlements: network server and client, files chosen in save dialogs |
| `packaging/macos/Info.plist` | Shared with the direct download (bundle ID `com.scalexy.scratchpad`, category *Developer Tools*) |
| `.github/workflows/release.yml` | `mac-app-store` job (always) and `mac-app-store-upload` job (when configured) |

## What the sandbox changes

Run headless, the sandboxed build captures mail, serves the web interface, checks links and
exports. The desktop flows in the checklist below still need a manual pass.

* **Data lives in the app's container** (path above), not in `~/Library/Application Support`. The
  direct download and the Store version share the bundle ID but not their data. To move data
  across, quit the app and copy `scratchpad.sqlite3`, `settings.json` and `credentials.json`.
* **Files can only be written where the user picks** in a save dialog (exports, `.eml`,
  attachments). `--data-dir` outside the container fails with *Operation not permitted*.
* **Headless and command-line use** of the binary inside the Store app is sandboxed too. For
  servers, CI and scripts use the CLI archive, the `.deb` or the container image instead.

## 1. One-time setup

You need a paid Apple Developer Program membership for ScaleXY (an organization account needs a
D-U-N-S number). The **Team ID** is under *Membership details* at
<https://developer.apple.com/account>.

1. **Register the App ID.** *Certificates, Identifiers & Profiles → Identifiers → +* →
   *App IDs* → *App*. Platform macOS, description `Scratchpad`, explicit bundle ID
   `com.scalexy.scratchpad`. No capabilities are needed: the App Sandbox is an entitlement.
2. **Create two certificates.** *Certificates → +*, once for **Apple Distribution** and once for
   **Mac Installer Distribution**. Each asks for a certificate signing request: in Keychain Access,
   *Certificate Assistant → Request a Certificate From a Certificate Authority*, saved to disk.
   Download both certificates and double-click them to add them to your login keychain.
   `security find-identity -v` then lists `Apple Distribution: ScaleXY (TEAMID)` and
   `3rd Party Mac Developer Installer: ScaleXY (TEAMID)`.
3. **Create the provisioning profile.** *Profiles → +* → *Distribution → Mac App Store Connect*,
   App ID `com.scalexy.scratchpad`, the Apple Distribution certificate. Download the
   `.provisionprofile`.
4. **Create the app in App Store Connect.** <https://appstoreconnect.apple.com> → *Apps → + →
   New App*: platform macOS, name `Scratchpad` (names are unique across the App Store; if it's
   taken, use something like `Scratchpad by ScaleXY`, at most 30 characters), primary language,
   bundle ID `com.scalexy.scratchpad`, SKU e.g. `scratchpad-macos`.
5. **Create an API key for uploads.** *Users and Access → Integrations → App Store Connect API →
   Team Keys → +*, access **App Manager**. Download `AuthKey_<KEY ID>.p8` (only possible once) and
   note the **Key ID** and the **Issuer ID** shown above the list.

## 2. Build and test locally

The Store only accepts distribution-signed packages, and those can't be launched directly. So
test the sandbox with an ad-hoc signed build first, then the signed build through TestFlight.

**Sandbox test build** (no certificates needed):

```sh
rustup target add aarch64-apple-darwin x86_64-apple-darwin
scripts/package-macos-appstore.sh dist
open dist/appstore-test/Scratchpad.app
```

Check, in the sandboxed app:

* *Setup → Send test email* arrives, with link and spam results;
* *Export .html* and *Save .eml* write where you pick;
* *Open in browser* shows the HTML preview, attachments open, links in an email open;
* a desktop notification appears for new mail;
* a cloud share uploads to your bucket.

**Signed build:**

```sh
export MAS_PROVISIONING_PROFILE=~/Downloads/Scratchpad_Mac_App_Store.provisionprofile
scripts/package-macos-appstore.sh dist          # dist/scratchpad-<version>-macos-appstore.pkg
```

The script takes the team and application identifier from the profile and signs with the
certificates in your login keychain. If you have certificates from more than one team, set
`MAS_APP_IDENTITY` and `MAS_INSTALLER_IDENTITY` to the full names.

**Upload** (or drag the `.pkg` into Apple's *Transporter* app):

```sh
export APP_STORE_CONNECT_KEY_ID=ABC123DEFG
export APP_STORE_CONNECT_ISSUER_ID=57246542-96fe-1a63-e053-0824d011072a
export APP_STORE_CONNECT_KEY="$(cat ~/Downloads/AuthKey_ABC123DEFG.p8)"
scripts/package-macos-appstore.sh --upload dist/scratchpad-0.1.0-macos-appstore.pkg
```

The upload is validated first, so signing or `Info.plist` problems are reported before
anything is sent.

## 3. First submission

1. **Wait for processing** (usually 5–30 minutes). The build appears under *TestFlight*, and
   Apple emails you if it finds problems.
2. **Export compliance.** App Store Connect asks whether the app uses encryption. Scratchpad
   implements standard encryption itself rather than only calling macOS: TLS (rustls) for SMTP,
   IMAP, S3 and HTTPS, and XChaCha20-Poly1305 with Argon2id for the credential vault. Answer the
   questions accordingly and check Apple's export compliance guide for whether documentation is
   needed. Once you know the answer, add `ITSAppUsesNonExemptEncryption` to
   `packaging/macos/Info.plist` so it isn't asked for every build.
3. **Test with TestFlight.** Add yourself as an internal tester, install the TestFlight app on a
   Mac and run through the checks from step 2.
4. **Fill in the App Store page** for the version:
   * **Screenshots:** 16:10, 1280×800, 1440×900, 2560×1600 or 2880×1800. Render them with
     `cargo test -p scratchpad -- --ignored gui_snapshots`; the `store-*.png` files in
     `target/gui-snapshots/` are 2880×1800.
   * **Description, keywords** (e.g. *email testing, SMTP, developer tools, Laravel*), **support
     URL** and **privacy policy URL** (required). The policy should say that emails stay on the
     Mac unless the user forwards or shares them, and that S3 credentials stay on the Mac unless
     the user turns on sync.
   * **App Privacy:** *Data Not Collected* as long as there's no analytics and sync is a stub.
     Shares go to the user's own bucket. Revisit this when the cloud sync service ships.
   * **Category:** *Developer Tools* (from `Info.plist`), **age rating** questionnaire,
     **pricing and availability**.
   * **App Review notes:** no sign-in is needed. For example: "Scratchpad is a local SMTP server
     that captures emails sent by apps under development. To try it, open Setup and click *Send
     test email*; the email appears in the inbox. Any SMTP client can send to 127.0.0.1:2525."
5. **Select the build and submit for review.** Reviews usually take 1–3 days.

## 4. Automate uploads from GitHub

1. **Repository secrets** (*Settings → Secrets and variables → Actions*):

   | Secret | Value |
   | --- | --- |
   | `MAS_CERTIFICATES` | In Keychain Access → *My Certificates*, select both certificates, *Export 2 items…* as `.p12` with a password, then `base64 -i certs.p12 \| pbcopy` |
   | `MAS_CERTIFICATES_PASSWORD` | the `.p12` password |
   | `MAS_PROVISIONING_PROFILE` | `base64 -i Scratchpad_Mac_App_Store.provisionprofile \| pbcopy` |

   From the next release on, the `mac-app-store-package` workflow artifact holds the signed
   `scratchpad-<version>-macos-appstore.pkg`. Without these secrets the job still builds an
   unsigned test package, which shows the packaging works.
2. **Create a GitHub environment** named `mac-app-store` (*Settings → Environments*) with
   **required reviewers**, and add the environment secrets `APP_STORE_CONNECT_KEY_ID`,
   `APP_STORE_CONNECT_ISSUER_ID` and `APP_STORE_CONNECT_KEY` (the contents of the `.p8` file).
3. **Add the repository variable** `MAS_UPLOAD` = `true`.

On every non-prerelease tag, the `mac-app-store-upload` job then waits for approval and uploads
the package. When it has been processed, add the *What's New* text in App Store Connect, select
the build and submit it for review. Prereleases (`v1.2.0-rc.1`) are never uploaded.

## Notes

* **Versions.** `CFBundleShortVersionString` is the version from `Cargo.toml` without a
  prerelease suffix and must be higher than the last approved version. `CFBundleVersion` (the
  build) is the same by default, and each upload of a version needs a new, higher build. To upload
  the same version again, set `MAS_BUILD_NUMBER`. It can have at most three numbers, so for
  `1.2.0` use e.g. `1.2.1`; the version shown in the Store stays `1.2.0`.
* **Quarantined files (ITMS-91109).** Files downloaded in a browser, such as the provisioning
  profile, carry a `com.apple.quarantine` attribute, which App Store Connect rejects. The script
  removes extended attributes from the app before signing and checks the finished package, so
  rebuild with it rather than repackaging by hand.
* **Renewals.** Distribution certificates expire after a year. Create new ones and a new
  profile, then update the three repository secrets. Installed copies keep working.
* **App Review rules that apply** (guideline 2.4.5): the app is sandboxed, has no updater of its
  own, doesn't start at login and stops its SMTP and web servers when it quits.
* **SDK.** The upload records the Xcode and SDK used (`DT*` keys in `Info.plist`). If App Store
  Connect starts requiring a newer macOS SDK, move the macOS build jobs in `release.yml` to a
  newer runner (e.g. `macos-26`).
