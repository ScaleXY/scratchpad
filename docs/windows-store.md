# Publishing Scratchpad to the Microsoft Store

Scratchpad ships to the Store as an MSIX bundle (x64, x86 and arm64 in one `.msixbundle`). The
release workflow builds it on every tag and, once configured, uploads it to Partner Center. The
Store signs the package, so no code-signing certificate is needed for the Store build.

What's in the repository:

| File | Purpose |
| --- | --- |
| `packaging/windows/msix/AppxManifest.xml` | Package manifest template (identity filled in at build time) |
| `packaging/windows/msix/Assets/` | Tiles, Store logo, splash screen and taskbar icons (`scripts/build-msix-assets.py`) |
| `scripts/package-msix.ps1` | Builds one `.msix` per architecture, then the `.msixbundle` and `.msixupload` |
| `.github/workflows/release.yml` | `msix` job (always) and `store` job (when configured) |

The package runs as a full-trust desktop app (`runFullTrust`) on Windows 10 2004 (build 19041)
or later, and registers the `scratchpad` command for terminals (an app execution alias).

## 1. One-time setup in Partner Center

1. **Create a developer account** at <https://partner.microsoft.com/dashboard> (Apps and games).
   Use a company account if you publish as ScaleXY; verification can take a few days.
2. **Reserve the name.** *Apps and games → New product → MSIX or PWA app*, then reserve
   `Scratchpad` (or the name you want to show in the Store).
3. **Copy the package identity.** Open the app, then *Product management → Product identity* and
   note:
   * `Package/Identity/Name`, e.g. `12345ScaleXY.Scratchpad`
   * `Package/Identity/Publisher`, e.g. `CN=1A2B3C4D-5E6F-...`
   * `Package/Properties/PublisherDisplayName`, e.g. `ScaleXY`
   * the **Store ID** (12 characters, e.g. `9NBLGGH4R315`)
4. **Add GitHub repository variables** (*Settings → Secrets and variables → Actions → Variables*):

   | Variable | Value |
   | --- | --- |
   | `MSIX_IDENTITY_NAME` | Package/Identity/Name |
   | `MSIX_PUBLISHER` | Package/Identity/Publisher |
   | `MSIX_PUBLISHER_DISPLAY_NAME` | PublisherDisplayName |
   | `MSIX_DISPLAY_NAME` | the reserved name, exactly as reserved |

   From the next release on, the `msstore-package` workflow artifact contains
   `scratchpad-<version>.msixbundle` and `scratchpad-<version>.msixupload` with the right identity.

## 2. Test the package locally (recommended before the first submission)

On a Windows 10/11 machine with the Windows SDK:

```powershell
cargo build --release --locked --target x86_64-pc-windows-msvc -p scratchpad
$env:MSIX_IDENTITY_NAME = "12345ScaleXY.Scratchpad"     # values from step 1.3
$env:MSIX_PUBLISHER = "CN=1A2B3C4D-5E6F-..."
$env:MSIX_PUBLISHER_DISPLAY_NAME = "ScaleXY"
scripts/package-msix.ps1 -Target x86_64-pc-windows-msvc -OutDir dist-store
```

The Store signs packages, so an unsigned `.msix` can't be installed directly. To try it:

* **Developer Mode** (*Settings → System → For developers*), then register the staged folder that
  the script leaves in `%TEMP%\scratchpad-msix-x64`:
  `Add-AppxPackage -Register "$env:TEMP\scratchpad-msix-x64\AppxManifest.xml"`
* or sign it with a throwaway certificate whose subject equals `MSIX_PUBLISHER`:

  ```powershell
  $cert = New-SelfSignedCertificate -Type CodeSigningCert -Subject $env:MSIX_PUBLISHER `
    -CertStoreLocation Cert:\CurrentUser\My -TextExtension @("2.5.29.37={text}1.3.6.1.5.5.7.3.3", "2.5.29.19={text}")
  Export-Certificate -Cert $cert -FilePath test.cer
  Import-Certificate -FilePath test.cer -CertStoreLocation Cert:\LocalMachine\TrustedPeople   # admin
  signtool sign /fd SHA256 /sha1 $cert.Thumbprint dist-store\scratchpad-*-x64.msix
  Add-AppxPackage dist-store\scratchpad-*-x64.msix
  ```

Then run the **Windows App Certification Kit**, which performs the same checks as Store
certification:

```powershell
& "${env:ProgramFiles(x86)}\Windows Kits\10\App Certification Kit\appcert.exe" test `
  -appxpackagepath dist-store\scratchpad-0.1.0-x64.msix -reportoutputpath wack.xml
```

