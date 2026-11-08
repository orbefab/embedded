#!/usr/bin/env bash
# Puts the blobs no example commits into the blob cache, and prints the cache folder.
#
#   packages/engine/scripts/blobs.sh                 fetch what the cache lacks, then print the folder
#   packages/engine/scripts/blobs.sh --from <file>   take a blob from a local file instead of fetching it
#
# The cache is $ORBE_BLOBS, else $XDG_CACHE_HOME/orbe/blobs, else ~/.cache/orbe/blobs.
# Each blob is a file named by its SHA-256. The one such blob is the ESP32-S3 revision 0 ROM ELF
# from Espressif's esp-rom-elfs release 20241011, the archive ESP-IDF v5.5.5 pins in
# tools/tools.json. `--from` takes that archive or the ELF itself, and either must hash to the
# pinned value. Nothing is fetched while the cache holds a good copy.
set -euo pipefail

rom_sha=c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd
rom_name=esp32s3_rev0_rom.elf
archive_url=https://github.com/espressif/esp-rom-elfs/releases/download/20241011/esp-rom-elfs-20241011.tar.gz
archive_sha=921f000164a421c7628fbfee55b173384aafaa51883adc65cd27bf9b0af9e9a9

cache=${ORBE_BLOBS:-${XDG_CACHE_HOME:-$HOME/.cache}/orbe/blobs}
from=
if [[ ${1:-} == --from ]]; then
    from=${2:?--from needs a file}
fi

sha() {
    sha256sum "$1" | cut -d' ' -f1
}

# Files the ELF from archive or ELF $1 into the cache, or fails.
take() {
    local file=$1 work
    work=$(mktemp -d)
    trap 'rm -rf "$work"' RETURN
    if [[ $(sha "$file") == "$archive_sha" ]]; then
        tar -xzf "$file" -C "$work" "$rom_name"
        file=$work/$rom_name
    fi
    if [[ $(sha "$file") != "$rom_sha" ]]; then
        echo "blobs.sh: $1 is neither the pinned archive nor the pinned ROM ELF" >&2
        return 1
    fi
    cp "$file" "$work/blob"
    mv "$work/blob" "$cache/$rom_sha"
}

mkdir -p "$cache"
if [[ -f $cache/$rom_sha && $(sha "$cache/$rom_sha") == "$rom_sha" ]]; then
    :
elif [[ -n $from ]]; then
    take "$from"
else
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    curl -fsSL --retry 5 --retry-delay 2 -o "$work/archive.tar.gz" "$archive_url"
    take "$work/archive.tar.gz"
fi
echo "$cache"
