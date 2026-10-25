# Music Hub Sync

## Purpose & links

- Desktop app that mirrors Music Hub projects into a local folder: versions and current stems, both directions. Talks to the API of `robinchoice/music-hub` (`GET /api/v1/sync/state`, the existing upload, download and trash endpoints, device-flow login like `robinchoice/musichub-plugin`).
- Plan and decisions: Savor document "Plan: Music Hub Sync (Sync-Ordner)" in the music-hub project. Approved design: mockup direction "Mini-Hub" (tray popover with sync status and what's new in your projects).

## Checks

```sh
cargo test -p musichub-sync-core && (cd app && bun run build)
```

The Tauri shell (`app/src-tauri`) only compiles in CI or where webkit2gtk is installed.

## Deploy

- A push to `main` runs `ci.yml`: core tests, then the app for macOS (universal dmg), Windows (msi) and Linux (AppImage, deb) as artifacts. Verify: `gh run watch`.
- A tag `v*` also creates a GitHub prerelease with these files. Releases only on Robin's go; the tag message body becomes the release notes (German).
- macOS builds are signed with the Developer ID certificate of team T2F442364K and notarized with the App Store Connect key (secrets `MAC_CERTIFICATE`, `MAC_CERTIFICATE_PASSWORD`, `ASC_KEY_P8`, variables `ASC_KEY_ID`, `ASC_ISSUER_ID`; copies from `~/.secrets`, see infra.md). The CI checks signature and notarization. Windows builds stay unsigned for now (SmartScreen warns); after the first release apply to SignPath Foundation (free for OSS, Robin applies himself).

## Pitfalls

- `core::plan` is pure: remote state, last-synced index and disk scan in, actions out. Keep IO out of it so every folder rule stays a unit test in `core/tests/plan.rs`.
- Folder and file names come from `core::layout`. Changing a name rule renames files on every user's disk on the next sync.
- The Tauri shell needs webkit2gtk, which the main dev laptop lacks; build it in CI.
- The session token lives in the system keychain (`keyring`, service `org.pleasance.musichubsync`) and never reaches the webview; commands that need the hub run on the Rust side.
- Config (`config.json`) and the sync index (`index-<hub host>.json`, bound to its sync folder) live in the app's config/data folders, never in the sync folder. A new folder or hub starts with an empty index.
- Rounds run every 20 s and right after a setting changes; there is no file watcher yet, so a bounce goes up within about 25 s.
- `MUSICHUB_URL` points the app at another hub, e.g. a local one.
