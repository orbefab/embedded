#!/bin/sh
# Installs a prebuilt orbe, with the viewer built in, into a directory on this machine.
# It picks the archive for this OS and arch, downloads it from a GitHub release, checks
# the sha256, and copies the orbe binary. It never edits shell rc files.
#
# Usage:
#   install.sh [--dir DIR] [--version VERSION] [--base URL] [--tarball FILE]
#
# VERSION is latest, the default, or a version such as 0.1.0 or v0.1.0.
# DIR is where the binary goes. The default is ~/.local/bin.
# --base URL downloads from that directory instead of the GitHub release. It needs a
# version. The directory holds orbe-<version>-<target>.tar.gz and the .sha256 beside it.
# A local server can be that directory.
# --tarball FILE installs that file and checks FILE.sha256. It does not download.
#
# The release targets are Linux x86_64 and macOS arm64. Anything else is refused.
#
# Everything runs inside main, called on the last line, so a download cut short
# through `curl | sh` defines main and runs nothing.
set -eu

main() {

repo=https://github.com/orbefab/embedded

die() {
  echo "install.sh: $*" >&2
  exit 1
}

usage() {
  cat <<'EOF'
usage: install.sh [--dir DIR] [--version VERSION] [--base URL] [--tarball FILE]

Install orbe into DIR. The default DIR is ~/.local/bin. VERSION is latest, or a
version such as 0.1.0. The archive is the one for this machine: Linux x86_64 or
macOS arm64.

--base URL     Download from URL instead of the GitHub release. URL is the
               directory of orbe-<version>-<target>.tar.gz and its .sha256 file.
               Pass a version with it.
--tarball FILE Install FILE and check FILE.sha256. Do not download.

When DIR is not on PATH, the script says so. It does not edit shell rc files.
EOF
}

usage_error() {
  echo "install.sh: $*" >&2
  usage >&2
  exit 2
}

dir=
version=
base=
tarball=
while [ $# -gt 0 ]; do
  case $1 in
    --dir)
      [ $# -ge 2 ] || usage_error "--dir needs a directory"
      dir=$2
      shift 2
      ;;
    --version)
      [ $# -ge 2 ] || usage_error "--version needs a version"
      [ -z "$version" ] || usage_error "version given twice"
      version=$2
      shift 2
      ;;
    --base)
      [ $# -ge 2 ] || usage_error "--base needs a URL"
      base=$2
      shift 2
      ;;
    --tarball)
      [ $# -ge 2 ] || usage_error "--tarball needs a file"
      tarball=$2
      shift 2
      ;;
    -h | --help)
      usage
      exit 0
      ;;
    --)
      shift
      break
      ;;
    -*)
      usage_error "unknown option $1"
      ;;
    *)
      [ -z "$version" ] || usage_error "unexpected argument $1"
      version=$1
      shift
      ;;
  esac
done
[ $# -eq 0 ] || usage_error "unexpected argument $1"
if [ -n "$tarball" ] && [ -n "$base" ]; then
  usage_error "pass --base or --tarball, not both"
fi

if [ -z "$dir" ]; then
  [ -n "${HOME-}" ] || die "HOME is not set, so pass --dir"
  dir=$HOME/.local/bin
fi
[ -n "$version" ] || version=latest

os=$(uname -s)
arch=$(uname -m)
case "$os:$arch" in
  Linux:x86_64)
    target=x86_64-unknown-linux-gnu
    ;;
  Darwin:arm64 | Darwin:aarch64)
    target=aarch64-apple-darwin
    ;;
  Linux:aarch64 | Linux:arm64)
    die "Linux arm64 is not supported. orbe publishes Linux x86_64 and macOS arm64."
    ;;
  Darwin:x86_64)
    die "macOS Intel is not supported. orbe publishes Linux x86_64 and macOS arm64."
    ;;
  *)
    die "this machine is $os $arch, which is not supported. orbe publishes Linux x86_64 and macOS arm64."
    ;;
esac

normalize_version() {
  case $version in
    v*) version=${version#v} ;;
  esac
  case $version in
    "" | latest)
      die "pass a version such as 0.1.0"
      ;;
    *[!0-9A-Za-z._-]*)
      die "version $version has a character a release name cannot use"
      ;;
    *..* | .*)
      die "version $version is not a release version"
      ;;
  esac
}

