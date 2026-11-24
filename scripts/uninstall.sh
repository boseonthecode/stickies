#!/usr/bin/env bash
set -euo pipefail

app_id="io.github.chthonic.Stickies"
purge=0

usage() {
    cat <<'USAGE'
Usage: ./scripts/uninstall.sh [--purge]

Removes the Stickies binary and desktop integration files. Note data is kept
unless --purge is supplied.
USAGE
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --purge)
            purge=1
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

exec_path="$bin_home/stickies"
applications_dir="$data_home/applications"
autostart_dir="$config_home/autostart"
icon_dir="$data_home/icons/hicolor/scalable/apps"
metainfo_dir="$data_home/metainfo"
data_dir="$data_home/stickies"

remove_file() {
    rm -f -- "$1"
}

remove_file "$exec_path"
remove_file "$applications_dir/$app_id.desktop"
remove_file "$autostart_dir/$app_id-autostart.desktop"
remove_file "$icon_dir/$app_id.svg"
remove_file "$metainfo_dir/$app_id.metainfo.xml"
remove_file "$data_home/dbus-1/services/$app_id.service"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$applications_dir" >/dev/null 2>&1 || true
fi
if command -v gtk-update-icon-cache >/dev/null 2>&1; then
    gtk-update-icon-cache -q -t -f "$data_home/icons/hicolor" >/dev/null 2>&1 || true
fi

if [ "$purge" -eq 1 ]; then
    if [ -L "$data_dir" ]; then
        echo "Refusing to purge a symlinked data directory: $data_dir" >&2
        exit 1
    fi
    if [ -n "$data_dir" ] && [ "$data_dir" != "/" ] && [ -d "$data_dir" ]; then
        rm -rf -- "$data_dir"
        echo "Removed note data: $data_dir"
    fi
else
    echo "Note data preserved at: $data_dir"
    echo "Run ./scripts/uninstall.sh --purge to remove it."
fi

echo "Stickies uninstalled."
