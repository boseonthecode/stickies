#!/usr/bin/env bash
set -euo pipefail

app_id="io.github.chthonic.Stickies"
script_dir=$(CDPATH= cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)
repo_root=$(CDPATH= cd -- "$script_dir/.." && pwd)
build=1

usage() {
    cat <<'USAGE'
Usage: ./scripts/install.sh [--no-build]

Builds a release binary (unless --no-build is given) and installs Stickies
under the current user's XDG directories. No root privileges are required.
USAGE
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --no-build)
            build=0
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            echo "Unknown option: $1" >&2
            usage >&2
            exit 2
            ;;
    esac
    shift
done

data_home=${XDG_DATA_HOME:-"$HOME/.local/share"}
config_home=${XDG_CONFIG_HOME:-"$HOME/.config"}
bin_home=${XDG_BIN_HOME:-"$HOME/.local/bin"}

case "$data_home" in
    /*) ;;
    *)
        echo "XDG_DATA_HOME must be an absolute path: $data_home" >&2
        exit 1
        ;;
esac
case "$config_home" in
    /*) ;;
    *)
        echo "XDG_CONFIG_HOME must be an absolute path: $config_home" >&2
        exit 1
        ;;
esac
case "$bin_home" in
    /*) ;;
    *)
        echo "XDG_BIN_HOME must be an absolute path: $bin_home" >&2
        exit 1
        ;;
esac

if [ "$build" -eq 1 ]; then
    if ! command -v cargo >/dev/null 2>&1; then
        echo "cargo was not found. Install the Rust toolchain or use --no-build." >&2
        exit 1
    fi
    (
        cd "$repo_root"
        cargo build --release
    )
fi

binary="$repo_root/target/release/stickies"
if [ ! -x "$binary" ]; then
    echo "Release binary not found at $binary. Run without --no-build." >&2
    exit 1
fi

exec_path="$bin_home/stickies"
applications_dir="$data_home/applications"
autostart_dir="$config_home/autostart"
icon_dir="$data_home/icons/hicolor/scalable/apps"
metainfo_dir="$data_home/metainfo"

escape_sed_replacement() {
    printf '%s' "$1" | sed -e 's/[\\&|]/\\&/g'
}

render_template() {
    local source=$1
    local destination=$2

    rm -f -- "$destination"
    mkdir -p -- "$(dirname -- "$destination")"
    sed "s|@EXEC@|$escaped_exec|g" "$source" >"$destination"
    chmod 644 "$destination"
}

escaped_exec=$(escape_sed_replacement "$exec_path")

rm -f -- "$exec_path"
mkdir -p -- "$bin_home"
install -m 755 "$binary" "$exec_path"

rm -f -- "$applications_dir/$app_id.desktop"
mkdir -p -- "$applications_dir"
render_template "$repo_root/data/$app_id.desktop" "$applications_dir/$app_id.desktop"

rm -f -- "$autostart_dir/$app_id-autostart.desktop"
mkdir -p -- "$autostart_dir"
render_template \
    "$repo_root/data/$app_id-autostart.desktop" \
    "$autostart_dir/$app_id-autostart.desktop"

rm -f -- "$icon_dir/$app_id.svg"
mkdir -p -- "$icon_dir"
install -m 644 "$repo_root/data/$app_id.svg" "$icon_dir/$app_id.svg"

rm -f -- "$metainfo_dir/$app_id.metainfo.xml"
mkdir -p -- "$metainfo_dir"
install -m 644 \
    "$repo_root/data/$app_id.metainfo.xml" \
    "$metainfo_dir/$app_id.metainfo.xml"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$applications_dir" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "$data_home/icons/hicolor" >/dev/null 2>&1 || true
fi

cat <<EOF
Stickies installed.

  Binary:      $exec_path
  Launcher:    $applications_dir/$app_id.desktop
  Autostart:   $autostart_dir/$app_id-autostart.desktop
  Icon:        $icon_dir/$app_id.svg
  Metainfo:    $metainfo_dir/$app_id.metainfo.xml
  Notes data:  $data_home/stickies

Add this directory to PATH if it is not already present:
  export PATH="$bin_home:\$PATH"

Then bind GNOME custom shortcuts to:
  stickies new
  stickies browse

Run "$exec_path quit" to stop an existing resident instance before relying on
the newly installed binary.
EOF

case ":$PATH:" in
    *":$bin_home:"*) ;;
    *)
        echo
        echo "Note: $bin_home is not currently in PATH."
        ;;
esac
