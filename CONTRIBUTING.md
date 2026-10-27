# Contributing to Glint

Thanks for helping. Bug reports, ideas and pull requests are all welcome.

## Before you start

- For a bug, open an issue with your Windows version, how you installed Glint
  (Just for me / Full disk search) and the steps to reproduce.
  If Glint crashed, attach `%APPDATA%\Glint\crash.log`.
- For a bigger change, open an issue first so we can agree on the approach.

## Development

```bash
rustup target add x86_64-pc-windows-gnu
cargo test --workspace          # engine and settings tests
cargo run -p glint              # UI with demo data (Linux/macOS)
bash installer/build.sh         # Windows exe + setup (needs mingw and nsis)
```

Preview knobs for UI work on Linux: `GLINT_QUERY`, `GLINT_CATEGORY` (0-3),
`GLINT_DARK` (0/1), `GLINT_SETTINGS=1` to open the settings window.

Guidelines:
- Keep idle CPU and memory near zero. Anything that runs in the background
  should wait on an event, not poll.
- Match the Windows 11 look (Segoe UI Variable, Fluent spacing, system accent).
- Run `cargo test --workspace` before opening a pull request.

## Releases and signing

1. Update the version in `crates/glint/Cargo.toml` and `CHANGELOG.md`.
2. Tag and push: `git tag v0.3.0 && git push origin v0.3.0`.
3. The **Release** workflow builds `Glint.exe` and `GlintSetup.exe`, signs them
   when a certificate is configured, and publishes the GitHub release.

To enable signing, add two repository secrets
(Settings → Secrets and variables → Actions):

| Secret | Value |
|---|---|
| `SIGNING_CERT_PFX_BASE64` | The code-signing certificate (`.pfx`), base64-encoded: `base64 -w0 cert.pfx` |
| `SIGNING_CERT_PASSWORD` | The password of that `.pfx` |

A self-signed certificate does not remove the Windows SmartScreen warning;
only a certificate from a trusted authority does. Free options for open source
projects include [SignPath Foundation](https://signpath.org/).
