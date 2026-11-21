#!/usr/bin/env bash
# Installs the prebuilt wasm-bindgen CLI at the version Cargo.lock holds, where
# scripts/wasm.sh looks: .scratch/wasm/tools/wasm-bindgen-<version>/bin/wasm-bindgen.
# It does nothing when that binary is already the right version. WASM_TOOLS_DIR
# overrides the tools directory, as it does for scripts/wasm.sh.
# Usage: scripts/install-wasm-bindgen.sh
set -euo pipefail
cd "$(dirname "$0")/.."

if [[ $# -gt 0 ]]; then
  echo "usage: scripts/install-wasm-bindgen.sh" >&2
  exit 2
fi

version=$(awk '$0 == "name = \"wasm-bindgen\"" { getline; gsub(/version = |"/, ""); print; exit }' Cargo.lock)
if [[ ! $version =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]]; then
  echo "install-wasm-bindgen: Cargo.lock has no wasm-bindgen version" >&2
  exit 1
fi

tools=${WASM_TOOLS_DIR:-$PWD/.scratch/wasm/tools}
[[ $tools == /* ]] || tools=$PWD/$tools
root=$tools/wasm-bindgen-$version
bindgen=$root/bin/wasm-bindgen
if [[ -x $bindgen ]] && [[ $($bindgen --version | awk '{ print $2 }') == "$version" ]]; then
  exit 0
fi

case $(uname -s)-$(uname -m) in
  Linux-x86_64) triple=x86_64-unknown-linux-musl ;;
  Linux-aarch64) triple=aarch64-unknown-linux-musl ;;
  Darwin-arm64) triple=aarch64-apple-darwin ;;
  Darwin-x86_64) triple=x86_64-apple-darwin ;;
  *)
    echo "install-wasm-bindgen: no prebuilt wasm-bindgen for $(uname -s)-$(uname -m). Install it with:" >&2
    echo "  cargo install wasm-bindgen-cli --version =$version --locked --root $root" >&2
    exit 2
    ;;
esac

asset=wasm-bindgen-$version-$triple.tar.gz
url=https://github.com/rustwasm/wasm-bindgen/releases/download/$version/$asset
# SHA-256 of the published Linux x86_64 tarball. CI is that platform.
sum=
case $version-$triple in
  0.2.129-x86_64-unknown-linux-musl) sum=82d12bb940e2d4e72e0d5605387fc1b8ca179044e012b620f0ce4e7440e8320e ;;
esac
if [[ -z $sum && $triple == x86_64-unknown-linux-musl ]]; then
  echo "install-wasm-bindgen: no SHA-256 pinned for $asset" >&2
  exit 1
fi
work=$(mktemp -d "${TMPDIR:-/tmp}/wasm-bindgen.XXXXXX")
trap 'rm -rf "$work"' EXIT
curl --proto '=https' --tlsv1.2 -fsSL "$url" -o "$work/$asset"
if [[ -n $sum ]]; then
  echo "$sum  $work/$asset" | sha256sum -c -
fi
tar -xzf "$work/$asset" -C "$work"
mkdir -p "$root/bin"
install -m 755 "$work/wasm-bindgen-$version-$triple/wasm-bindgen" "$bindgen"
"$bindgen" --version >/dev/null
