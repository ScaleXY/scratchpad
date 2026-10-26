// The version of a build, from its git tag. Included by the crates' build scripts, which
// expose it to the code as `env!("SCRATCHPAD_VERSION")`. scripts/version.sh and
// scripts/version.ps1 implement the same rules for the packaging scripts.
//
//   SCRATCHPAD_VERSION set     that version (CI and Docker builds; a leading `v` is dropped)
//   HEAD tagged v1.2.0         1.2.0
//   3 commits after v1.2.0     1.2.0+3.gabc1234
//   no git or no tag           the Cargo.toml version (0.0.0)

use std::path::Path;
use std::process::Command;

pub fn version() -> String {
    println!("cargo:rerun-if-env-changed=SCRATCHPAD_VERSION");
    if let Ok(version) = std::env::var("SCRATCHPAD_VERSION") {
        let version = version.trim().trim_start_matches('v');
        if !version.is_empty() {
            return version.to_string();
        }
    }
    // Rebuild when HEAD moves or tags change. Missing files would force a rebuild every time.
    let branch = git(&["rev-parse", "--symbolic-full-name", "HEAD"]);
    for file in ["HEAD", "packed-refs", "refs/tags"]
        .into_iter()
        .chain(branch.as_deref())
    {
        if let Some(path) = git(&["rev-parse", "--git-path", file]) {
            if Path::new(&path).exists() {
                println!("cargo:rerun-if-changed={path}");
            }
        }
    }
    git(&["describe", "--tags", "--long", "--match", "v[0-9]*"])
        .and_then(|describe| from_describe(&describe))
        .unwrap_or_else(|| std::env::var("CARGO_PKG_VERSION").unwrap())
}

/// `v1.2.0-0-gabc1234` → `1.2.0`, `v1.2.0-3-gabc1234` → `1.2.0+3.gabc1234`.
fn from_describe(describe: &str) -> Option<String> {
    let mut parts = describe.rsplitn(3, '-');
    let (hash, count, tag) = (parts.next()?, parts.next()?, parts.next()?);
    let version = tag.strip_prefix('v')?;
    Some(if count == "0" {
        version.to_string()
    } else {
        format!("{version}+{count}.{hash}")
    })
}

fn git(args: &[&str]) -> Option<String> {
    let output = Command::new("git").args(args).output().ok()?;
    let stdout = String::from_utf8(output.stdout).ok()?;
    (output.status.success() && !stdout.trim().is_empty()).then(|| stdout.trim().to_string())
}
