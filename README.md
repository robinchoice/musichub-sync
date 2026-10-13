# Music Hub Sync

Keeps a folder on your computer in sync with [Music Hub](https://hub.pleasance.org). Bounce a mix into a track's `Versionen` folder and it shows up in the hub as a new version; what your band uploads lands in the folder by itself.

```
~/Music Hub/
  <Project>/
    <Track>/
      Versionen/   v1 Rough Mix.wav, v2 Rough Mix.wav, …
      Stems/       Drums.wav, Bass.wav, …
```

- A new file in `Versionen/` becomes a new version, saving over one adds a version on top of it.
- Renaming `v3 Rough Mix.wav` to `v3 Final.wav` changes the version's label in the hub.
- A new file in `Stems/` becomes a stem, saving over one gives the stem a new file and keeps the earlier one in the hub.
- A mix dropped straight into a project folder becomes a new track.
- Deleting a file moves it into the hub's trash; what's deleted in the hub goes into the system trash.
- Only audio (`wav mp3 flac aiff aif aifc`) syncs. Everything else stays local.

Status: early. The sync core (`core/`) plans what to do; the desktop app (Tauri 2, tray with a small popover) follows.

## License

AGPL-3.0
