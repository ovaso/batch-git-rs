#!/bin/sh
set -eu

repository="ovaso/batch-git-rs"
version=""
prefix="${BATCH_GIT_INSTALL_PREFIX:-${HOME:?HOME is required}/.local}"

usage() {
    printf '%s\n' "Usage: install.sh --version vX.Y.Z [--prefix PATH]"
}

fail() {
    printf 'batch-git installer: %s\n' "$1" >&2
    exit 1
}

while [ "$#" -gt 0 ]; do
    case "$1" in
        --version)
            [ "$#" -ge 2 ] || fail "--version requires a value"
            version=$2
            shift 2
            ;;
        --prefix)
            [ "$#" -ge 2 ] || fail "--prefix requires a value"
            prefix=$2
            shift 2
            ;;
        -h|--help)
            usage
            exit 0
            ;;
        *)
            fail "unknown argument: $1"
            ;;
    esac
done

[ -n "$version" ] || fail "--version is required"
case "$version" in
    v[0-9]*) ;;
    *) fail "version must start with v followed by a digit" ;;
esac
case "$version" in
    *[!A-Za-z0-9._-]*) fail "version contains unsupported characters" ;;
esac

machine=$(uname -m)
case "$(uname -s):$machine" in
    Linux:x86_64|Linux:amd64) target="x86_64-unknown-linux-gnu" ;;
    Darwin:x86_64|Darwin:amd64) target="x86_64-apple-darwin" ;;
    Darwin:arm64|Darwin:aarch64) target="aarch64-apple-darwin" ;;
    *) fail "unsupported platform: $(uname -s) $machine" ;;
esac

archive="batch-git-$target.tar.gz"
base_url="https://github.com/$repository/releases/download/$version"
temporary_directory=$(mktemp -d "${TMPDIR:-/tmp}/batch-git-install.XXXXXX")
trap 'rm -rf "$temporary_directory"' EXIT HUP INT TERM

download() {
    url=$1
    destination=$2
    if command -v curl >/dev/null 2>&1; then
        curl --fail --location --silent --show-error "$url" --output "$destination"
    elif command -v wget >/dev/null 2>&1; then
        wget --quiet --output-document="$destination" "$url"
    else
        fail "curl or wget is required"
    fi
}

download "$base_url/$archive" "$temporary_directory/$archive"
download "$base_url/$archive.sha256" "$temporary_directory/$archive.sha256"
expected=$(awk 'NR == 1 { print $1 }' "$temporary_directory/$archive.sha256")
case "$expected" in
    *[!0-9A-Fa-f]*|'') fail "release checksum is malformed" ;;
esac
[ "${#expected}" -eq 64 ] || fail "release checksum is malformed"
if command -v sha256sum >/dev/null 2>&1; then
    actual=$(sha256sum "$temporary_directory/$archive" | awk '{ print $1 }')
elif command -v shasum >/dev/null 2>&1; then
    actual=$(shasum -a 256 "$temporary_directory/$archive" | awk '{ print $1 }')
else
    fail "sha256sum or shasum is required"
fi
[ "$actual" = "$expected" ] || fail "SHA-256 verification failed"

tar -xzf "$temporary_directory/$archive" -C "$temporary_directory"
package="$temporary_directory/batch-git-$target"
[ -f "$package/batch-git" ] || fail "archive does not contain batch-git"

mkdir -p "$prefix/bin"
install -m 0755 "$package/batch-git" "$prefix/bin/batch-git"
mkdir -p "$prefix/share/bash-completion/completions"
mkdir -p "$prefix/share/zsh/site-functions"
mkdir -p "$prefix/share/fish/vendor_completions.d"
install -m 0644 "$package/completions/batch-git.bash" \
    "$prefix/share/bash-completion/completions/batch-git"
install -m 0644 "$package/completions/_batch-git" \
    "$prefix/share/zsh/site-functions/_batch-git"
install -m 0644 "$package/completions/batch-git.fish" \
    "$prefix/share/fish/vendor_completions.d/batch-git.fish"

printf 'installed batch-git %s to %s/bin/batch-git\n' "$version" "$prefix"
printf 'ensure %s/bin is on PATH\n' "$prefix"
