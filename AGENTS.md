# Music Hub Sync

## Purpose & links

- Desktop app that mirrors Music Hub projects into a local folder: versions and current stems, both directions. Talks to the API of `robinchoice/music-hub` (`GET /api/v1/sync/state`, the existing upload, download and trash endpoints, device-flow login like `robinchoice/musichub-plugin`).
- Plan and decisions: Savor document "Plan: Music Hub Sync (Sync-Ordner)" in the music-hub project. Approved design: mockup direction "Mini-Hub" (tray popover with sync status and what's new in your projects).

## Checks

```sh
cargo test --workspace
```

## Deploy

- A push to `main` runs `ci.yml` (core tests). Verify: `gh run watch`.
- No releases yet. Builds and tagged prereleases come with the Tauri app, tags only on Robin's go.

## Pitfalls

- `core::plan` is pure: remote state, last-synced index and disk scan in, actions out. Keep IO out of it so every folder rule stays a unit test in `core/tests/plan.rs`.
- Folder and file names come from `core::layout`. Changing a name rule renames files on every user's disk on the next sync.
- The Tauri shell needs webkit2gtk, which the main dev laptop lacks; build it in CI.
