#!/usr/bin/env bash
set -euo pipefail

# Extract the Arch packages into the probe directory without changing the host installation.
probe_output="$1"
mkdir -p "$probe_output"
probe_output="$(cd "$probe_output" && pwd)"
mapfile -t packages < <(pacman -Sp --print-format %l extra/weston)
for package in "${packages[@]}"; do
    case "$package" in https://*) ;; *) echo "Unexpected package URL: $package" >&2; exit 1 ;; esac
    archive="$probe_output/${package##*/}"
    curl --fail --location --output "$archive" "$package"
    bsdtar -xf "$archive" -C "$probe_output" usr
done
LD_LIBRARY_PATH="$probe_output/usr/lib:$probe_output/usr/lib/weston${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}" "$probe_output/usr/bin/weston" --version
