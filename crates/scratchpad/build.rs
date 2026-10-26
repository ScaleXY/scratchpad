#[path = "../version.rs"]
mod version;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../version.rs");
    let version = version::version();
    println!("cargo:rustc-env=SCRATCHPAD_VERSION={version}");
    // Embed the icon and version information into Windows executables.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        // The numeric version is major.minor.patch.0, without prerelease or build suffix.
        let numeric = version
            .split(['-', '+'])
            .next()
            .unwrap_or_default()
            .split('.')
            .take(3)
            .enumerate()
            .fold(0u64, |acc, (i, part)| {
                acc | part.parse::<u64>().unwrap_or(0) << (48 - 16 * i)
            });
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../../packaging/icons/scratchpad.ico")
            .set("ProductName", "Scratchpad")
            .set(
                "FileDescription",
                "Scratchpad - email testing and debugging",
            )
            .set("CompanyName", "ScaleXY")
            .set("LegalCopyright", "Copyright (c) 2026 ScaleXY")
            .set("FileVersion", &version)
            .set("ProductVersion", &version)
            .set_version_info(winresource::VersionInfo::FILEVERSION, numeric)
            .set_version_info(winresource::VersionInfo::PRODUCTVERSION, numeric);
        if let Err(error) = resource.compile() {
            println!("cargo:warning=could not embed Windows resources: {error}");
        }
    }
}
