#!/bin/sh

set -eu

repo="${ASH_REPO:-kodin00/ash}"
version="${ASH_VERSION:-latest}"
install_dir="${ASH_INSTALL_DIR:-$HOME/.local/bin}"

case "$(uname -s)" in
    Darwin) os="apple-darwin" ;;
    Linux) os="unknown-linux-musl" ;;
    *)
        printf 'ash installer: unsupported operating system: %s\n' "$(uname -s)" >&2
        exit 1
        ;;
esac

case "$(uname -m)" in
    x86_64 | amd64) arch="x86_64" ;;
    arm64 | aarch64) arch="aarch64" ;;
    *)
        printf 'ash installer: unsupported CPU architecture: %s\n' "$(uname -m)" >&2
        exit 1
        ;;
esac

target="${arch}-${os}"
archive="ash-${target}.tar.gz"

if [ -n "${ASH_DOWNLOAD_BASE_URL:-}" ]; then
    base_url="$ASH_DOWNLOAD_BASE_URL"
elif [ "$version" = "latest" ]; then
    base_url="https://github.com/${repo}/releases/latest/download"
else
    base_url="https://github.com/${repo}/releases/download/${version}"
fi

tmp_dir="$(mktemp -d 2>/dev/null || mktemp -d -t ash-install)"
trap 'rm -rf "$tmp_dir"' EXIT HUP INT TERM

download() {
    url="$1"
    output="$2"
    if command -v curl >/dev/null 2>&1; then
        case "$url" in
            https://*) curl --proto '=https' --tlsv1.2 -LsSf "$url" -o "$output" ;;
            *) curl -LsSf "$url" -o "$output" ;;
        esac
    elif command -v wget >/dev/null 2>&1; then
        wget -q "$url" -O "$output"
    else
        printf 'ash installer: curl or wget is required\n' >&2
        exit 1
    fi
}

printf 'Downloading ash for %s...\n' "$target"
download "${base_url}/${archive}" "${tmp_dir}/${archive}"
download "${base_url}/${archive}.sha256" "${tmp_dir}/${archive}.sha256"

(
    cd "$tmp_dir"
    if command -v sha256sum >/dev/null 2>&1; then
        sha256sum -c "${archive}.sha256"
    elif command -v shasum >/dev/null 2>&1; then
        shasum -a 256 -c "${archive}.sha256"
    else
        printf 'ash installer: sha256sum or shasum is required\n' >&2
        exit 1
    fi
)

tar -xzf "${tmp_dir}/${archive}" -C "$tmp_dir"
mkdir -p "$install_dir"
cp "${tmp_dir}/ash" "${install_dir}/ash"
chmod 755 "${install_dir}/ash"

printf '\nash was installed to %s/ash\n' "$install_dir"
case ":$PATH:" in
    *":${install_dir}:"*) ;;
    *)
        printf '%s is not currently on PATH. Add this to your shell profile:\n' "$install_dir"
        printf '  export PATH="%s:$PATH"\n' "$install_dir"
        ;;
esac
printf 'Run: ash --help\n'
