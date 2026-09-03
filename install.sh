#!/bin/sh

set -eu

repository=${TERMI_REPOSITORY:-tuna4ll/termi}
version=${TERMI_VERSION:-}
install_dir=${TERMI_INSTALL_DIR:-"${HOME:?HOME is not set}/.local/bin"}

fail() {
    printf 'termi: %s\n' "$1" >&2
    exit 1
}

command -v curl >/dev/null 2>&1 || fail "curl is required"
command -v tar >/dev/null 2>&1 || fail "tar is required"

if [ -z "$version" ]; then
    release_url=$(curl -fsSL -o /dev/null -w '%{url_effective}' \
        "https://github.com/$repository/releases/latest")
    version=${release_url##*/}
fi

case "$(uname -s)" in
    Linux)
        case "$(uname -m)" in
            x86_64 | amd64) target=x86_64-unknown-linux-musl ;;
            *) fail "Linux architecture $(uname -m) is not supported" ;;
        esac
        ;;
    Darwin)
        case "$(uname -m)" in
            arm64 | aarch64) target=aarch64-apple-darwin ;;
            x86_64 | amd64) target=x86_64-apple-darwin ;;
            *) fail "macOS architecture $(uname -m) is not supported" ;;
        esac
        ;;
    *) fail "operating system $(uname -s) is not supported" ;;
esac

archive="termi-$version-$target.tar.gz"
download_root=${TERMI_DOWNLOAD_ROOT:-"https://github.com/$repository/releases/download/$version"}
temp_dir=$(mktemp -d 2>/dev/null || mktemp -d -t termi)
trap 'rm -rf "$temp_dir"' EXIT HUP INT TERM

curl -fL --progress-bar "$download_root/$archive" -o "$temp_dir/$archive"
curl -fsSL "$download_root/SHA256SUMS" -o "$temp_dir/SHA256SUMS"

expected=$(awk -v name="$archive" '$2 == name || $2 == "*" name { print $1 }' \
    "$temp_dir/SHA256SUMS")
[ -n "$expected" ] || fail "checksum for $archive is missing"

if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$temp_dir/$archive" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$temp_dir/$archive" | awk '{ print $1 }')
else
    fail "sha256sum or shasum is required"
fi

[ "$actual" = "$expected" ] || fail "checksum verification failed"

tar -xzf "$temp_dir/$archive" -C "$temp_dir"
mkdir -p "$install_dir"
install -m 755 "$temp_dir/termi-$version-$target/termi" "$install_dir/termi"

printf 'termi %s installed to %s/termi\n' "$version" "$install_dir"
case ":${PATH:-}:" in
    *:"$install_dir":*) ;;
    *) printf 'Add %s to PATH to run termi from any directory.\n' "$install_dir" ;;
esac
