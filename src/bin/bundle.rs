//! Builds `target/bundle/less-browser.app` (main exe, helper apps, CEF framework) and ad-hoc signs it.
//! Run from the crate root: `cargo run --bin bundle`.

use cef::build_util::mac::{BundleInfo, build_bundle};
use std::{path::Path, process::Command};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let info = BundleInfo::new(
        "less-browser",
        "dev.less.browser",
        "less-browser",
        "English",
        semver::Version::new(0, 1, 0),
    );
    let app = build_bundle(Path::new("target/bundle"), "less-browser", info)?;
    let status = Command::new("codesign")
        .args(["--force", "--deep", "--sign", "-"])
        .arg(&app)
        .status()?;
    if !status.success() {
        return Err("codesign failed".into());
    }
    println!("Bundled {}", app.display());
    Ok(())
}
