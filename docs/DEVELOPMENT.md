# Development and releases

## Updates

On Windows x64 and Apple Silicon Macs, the desktop app checks the latest stable GitHub release once per launch. A newer version offers **Update now** or **Delay**. Downloads and installation begin only when you choose **Update now**; the app then exits so a helper can replace it and relaunch it. **Delay** dismisses the offer for this launch, and the next launch checks again. You can also use **Settings → Updates → Check for updates**.

On Windows, **Update now** replaces the per-user install at `%LOCALAPPDATA%\Token Usage\token-usage.exe` and relaunches it. That folder is only the program. Settings stay in `%LOCALAPPDATA%\token-usage`. On macOS, copy the app out of the DMG before updating. If the installed copy cannot be replaced, the downloaded EXE or DMG is opened for manual installation. Mac replacement prepares the new app on the installation volume and keeps the previous app until replacement succeeds. A failed replacement restores the previous app; if restoration also fails, its backup remains in a folder named `Token Usage.app.update-*` beside the installation.

Intel Mac builds and other architectures do not receive automatic updates. The CLI does not check for app updates.

Settings and cache live in `%LOCALAPPDATA%/token-usage` on Windows or `~/Library/Application Support/token-usage` on macOS. `TOKEN_USAGE_DATA_DIR` overrides that location.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo run --bin package
```

Packaging creates a release build in `dist/` for the current platform. The macOS app is ad-hoc signed; the DMG container is unsigned. No Apple Developer credentials are needed. See the [README installation notes](../README.md#install) for first-launch approval.

## App icons

The green gauge artwork lives in `assets/AppIcon.svg`; `assets/TrayIcon.svg` is the monochrome macOS tray mark. Run `node scripts/generate-icons.cjs` with the `sharp` package available to regenerate the PNG, multi-size Windows ICO, macOS ICNS, and tray RGBA assets. Commit the generated assets with artwork changes. Builds do not require Node.js.

## Publishing a release

After the **Release** workflow is merged into the default branch:

1. Open **Actions → Release → Run workflow** and select `main`.
2. Choose a `patch`, `minor`, or `major` version bump. Choose `finish` to build the current version and publish its existing draft without bumping again.
3. Run the workflow. A bump updates `Cargo.toml` and `Cargo.lock`, commits that version to the selected branch, and creates the `vX.Y.Z` tag with a draft GitHub release. `finish` keeps the current version and the open draft.
4. Windows and Apple Silicon macOS jobs build that commit and upload `token-usage-X.Y.Z.exe` and `token-usage-X.Y.Z.dmg`, plus identical unversioned `token-usage.exe`, `token-usage.dmg`, `open-usage.exe`, and `open-usage.dmg` copies for updaters that require fixed filenames. The Mac job verifies both the Rust host and packaged executable are ARM64, runs the update-helper regression checks, and runs `hdiutil verify` and `hdiutil attach` on the versioned DMG. The app remains ad-hoc signed; the DMG container is unsigned.
5. The publish job verifies all six assets exist, adds the macOS first-launch instructions to the release notes, then publishes the release so desktop update checks can discover it.

The workflow needs permission to push the version commit and write releases; branch rules must allow that push. If packaging or publication fails, the release stays a draft. Run the workflow again with `finish` so that version gets both assets and is published. Do not start another bump while the draft is still open.
