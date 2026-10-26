#[path = "../version.rs"]
mod version;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../version.rs");
    println!("cargo:rustc-env=SCRATCHPAD_VERSION={}", version::version());
}