Check that Scratchpad starts from the Start menu, captures a test email (*Setup → Send test email*),
opens a preview in the browser, and that `scratchpad --version` works in a new terminal.

## 3. First submission (manual)

Partner Center's submission API only works for apps that already have one submission completed
in Partner Center, so the first one is manual:

1. Run a release (or the Release workflow manually) and download the `msstore-package` artifact.
2. In Partner Center, *Start your submission* and complete each section:
   * **Pricing and availability:** Free; markets; visibility.
   * **Properties:** category *Developer tools*. Add a **privacy policy URL** (required: the app
     uses the network). It should say that emails stay on the device unless the user shares or
     forwards them, and that S3 credentials stay on the device unless the user turns on sync.
   * **Age ratings:** complete the IARC questionnaire (no user-to-user communication, no
     purchases).
   * **Packages:** upload `scratchpad-<version>.msixupload`. Partner Center reads the architectures
     from the bundle.
   * **Store listings:** description, at least one screenshot (1366×768 or larger, see
     `docs/screenshots/`), logo, keywords such as *email testing, SMTP, developer tools*.
   * **Submission options → Restricted capabilities:** explain `runFullTrust`, e.g. "Scratchpad is
     a desktop developer tool packaged with MSIX. It runs a local SMTP server and a native GUI and
     needs full trust like any Win32 desktop application."
3. Submit for certification. It usually takes 1–3 business days.

## 4. Automate updates from GitHub

1. **Create an Entra ID (Azure AD) app for Partner Center:** *Account settings → User management →
   Microsoft Entra applications → Add Microsoft Entra application* (or create a new one), give it
   the **Manager** role, then *Add new key* to create a client secret. Note the **tenant ID**,
   **client ID** and **secret**. The **seller ID** is under *Account settings → Legal info →
   Developer*.
2. **Create a GitHub environment** named `microsoft-store` (*Settings → Environments*). Add
   **required reviewers** so each Store submission waits for approval, and add these environment
   secrets:
   `PARTNER_CENTER_TENANT_ID`, `PARTNER_CENTER_SELLER_ID`, `PARTNER_CENTER_CLIENT_ID`,
   `PARTNER_CENTER_CLIENT_SECRET`.
3. **Add repository variables:** `MS_STORE_PRODUCT_ID` = the Store ID. Optionally set
   `MS_STORE_AUTO_COMMIT` = `true` to submit for certification right away. Without it the upload
   stays a **draft** that you review and submit in Partner Center (*Submit to the Store*).

On every non-prerelease tag the `store` job then:

1. waits for approval in the `microsoft-store` environment,
2. installs the Microsoft Store Developer CLI (`microsoft/microsoft-store-apppublisher`),
3. runs `msstore publish --inputFile scratchpad-<version>.msixupload --appId <Store ID>`
   (`--noCommit` unless auto-commit is on), creating a new submission from the last published
   one with the new package,
4. prints the submission status.

Prereleases (`v1.2.0-rc.1`) never go to the Store. To roll out gradually, use
`msstore publish ... --packageRolloutPercentage 10`, then `msstore submission rollout update`
and `finalize` (see the CLI docs).

## Notes

* **Versions.** The MSIX version is `<major>.<minor>.<patch>.0` from the git tag. Every
  submission needs a higher version than the last one; the fourth number must stay `0`.
* **Data location.** Packaged apps write `%APPDATA%` into the package's private folder
  (`%LOCALAPPDATA%\Packages\<package family name>\LocalCache\Roaming\ScaleXY\Scratchpad`). Data from the
  installer (`.exe`) version isn't shared with the Store version, and uninstalling the Store
  version deletes its data. Users who switch can copy `scratchpad.sqlite3`, `settings.json` and
  `credentials.json` across.
* **Firewall.** The default `127.0.0.1` bindings don't trigger Windows Firewall. Binding to
  `0.0.0.0` (Docker, VMs) shows the usual firewall prompt.
* **Updates.** The Store updates the app automatically; the GitHub `.exe` installer and zip
  remain available for users who don't use the Store.
* **Assets.** Change the icon in `packaging/icons/scratchpad-1024.png`, then run
  `pip install pillow && python3 scripts/build-msix-assets.py`.
