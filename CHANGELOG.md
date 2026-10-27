# Changelog

All notable changes are listed here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/).

## [0.3.0] - 2026-10-27

### Changed
- Renamed the app to **Glint**. Settings move from `%APPDATA%\SiloSearch` to `%APPDATA%\Glint` automatically.
- One setup, `GlintSetup.exe`, with a choice between "Just for me" and "Full disk search" (administrator).
- The search window has no footer any more; keyboard shortcuts live in Settings.
- The search box no longer draws an accent line, and selected text uses a soft highlight.

### Added
- Settings: choose the keys for "Open file location" and "Next category".
- GitHub Actions for CI and releases, with optional code signing.

## [0.2.0] - 2026-10-27

### Added
- All / Apps / Folders / Files categories, matched-text highlighting, start-of-word ranking.
- Tray icon, desktop shortcut, new logo.

## [0.1.0] - 2026-10-26

- First version: NTFS index, desktop typing trigger, global shortcut, acrylic window, settings.

[0.3.0]: https://github.com/ThisXNeural/Glint/releases/tag/v0.3.0
