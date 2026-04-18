#!/usr/bin/env sh
# omd installer — downloads the latest GitHub release binary for your
# platform and drops it into a directory on your PATH.
#
# Usage:
#   curl -fsSL https://raw.githubusercontent.com/Allen-Saji/openmetadata-cli/main/install.sh | sh
#
# Env overrides:
#   INSTALL_DIR   destination (default: $HOME/.local/bin, falls back to /usr/local/bin if writable)
#   OMD_VERSION   specific tag to install (default: latest)

set -eu

REPO="Allen-Saji/openmetadata-cli"
BIN_NAME="omd"

err() { printf '%s\n' "error: $*" >&2; exit 1; }
info() { printf '%s\n' "$*"; }

have() { command -v "$1" >/dev/null 2>&1; }

detect_platform() {
    uname_s=$(uname -s 2>/dev/null || echo unknown)
    uname_m=$(uname -m 2>/dev/null || echo unknown)
    case "$uname_s" in
        Linux)  os=linux ;;
        Darwin) os=darwin ;;
        MINGW*|MSYS*|CYGWIN*) os=windows ;;
        *) err "unsupported OS: $uname_s" ;;
    esac
    case "$uname_m" in
        x86_64|amd64) arch=x86_64 ;;
        aarch64|arm64) arch=aarch64 ;;
        *) err "unsupported arch: $uname_m" ;;
    esac
    PLATFORM="${arch}-${os}"
}

fetch_latest_tag() {
    if have curl; then
        curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" \
            | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' \
            | head -n 1
    elif have wget; then
        wget -qO- "https://api.github.com/repos/$REPO/releases/latest" \
            | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' \
            | head -n 1
    else
        err "curl or wget required"
    fi
}

download() {
    url=$1; dest=$2
    if have curl; then
        curl -fsSL "$url" -o "$dest"
    else
        wget -qO "$dest" "$url"
    fi
}

pick_install_dir() {
    if [ -n "${INSTALL_DIR:-}" ]; then
        echo "$INSTALL_DIR"
        return
    fi
    candidate="$HOME/.local/bin"
    mkdir -p "$candidate" 2>/dev/null || true
    if [ -w "$candidate" ]; then
        echo "$candidate"
        return
    fi
    if [ -w /usr/local/bin ]; then
        echo /usr/local/bin
        return
    fi
    echo "$candidate"
}

main() {
    detect_platform

    version="${OMD_VERSION:-$(fetch_latest_tag)}"
    [ -n "$version" ] || err "could not determine latest version (set OMD_VERSION to override)"

    case "$os" in
        windows) archive_ext=zip ;;
        *)       archive_ext=tar.gz ;;
    esac
    archive="omd-${version}-${PLATFORM}.${archive_ext}"
    url="https://github.com/$REPO/releases/download/${version}/${archive}"

    dest_dir=$(pick_install_dir)
    mkdir -p "$dest_dir"

    tmp=$(mktemp -d 2>/dev/null || mktemp -d -t omd)
    trap 'rm -rf "$tmp"' EXIT

    info "installing omd $version for $PLATFORM -> $dest_dir"
    info "  $url"
    download "$url" "$tmp/$archive"

    case "$archive_ext" in
        tar.gz) tar -xzf "$tmp/$archive" -C "$tmp" ;;
        zip)
            if have unzip; then unzip -q "$tmp/$archive" -d "$tmp"
            else err "unzip required to extract $archive"; fi
            ;;
    esac

    bin_src=$(find "$tmp" -type f -name "$BIN_NAME" -o -name "${BIN_NAME}.exe" | head -n 1)
    [ -n "$bin_src" ] || err "binary '$BIN_NAME' not found in archive"

    install -m 0755 "$bin_src" "$dest_dir/$BIN_NAME" 2>/dev/null \
        || { cp "$bin_src" "$dest_dir/$BIN_NAME" && chmod 0755 "$dest_dir/$BIN_NAME"; }

    info "installed: $dest_dir/$BIN_NAME"

    case ":$PATH:" in
        *:"$dest_dir":*) : ;;
        *)
            info ""
            info "note: $dest_dir is not on your PATH."
            info "add this to your shell rc:  export PATH=\"\$PATH:$dest_dir\""
            ;;
    esac

    info ""
    "$dest_dir/$BIN_NAME" --version || true
}

main
