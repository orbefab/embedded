#!/usr/bin/env bash
# Puts the ESP32-S3 ROM into the blob cache, and prints the cache folder.
#
#   packages/engine/scripts/blobs.sh                 fill what the cache lacks, then print the folder
#   packages/engine/scripts/blobs.sh --from <file>   take a blob from a local file instead of the committed copy
#   packages/engine/scripts/blobs.sh --induction     also fetch Wireshark's wpa-Induction.pcap
#
# The cache is $ORBE_BLOBS, else $XDG_CACHE_HOME/orbe/blobs, else ~/.cache/orbe/blobs.
# Each blob is a file named by its SHA-256. The ROM is the ESP32-S3 revision 0 ROM ELF
# from Espressif's esp-rom-elfs release 20241011, the archive ESP-IDF v5.5.5 pins in
# tools/tools.json. The catalog part parts/esp32s3-qfn56 commits that ELF. When the cache lacks
# a good copy, this script copies the committed file. It fetches the release only when that
# file is absent or does not hash to the pinned value. `--from` takes the archive or the ELF itself instead, and either must hash
# to the pinned value. Nothing is fetched while the cache holds a good copy. The release URL is
# tried first. If that request fails, the same asset is fetched from the GitHub API.
#
# `--induction` also fetches wpa-Induction.pcap from Wireshark's sample captures
# (https://wiki.wireshark.org/SampleCaptures), the WPA2-PSK handshake that orbe-net's replay
# test reads. Its page states no licence, so the repo does not commit it. It is fetched only when
# the cache lacks a copy that hashes to the pinned value. The full tier of scripts/check.sh asks
# for it.
set -euo pipefail

rom_sha=c0ce0f338d1de1bdc6efbef1591779a2a42c1ab7d759d3c6ae8ae63a7dd34cfd
rom_name=esp32s3_rev0_rom.elf
archive_url=https://github.com/espressif/esp-rom-elfs/releases/download/20241011/esp-rom-elfs-20241011.tar.gz
archive_api=https://api.github.com/repos/espressif/esp-rom-elfs/releases/assets/198395584
archive_sha=921f000164a421c7628fbfee55b173384aafaa51883adc65cd27bf9b0af9e9a9
induction_url=https://wiki.wireshark.org/uploads/__moin_import__/attachments/SampleCaptures/wpa-Induction.pcap
induction_sha=2b57dca7fa2c3bd0e942060b546028d961bfb698fb12ed8b2947b13f88d170c8

cache=${ORBE_BLOBS:-${XDG_CACHE_HOME:-$HOME/.cache}/orbe/blobs}
root=$(cd "$(dirname "$0")/../../.." && pwd)
committed=$root/parts/esp32s3-qfn56/esp32s3_rev0_rom.elf
from=
induction=
case ${1:-} in
    --from) from=${2:?--from needs a file} ;;
    --induction) induction=1 ;;
esac

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
elif [[ -f $committed && $(sha "$committed") == "$rom_sha" ]]; then
    take "$committed"
else
    work=$(mktemp -d)
    trap 'rm -rf "$work"' EXIT
    if ! curl -fsSL --retry 5 --retry-delay 2 -o "$work/archive.tar.gz" "$archive_url"; then
        curl -fsSL --retry 5 --retry-delay 2 \
            -H 'Accept: application/octet-stream' -H 'User-Agent: orbe-blobs' \
            -o "$work/archive.tar.gz" "$archive_api"
    fi
    take "$work/archive.tar.gz"
fi
# Fetches the Induction capture into the cache, or fails.
fetch_induction() {
    local work
    work=$(mktemp -d)
    trap 'rm -rf "$work"' RETURN
    curl -fsSL --retry 5 --retry-delay 2 -o "$work/capture" "$induction_url"
    if [[ $(sha "$work/capture") != "$induction_sha" ]]; then
        echo "blobs.sh: $induction_url does not hash to the pinned value" >&2
        return 1
    fi
    mv "$work/capture" "$cache/$induction_sha"
}

if [[ -n $induction ]] && ! [[ -f $cache/$induction_sha && $(sha "$cache/$induction_sha") == "$induction_sha" ]]; then
    fetch_induction
fi
echo "$cache"