resolve_latest() {
  [ -z "$base" ] || die "--base needs a version, such as --version 0.1.0"
  command -v curl >/dev/null 2>&1 || die "need curl to find the latest release"
  latest_url=$repo/releases/latest
  effective=$(curl -fsSL --connect-timeout 15 --retry 3 --retry-delay 2 --speed-limit 1 --speed-time 30 -o /dev/null -w '%{url_effective}' "$latest_url") || die "could not read $latest_url"
  tag=${effective##*/}
  case $tag in
    v[0-9]*) version=${tag#v} ;;
    *) die "the latest release did not end in a version tag: $effective" ;;
  esac
  normalize_version
}

if [ -z "$tarball" ] && [ "$version" = latest ]; then
  resolve_latest
elif [ "$version" != latest ]; then
  normalize_version
fi

hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1" | awk 'NR==1 { print $1 }'
  elif command -v shasum >/dev/null 2>&1; then
    shasum -a 256 "$1" | awk 'NR==1 { print $1 }'
  else
    die "need sha256sum or shasum to check the archive"
  fi
}

fetch() {
  url=$1
  dest=$2
  if command -v curl >/dev/null 2>&1; then
    curl -fsSL --connect-timeout 15 --retry 3 --retry-delay 2 --speed-limit 1 --speed-time 30 -o "$dest" "$url" || return 1
  elif command -v wget >/dev/null 2>&1; then
    wget -q --timeout=15 --tries=3 -O "$dest" "$url" || return 1
  else
    die "need curl or wget to download $url"
  fi
}

command -v mktemp >/dev/null 2>&1 || die "need mktemp"
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

if [ -n "$tarball" ]; then
  [ -f "$tarball" ] || die "no archive at $tarball"
  sumfile=$tarball.sha256
  [ -f "$sumfile" ] || die "no sha256 file at $sumfile"
  archive=$tarball
else
  [ "$version" != latest ] || die "pass a version such as 0.1.0"
  name=orbe-$version-$target.tar.gz
  if [ -z "$base" ]; then
    base=$repo/releases/download/v$version
  fi
  base=${base%/}
  archive=$work/$name
  if ! fetch "$base/$name" "$archive"; then
    rm -f "$archive"
    die "could not download $base/$name"
  fi
  sumfile=$work/$name.sha256
  if ! fetch "$base/$name.sha256" "$sumfile"; then
    rm -f "$sumfile"
    die "could not download $base/$name.sha256"
  fi
fi

want=$(awk 'NR==1 { print $1 }' "$sumfile" | tr 'A-F' 'a-f')
case $want in
  *[!0-9a-f]*) die "the checksum in $sumfile is not hex" ;;
esac
[ "${#want}" -eq 64 ] || die "the checksum in $sumfile is not 64 hex digits"
got=$(hash_file "$archive" | tr 'A-F' 'a-f')
[ "$want" = "$got" ] || die "sha256 mismatch: the archive has $got and $sumfile says $want"

tar -tzf "$archive" >"$work/list" || die "could not list the archive"
while IFS= read -r entry; do
  case $entry in
    "") continue ;;
    /*) die "archive entry $entry starts at the root" ;;
    *..*) die "archive entry $entry leaves its folder" ;;
  esac
done <"$work/list"
mkdir -p "$work/root"
tar -xzf "$archive" -C "$work/root" || die "could not extract the archive"
[ -f "$work/root/orbe" ] || die "the archive has no orbe binary at its top"

mkdir -p "$dir" || die "could not create $dir"
[ -d "$dir" ] || die "$dir is not a directory"
cp "$work/root/orbe" "$dir/orbe" || die "could not copy orbe into $dir"
chmod 755 "$dir/orbe" || die "could not mark $dir/orbe executable"

here=$PWD
cd "$dir" || die "could not open $dir"
abs=$(pwd -P)
cd "$here" || die "could not return to $here"

installed=$("$dir/orbe" --version) || die "the installed orbe did not run"
case $installed in
  "orbe "[0-9]*) ;;
  *) die "the installed binary printed '$installed', not a version" ;;
esac
if [ "$version" != latest ]; then
  case $installed in
    "orbe $version" | "orbe $version "*) ;;
    *) die "the binary reports '$installed', and this release is $version" ;;
  esac
fi

echo "install.sh: installed $installed ($target) to $abs/orbe"
on_path=0
rest=${PATH-}
while [ -n "$rest" ]; do
  part=${rest%%:*}
  case $rest in
    *:*) rest=${rest#*:} ;;
    *) rest= ;;
  esac
  [ -n "$part" ] || part=.
  if [ "$part" = "$dir" ] || [ "$part" = "$abs" ]; then
    on_path=1
  fi
done
if [ "$on_path" -eq 0 ]; then
  echo "install.sh: $abs is not on PATH. Add it yourself. This script does not edit your shell rc files."
fi
}

main "$@"
