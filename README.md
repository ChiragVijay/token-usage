# TokenDock

A native Rust + GPUI app for tracking AI usage, costs, and account limits across ChatGPT (Codex), Claude Code, Cursor, Kiro, Antigravity, and OpenCode.

![Homepage with example data](docs/images/homepage-six-providers-light.jpg)

[Dark theme preview](docs/images/homepage-six-providers-dark.jpg)

## Get started

Download the app from [GitHub Releases](https://github.com/ChiragVijay/token-dock/releases/latest):

| Platform | Download | Install |
| --- | --- | --- |
| Windows x64 | `token-dock-X.Y.Z.exe` | Run the download. It installs for the current user, with no admin prompt, to `%LOCALAPPDATA%\TokenDock\token-dock.exe` and adds a TokenDock shortcut at `%APPDATA%\Microsoft\Windows\Start Menu\Programs\TokenDock.lnk`. |
| macOS 13 or later, Apple Silicon | `token-dock-X.Y.Z.dmg` | Open the disk image and drag **TokenDock.app** into **Applications**. |

The macOS release supports Apple Silicon only; there is no Intel or universal DMG. These downloads do not require Rust or the development tools below. `X.Y.Z` is the release version. Unversioned `token-dock` and legacy `open-usage` downloads are identical compatibility copies for updaters that require fixed filenames.

### macOS first launch

The DMG container is unsigned; the app inside is ad-hoc signed. Releases do not require a paid Apple Developer account and are not notarized, so macOS may show “Apple could not verify … is free of malware.” Ad-hoc signing verifies bundle integrity but does not establish an Apple-verified developer identity.

1. Download the DMG from this repository's GitHub Releases and open it. If macOS blocks it, choose **Done**, then open **System Settings → Privacy & Security**, scroll to **Security**, and choose **Open Anyway** for that download. Confirm the prompt.
2. Drag **TokenDock.app** into **Applications** and launch that installed copy.
3. If macOS blocks the app too, repeat **Open Anyway** for **TokenDock.app** and confirm. Only grant this exception when you trust the download.

This is [Apple's documented way to open an unnotarized app](https://support.apple.com/en-us/102445). The warning cannot be eliminated through ad-hoc signing. If macOS reports actual damage or malware instead, do not treat that as the same warning; download a fresh copy and report the error.

### Build from source

Install Rust **1.97.1** (or run `mise install`), then launch an optimized build:

```sh
cargo run --release --bin token-dock
```

Windows requires Visual Studio Build Tools with Desktop development with C++ and a Windows SDK. macOS requires Xcode command-line tools. On Windows, launching the desktop program installs it for the current user and continues from `%LOCALAPPDATA%\TokenDock\token-dock.exe`.

Sign in through your provider's app or CLI. TokenDock discovers existing logins and local history automatically. Use **Connections** to add a data folder or import a Cursor CSV.

| Provider | Connection |
| --- | --- |
| ChatGPT | Codex login and local session history |
| Claude Code | Claude Code OAuth login and local history |
| Cursor | Cursor desktop/CLI login or dashboard CSV |
| Kiro | Kiro CLI or IDE login, including Kiro Crew; usage is reported in credits |
| Antigravity | IDE/`agy` connection or existing OAuth login; local generation history |
| OpenCode | Local Go/Zen history; a Go key supplies account limits |

Kiro credits and Antigravity quotas are verified with Windows CLI logins. OpenCode quota parsing is fixture-tested; its signed-in live checks and macOS execution still need validation.

## Using the app

- **Accounts** shows usage totals, current allowances, reset times, and a usage chart. **Limits** shows every reported allowance.
- **Models**, **Activity**, and **Insights** provide breakdowns. Search and filter tables, click column headings to sort, and export Activity to CSV.
- Choose **2 hours, 24 hours, 7 days, 30 days, or 90 days**. The tray's quick view uses Today.
- Adjust automatic refresh, theme, and display privacy in **Settings**. Display privacy hides figures on screen; CSV exports contain actual values.
- Closing the window keeps tracking active when the tray is available. Use **Ctrl/Cmd + Q** to quit and **Ctrl/Cmd + R** to refresh.

## Updates

On Windows x64 and Apple Silicon Macs, the desktop app checks the latest stable GitHub release once per launch. A newer version offers **Update now** or **Delay**. Downloads and installation begin only when you choose **Update now**; the app then exits so a helper can replace it and relaunch it. **Delay** dismisses the offer for this launch, and the next launch checks again. You can also use **Settings → Updates → Check for updates**.

On Windows, **Update now** replaces the per-user install at `%LOCALAPPDATA%\TokenDock\token-dock.exe` and relaunches it. That folder is only the program. Settings stay in `%LOCALAPPDATA%\token-dock`. On macOS, copy the app out of the DMG before updating. If the installed copy cannot be replaced, the downloaded EXE or DMG is opened for manual installation. Mac replacement prepares the new app on the installation volume and keeps the previous app until replacement succeeds. A failed replacement restores the previous app; if restoration also fails, its backup remains in a folder named `TokenDock.app.update-*` beside the installation.

Intel Mac builds and other architectures do not receive automatic updates. The CLI does not check for app updates.

## Costs and privacy

Costs use provider-reported amounts when available, otherwise public API-rate estimates from [LiteLLM](https://github.com/BerriAI/litellm/blob/main/model_prices_and_context_window.json). Estimates are not subscription bills and exclude subscription fees. Unknown prices stay **-**; Kiro credits stay separate from tokens and dollars. Cached or expired allowances are marked.

History and settings stay on your device. Provider stores are read-only; existing credentials are used for account checks. Prompts, conversations, and usage records are never uploaded. There is no telemetry. Network requests check provider allowances/history, download model prices, and check GitHub releases. Desktop update checks run even when provider online refresh is off; update files are downloaded only after you choose **Update now**.

## CLI

```sh
cargo run --release --no-default-features --bin usage -- --online --period 7d --json
```

Use `--offline --refresh` to read local history and cached allowances. Periods: `2h`, `24h`, `7d`, `30d`, `90d`, or `today`. Run with `--help` for options.

Settings and cache live in `%LOCALAPPDATA%/token-dock` on Windows or `~/Library/Application Support/token-dock` on macOS. `TOKEN_DOCK_DATA_DIR` overrides that location.

## Development

```sh
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo run --bin package
```

Packaging creates a release build in `dist/` for the current platform. The macOS app is ad-hoc signed; the DMG container is unsigned. No Apple Developer credentials are needed. First-launch Gatekeeper approval is described above.

## Publishing a release

After the **Release** workflow is merged into the default branch:

1. Open **Actions → Release → Run workflow** and select `main`.
2. Choose a `patch`, `minor`, or `major` version bump. Choose `finish` to build the current version and publish its existing draft without bumping again.
3. Run the workflow. A bump updates `Cargo.toml` and `Cargo.lock`, commits that version to the selected branch, and creates the `vX.Y.Z` tag with a draft GitHub release. `finish` keeps the current version and the open draft.
4. Windows and Apple Silicon macOS jobs build that commit and upload `token-dock-X.Y.Z.exe` and `token-dock-X.Y.Z.dmg`, plus identical unversioned `token-dock.exe`, `token-dock.dmg`, `open-usage.exe`, and `open-usage.dmg` copies for updaters that require fixed filenames. The Mac job verifies both the Rust host and packaged executable are ARM64, runs the update-helper regression checks, and runs `hdiutil verify` and `hdiutil attach` on the versioned DMG. The app remains ad-hoc signed; the DMG container is unsigned.
5. The publish job verifies all six assets exist, adds the macOS first-launch instructions to the release notes, then publishes the release so desktop update checks can discover it.

The workflow needs permission to push the version commit and write releases; branch rules must allow that push. If packaging or publication fails, the release stays a draft. Run the workflow again with `finish` so that version gets both assets and is published. Do not start another bump while the draft is still open.

## Credits

Built with [GPUI](https://gpui.rs/) and [gpui-component](https://github.com/longbridge/gpui-component). Provider integrations and design reference [OpenUsage](https://github.com/robinebers/openusage), its [Windows fork](https://github.com/mesomya/openusage-windows), [ai-usagebar](https://github.com/akitaonrails/ai-usagebar), and [t3code](https://github.com/pingdotgg/t3code).

Provider SVGs come from OpenUsage and its Windows fork. Their [MIT notice](docs/licenses/OpenUsage-MIT.txt) and [Windows-fork notice](docs/licenses/OpenUsage-Windows-MIT.txt) are included in packaged builds.
