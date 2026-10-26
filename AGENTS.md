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
- A tag `v*` also creates a GitHub release with these files and `latest.json`. Releases only on Robin's go; the tag message body becomes the release notes (German). The tag has to match `version` in `app/src-tauri/Cargo.toml` (the only version; CI checks it): bump it, commit, then tag.
- Updates: the app checks `releases/latest/download/latest.json` 10 s after start and every 6 h, the popover offers the install. Releases are therefore no GitHub prereleases (`latest` skips them). Update bundles are signed with the updater key (secrets `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`, copies `TAURI_MUSICHUBSYNC_UPDATER_*` in `~/.secrets`); its public key is in `tauri.conf.json`. Losing the key means installed apps take no more updates.
- macOS builds are signed with the Developer ID certificate of team T2F442364K and notarized with the App Store Connect key (secrets `MAC_CERTIFICATE`, `MAC_CERTIFICATE_PASSWORD`, `ASC_KEY_P8`, variables `ASC_KEY_ID`, `ASC_ISSUER_ID`; copies from `~/.secrets`, see infra.md). The CI checks signature and notarization. Windows builds stay unsigned for now (SmartScreen warns); after the first release apply to SignPath Foundation (free for OSS, Robin applies himself).

## Pitfalls

- `core::plan` is pure: remote state, last-synced index and disk scan in, actions out. Keep IO out of it so every folder rule stays a unit test in `core/tests/plan.rs`.
- Folder and file names come from `core::layout`. Changing a name rule renames files on every user's disk on the next sync. Files named `… (lokal).wav` are the user's own copies next to the hub's file: never uploaded, shown as a hint.
- The Tauri shell needs webkit2gtk, which the main dev laptop lacks; build it in CI.
- The session token lives in the system keychain (`keyring`, service `org.pleasance.musichubsync`) and never reaches the webview; commands that need the hub run on the Rust side.
- Config (`config.json`) and the sync index (`index-<hash>.json`, one per hub and sync folder, bound to its account) live in the app's config/data folders, never in the sync folder. A new folder or hub starts with an empty index, a folder synced before gets its own back. A folder whose index belongs to another account is refused: that account's view would trash or upload what isn't its own. Logging in with another account sends the setup back to the folder step with a folder of its own.
- Rounds run every 20 s and right after a setting changes; a setting changed mid-round ends that round after the current action (a running up- or download finishes first). There is no file watcher yet, so a bounce goes up within about 25 s.
- `MUSICHUB_URL` points the app at another hub, e.g. a local one.
