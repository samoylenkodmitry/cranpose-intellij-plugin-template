#!/usr/bin/env bash
set -euo pipefail

# An isolated display keeps desktop lock screens and unrelated windows out of the probe.
probe_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
probe_output="$1"
probe_java_home="$2"
mkdir -p "$probe_output"
probe_output="$(cd "$probe_output" && pwd)"
display_file="$probe_output/display"
: > "$display_file"
Xvfb -displayfd 3 -screen 0 800x600x24 -nolisten tcp 3>"$display_file" >"$probe_output/xvfb.log" 2>&1 &
display_pid=$!
compositor_pid=""
cleanup() {
    if [[ -n "$compositor_pid" ]]; then kill "$compositor_pid" 2>/dev/null || true; fi
    kill "$display_pid" 2>/dev/null || true
    wait "$display_pid" 2>/dev/null || true
}
trap cleanup EXIT
for attempt in {1..50}; do
    if [[ -s "$display_file" ]]; then break; fi
    if ! kill -0 "$display_pid" 2>/dev/null; then cat "$probe_output/xvfb.log"; exit 1; fi
    sleep 0.1
done
if [[ ! -s "$display_file" ]]; then cat "$probe_output/xvfb.log"; exit 1; fi
export DISPLAY=":$(cat "$display_file")"
picom --backend xrender --config /dev/null >"$probe_output/compositor.log" 2>&1 &
compositor_pid=$!
sleep 0.3
python3 "$probe_dir/native_layer.py" --java-home "$probe_java_home" --output "$probe_output" --toolkit x11
