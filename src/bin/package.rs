//! Package the host platform into dist/.
use anyhow::{Context, Result, bail};
use std::{fs, path::PathBuf, process::Command};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error:#}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    if !std::env::args().any(|arg| arg == "--skip-build") {
        let status = Command::new("cargo")
            .args([
                "build",
                "--release",
                "--bin",
                "token-dock",
                "--bin",
                "usage",
            ])
            .current_dir(&root)
            .status()
            .context("Cargo could not be started")?;
        if !status.success() {
            bail!("Release build failed");
        }
    }
    let output = root.join("dist");
    fs::create_dir_all(&output)?;
    match std::env::consts::OS {
        "windows" => {
            let portable = output.join("TokenDock-windows");
            fs::create_dir_all(&portable)?;
            fs::copy(
                root.join("docs/licenses/OpenUsage-MIT.txt"),
                portable.join("OpenUsage-MIT.txt"),
            )?;
            fs::copy(
                root.join("docs/licenses/OpenUsage-Windows-MIT.txt"),
                portable.join("OpenUsage-Windows-MIT.txt"),
            )?;
            fs::copy(
                root.join("target/release/token-dock.exe"),
                portable.join("TokenDock.exe"),
            )?;
            fs::copy(
                root.join("target/release/usage.exe"),
                portable.join("usage.exe"),
            )?;
            println!("Portable app: {}", portable.display());
        }
        "macos" => {
            let bundle = output.join("TokenDock.app");
            let contents = bundle.join("Contents");
            let binaries = contents.join("MacOS");
            let resources = contents.join("Resources");
            fs::create_dir_all(&binaries)?;
            fs::create_dir_all(&resources)?;
            fs::copy(
                root.join("docs/licenses/OpenUsage-MIT.txt"),
                resources.join("OpenUsage-MIT.txt"),
            )?;
            fs::copy(
                root.join("docs/licenses/OpenUsage-Windows-MIT.txt"),
                resources.join("OpenUsage-Windows-MIT.txt"),
            )?;
            let executable = binaries.join("token-dock");
            fs::copy(root.join("target/release/token-dock"), &executable)?;
            fs::copy(
                root.join("assets/AppIcon.icns"),
                resources.join("AppIcon.icns"),
            )?;
            fs::write(
                contents.join("Info.plist"),
                format!(
                    r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
<key>CFBundleName</key><string>TokenDock</string>
<key>CFBundleDisplayName</key><string>TokenDock</string>
<key>CFBundleIdentifier</key><string>dev.chirag.token-dock</string>
<key>CFBundleExecutable</key><string>token-dock</string>
<key>CFBundleIconFile</key><string>AppIcon</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleInfoDictionaryVersion</key><string>6.0</string>
<key>CFBundleSignature</key><string>????</string>
<key>CFBundleShortVersionString</key><string>{version}</string>
<key>CFBundleVersion</key><string>{version}</string>
<key>CFBundleSupportedPlatforms</key><array><string>MacOSX</string></array>
<key>LSMinimumSystemVersion</key><string>13.0</string>
<key>NSHighResolutionCapable</key><true/>
</dict>
</plist>
"#,
                    version = env!("CARGO_PKG_VERSION")
                ),
            )?;
            // Eight bytes, no trailing newline. Finder uses this as the bundle type.
            fs::write(contents.join("PkgInfo"), "APPL????")?;
            #[cfg(unix)]
            {
                make_executable(&executable)?;
                sign_app(&bundle)?;
            }
            fs::copy(root.join("target/release/usage"), output.join("usage"))?;
            println!(
                "App bundle: {}\nAd-hoc signed. Notarization is not configured.",
                bundle.display()
            );
        }
        platform => bail!("Packaging is supported on Windows and macOS, not {platform}"),
    }
    Ok(())
}

#[cfg(unix)]
fn make_executable(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let mut permissions = fs::metadata(path)?.permissions();
    permissions.set_mode(permissions.mode() | 0o755);
    fs::set_permissions(path, permissions)?;
    Ok(())
}

#[cfg(unix)]
fn sign_app(bundle: &std::path::Path) -> Result<()> {
    // The linker ad-hoc signature says resources are sealed, but a freshly
    // copied bundle has no CodeResources file. Gatekeeper then reports the
    // download as damaged. An ad-hoc codesign replaces that signature.
    let status = Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(bundle)
        .status()
        .context("codesign could not be started")?;
    if !status.success() {
        bail!("Ad-hoc code signature failed");
    }
    let status = Command::new("/usr/bin/codesign")
        .args(["--verify", "--deep", "--strict"])
        .arg(bundle)
        .status()
        .context("codesign could not be started")?;
    if !status.success() {
        bail!("Ad-hoc code signature did not verify");
    }
    Ok(())
}
