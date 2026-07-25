# AI Usage

[![GitHub](https://img.shields.io/badge/GitHub-peipeitu%2Fai--usage-24292f?logo=github)](https://github.com/peipeitu/ai-usage)
[![Issues](https://img.shields.io/github/issues/peipeitu/ai-usage)](https://github.com/peipeitu/ai-usage/issues)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)

AI Usage is a small cross-platform Tauri desktop dashboard for local AI coding usage statistics. It reads local data in read-only mode and shows recent usage, token estimates, model distribution, workspace distribution, and recent sessions.

## Supported AI Sources

- Codex
  - Reads local Codex data from `CODEX_HOME` when set.
  - Defaults to `~/.codex`.
  - Shows local account name and Codex plan information when available.
  - Estimates cost from local Codex session token events using a CodexBar-compatible `$1 / 1M local tokens` estimate.
- Claude Code
  - Reads local Claude Code session logs from `CLAUDE_CONFIG_DIR` or `CLAUDE_HOME` when set.
  - Defaults to `~/.claude`.
  - Aggregates assistant message usage from Claude Code JSONL logs.
  - Reads local Claude account identity and rate-limit tier metadata when available.
- GitHub Copilot
  - Reads local VS Code Copilot extension storage from `GITHUB_COPILOT_HOME` or `COPILOT_HOME` when set.
  - Defaults to VS Code/Copilot global storage, such as `~/Library/Application Support/Code/User/globalStorage/github.copilot-chat` on macOS.
  - Aggregates recognized local Copilot chat usage fields and falls back to text-length token estimates when explicit token usage is unavailable.
- Cursor
  - Reads local Cursor editor storage from `AI_USAGE_CURSOR_HOME` or `CURSOR_HOME` when set.
  - Defaults to Cursor global storage, such as `~/Library/Application Support/Cursor/User/globalStorage` on macOS.
  - Aggregates recognized local Cursor chat/composer usage fields and falls back to text-length token estimates when explicit token usage is unavailable.
- ChatGPT
  - Reads local ChatGPT data from `AI_USAGE_CHATGPT_HOME` or `CHATGPT_HOME` when set.
  - Defaults to common ChatGPT desktop app storage, such as `~/Library/Application Support/com.openai.chat` on macOS.
  - Can also read ChatGPT export folders that include `conversations.json`.
  - Aggregates recognized local/exported conversation fields and falls back to text-length token estimates when explicit token usage is unavailable.
  - Shows local activity-based usage-window estimates when official quota data is unavailable.

## Accuracy Notes

- AI Usage only reads local files. It does not call provider billing APIs or upload usage data.
- Codex cost uses a local `$1 / 1M tokens` estimate and is not an official invoice.
- Claude Code, GitHub Copilot, Cursor, and ChatGPT token/activity numbers are local estimates. The app shows token usage for these providers instead of dollar costs unless a reliable price model is available.
- Remaining-usage cards for Claude Code and ChatGPT are inferred from local activity windows and may differ from official provider limits.

## Current Scope

- Supports Windows and Linux release packages through Tauri. macOS can be built locally, but no macOS release package is published yet.
- Uses a local desktop dashboard with a sidebar provider switch.
- Includes settings for Codex directory, Claude Code directory, GitHub Copilot directory, Cursor directory, ChatGPT directory, theme, accent color, chart period, enabled providers, and auto-refresh interval.
- Defaults to a 30-day chart period.
- Auto refresh can be disabled or configured in minutes. Manual refresh resets the auto-refresh countdown.
- Shows remaining usage in the system tray, with a Windows popup for provider details and refresh status.

## Development

```sh
npm install
npm run dev
```

Run tests:

```sh
npm test
```

Run formatting and lint checks:

```sh
npm run fmt:check
npm run lint
```

## Feedback

Bug reports, feature requests, and usage questions are welcome in [GitHub Issues](https://github.com/peipeitu/ai-usage/issues).

## License

This project is licensed under the [MIT License](LICENSE).

## Packaging

Build all artifacts from the repository root after installing dependencies.

### macOS

No macOS package is published for now. macOS distribution needs a Developer ID certificate, signing, and notarization to avoid Gatekeeper warnings, and the updater flow also needs a separate signed macOS artifact.

For local self-use, you can build an ad-hoc signed app on your own Mac. This validates the local bundle structure but does not replace Developer ID signing or notarization:

```sh
npm run package:mac
```

Output:

- `src-tauri/target/release/bundle/macos/AI Usage.app`

The packaging command applies an ad-hoc signature and runs strict `codesign` verification before it succeeds.

Install from the local app bundle:

```sh
cp -R "src-tauri/target/release/bundle/macos/AI Usage.app" /Applications/
open "/Applications/AI Usage.app"
```

If macOS blocks a trusted local build, clear the quarantine attribute:

```sh
xattr -dr com.apple.quarantine "/Applications/AI Usage.app"
open "/Applications/AI Usage.app"
```

This local build command is not used for release publishing.

### Windows

```sh
npm run package:win
```

Output:

- `src-tauri/target/release/bundle/nsis/AI Usage_<version>_<arch>-setup.exe`
- `src-tauri/target/release/bundle/nsis/AI Usage_<version>_<arch>-setup.exe.sig`

Install:

1. Run the generated `.exe` installer.
2. Follow the installer prompts.
3. Launch `AI Usage` from the Start menu or desktop shortcut.

The `.sig` file is used by the in-app updater to verify Windows updates. It is generated when `TAURI_SIGNING_PRIVATE_KEY` is set.

Unsigned Windows builds may show a SmartScreen warning. Use an Authenticode code-signing certificate in CI or your local certificate store for public distribution.

### Linux

```sh
npm run package:linux
```

Outputs:

- `src-tauri/target/release/bundle/appimage/ai-usage_<version>_<arch>.AppImage`
- `src-tauri/target/release/bundle/appimage/ai-usage_<version>_<arch>.AppImage.sig`
- `src-tauri/target/release/bundle/deb/ai-usage_<version>_<arch>.deb`

Install AppImage:

```sh
chmod +x src-tauri/target/release/bundle/appimage/ai-usage_0.1.6_amd64.AppImage
./src-tauri/target/release/bundle/appimage/ai-usage_0.1.6_amd64.AppImage
```

Install Debian package:

```sh
sudo apt install ./src-tauri/target/release/bundle/deb/ai-usage_0.1.6_amd64.deb
```

Package filenames include the current package version and target architecture, so adjust the examples if your generated filename differs.

### Auto Updates

Automatic updates are enabled only for Windows and Linux release builds. macOS is intentionally not wired to the updater or release packaging yet.

Generate updater signing keys once:

```sh
npx tauri signer generate -w ~/.tauri/ai-usage.key
```

Keep the private key secret. Build Windows or Linux releases with the public key embedded and the private key available for artifact signing:

```sh
export AI_USAGE_UPDATER_PUBLIC_KEY="content of the generated public key"
export TAURI_SIGNING_PRIVATE_KEY="$HOME/.tauri/ai-usage.key"
npm run package:linux
```

On Windows PowerShell:

```powershell
$env:AI_USAGE_UPDATER_PUBLIC_KEY="content of the generated public key"
$env:TAURI_SIGNING_PRIVATE_KEY="C:\Users\you\.tauri\ai-usage.key"
npm run package:win
```

The app checks `https://github.com/peipeitu/ai-usage/releases/latest/download/latest.json`. Tagged GitHub Actions releases require updater signing and always generate and upload this manifest.

Tagged releases require these GitHub Actions repository secrets:

- `AI_USAGE_UPDATER_PUBLIC_KEY`: the complete contents of `ai-usage.key.pub`.
- `TAURI_SIGNING_PRIVATE_KEY`: the complete contents of `ai-usage.key`, not a local file path.
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`: the private-key password when the key is encrypted; omit it for an unencrypted key.

Tagged builds fail before packaging when the public or private key is missing. Unsigned packages are available only through a manual `workflow_dispatch` build. CI verifies both installers against the configured public key before uploading them to a draft release. After publishing, it verifies the tag-specific manifest and client-facing `latest` manifest, downloads both released installers, and cryptographically verifies them against their uploaded signatures and the configured public key; a failed public verification returns the release to draft.

A normal rerun never overwrites an existing published release: it verifies the public assets and exits successfully when they are healthy. If the current latest release is published but broken, start the current workflow from `main` and explicitly select the source tag to rebuild:

```sh
gh workflow run build.yml --ref main -f platform=all -f package=true -f repair_release=true -f release_tag=v0.1.6
```

Repair mode is accepted only for an existing release. A published target must be the current GitHub Latest release; a Draft target may resume an interrupted repair only when it is not older than the current Latest release. Repair keeps the current workflow tooling on `main` while building the application source from `release_tag`. Release publication jobs share a repository-wide queue, and the workflow checks GitHub Latest again immediately before publishing, so concurrent or historical runs cannot move the updater channel backwards.

For a local signed release dry run, generate the manifest from already-built Windows and Linux artifacts:

```sh
npm run updater:latest -- --artifacts release-artifacts --output release-artifacts/latest.json --repo peipeitu/ai-usage --tag v0.1.6
```

Linux AppImage GPG signing is optional and separate from the updater signature:

```sh
SIGN=1 SIGN_KEY="your-gpg-key-id" APPIMAGETOOL_FORCE_SIGN=1 npm run package:linux
```
