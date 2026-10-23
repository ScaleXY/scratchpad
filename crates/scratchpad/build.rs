fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    // Embed the icon and version information into Windows executables.
    #[cfg(windows)]
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let mut resource = winresource::WindowsResource::new();
        resource
            .set_icon("../../packaging/icons/scratchpad.ico")
            .set("ProductName", "Scratchpad")
            .set(
                "FileDescription",
                "Scratchpad - email testing and debugging",
            )
            .set("CompanyName", "ScaleXY")
            .set("LegalCopyright", "Copyright (c) 2026 ScaleXY");
        if let Err(error) = resource.compile() {
            println!("cargo:warning=could not embed Windows resources: {error}");
        }
    }
}
