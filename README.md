# Stickies

Stickies is a fast, keyboard-first sticky-note application for Fedora GNOME. It captures text, images, clipboard contents, and files into durable local notes, then makes them quick to find with full-text search.

Notes are plain Markdown files under your local data directory. There is no network access, account, cloud sync, or telemetry.

## Features

- Autosaving Markdown editor with syntax highlighting
- Search across note text, tags, and attachment filenames
- Tags, pinning, trash/restore, and confirmed permanent deletion
- Inline image thumbnails and file attachments
- List and card-grid browser views
- Clipboard paste and drag-and-drop for text, images, and files
- Single-instance resident app with a thin CLI

## Dependencies

Fedora:

```bash
sudo dnf install gtk4-devel libadwaita-devel gtksourceview5-devel \
  sqlite-devel pkgconf-pkg-config gcc cargo rustfmt clippy
```

## Build

```bash
cargo build --release
```

The release binary is written to `target/release/stickies`.

## Install

Install under your home directory without root:

```bash
./scripts/install.sh
```

The script builds a release binary by default. To install an existing build:

```bash
./scripts/install.sh --no-build
```

It installs:

| Item | Location |
|---|---|
| Binary | `${XDG_BIN_HOME:-~/.local/bin}/stickies` |
| Launcher | `${XDG_DATA_HOME:-~/.local/share}/applications/io.github.chthonic.Stickies.desktop` |
| Icon | `${XDG_DATA_HOME:-~/.local/share}/icons/hicolor/scalable/apps/io.github.chthonic.Stickies.svg` |
| AppStream metainfo | `${XDG_DATA_HOME:-~/.local/share}/metainfo/io.github.chthonic.Stickies.metainfo.xml` |
| Autostart entry | `${XDG_CONFIG_HOME:-~/.config}/autostart/io.github.chthonic.Stickies-autostart.desktop` |

The install script is idempotent and overwrites only files it owns. It does not touch note data.

If `~/.local/bin` is not on your `PATH`, add it:

```bash
export PATH="$HOME/.local/bin:$PATH"
```

After upgrading, stop any existing resident process so the next launch uses the new binary:

```bash
stickies quit
```

## Usage

```bash
stickies            # open or focus the browser
stickies browse     # open or focus the browser
stickies new        # open a new note window
stickies quit       # stop the resident process
stickies --help     # CLI help
```

The autostart entry starts the resident process at login without opening a window. The first `new` or `browse` invocation then appears immediately.

### GNOME shortcuts

Open **GNOME Settings → Keyboard → Custom Shortcuts** and add:

| Shortcut | Command |
|---|---|
| `Super+N` | `stickies new` |
| `Super+Shift+N` | `stickies browse` |

Use the absolute path if your shortcut runner does not inherit your shell `PATH`, for example `~/.local/bin/stickies new`.

## Data location

Stickies stores notes under:

```text
${XDG_DATA_HOME:-~/.local/share}/stickies/
  notes/<uuid>/
    note.md
    meta.json
    attachments/
  index.db
```

- `note.md` is the authoritative Markdown body.
- `meta.json` stores tags, pin/trash state, timestamps, and attachment metadata.
- `index.db` is a rebuildable SQLite FTS5 search index.
- Data directories are created with `0700` permissions and note files with `0600` where supported.

Back up the `stickies/` directory to preserve notes and attachments. Deleting `index.db` is safe; it is rebuilt on the next launch.

## Uninstall

Remove installed files while preserving notes:

```bash
./scripts/uninstall.sh
```

Remove installed files and note data:

```bash
./scripts/uninstall.sh --purge
```

`--purge` deletes `${XDG_DATA_HOME:-~/.local/share}/stickies` and cannot be undone.

## Development

```bash
cargo test
cargo fmt --check
cargo clippy --all-targets -- -D warnings
```

Packaging assets live in `data/`; validation commands:

```bash
desktop-file-validate data/io.github.chthonic.Stickies.desktop
appstreamcli validate --no-net data/io.github.chthonic.Stickies.metainfo.xml
```
