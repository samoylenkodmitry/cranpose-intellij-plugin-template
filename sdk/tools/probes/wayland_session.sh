#!/usr/bin/env bash
set -euo pipefail

probe_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
probe_output="$1"
probe_java_home="$2"
weston_root="$3"
mkdir -p "$probe_output/runtime"
probe_output="$(cd "$probe_output" && pwd)"
weston_root="$(cd "$weston_root" && pwd)"
export XDG_RUNTIME_DIR="$probe_output/runtime"
chmod 700 "$XDG_RUNTIME_DIR"
export WAYLAND_DISPLAY=cranpose-gpu-probe
export LD_LIBRARY_PATH="$weston_root/usr/lib:$weston_root/usr/lib/weston${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
export WESTON_MODULE_MAP="headless-backend.so=$weston_root/usr/lib/libweston-15/headless-backend.so;gl-renderer.so=$weston_root/usr/lib/libweston-15/gl-renderer.so;kiosk-shell.so=$weston_root/usr/lib/weston/kiosk-shell.so"
"$weston_root/usr/bin/weston" --backend=headless --renderer=gl --shell=kiosk-shell.so --no-config \
    --socket="$WAYLAND_DISPLAY" --idle-time=0 --width=800 --height=600 --fake-seat --debug \
    >"$probe_output/weston.log" 2>&1 &
compositor_pid=$!
cleanup() {
    kill "$compositor_pid" 2>/dev/null || true
    wait "$compositor_pid" 2>/dev/null || true
}
trap cleanup EXIT
for attempt in {1..50}; do
    if [[ -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then break; fi
    if ! kill -0 "$compositor_pid" 2>/dev/null; then cat "$probe_output/weston.log"; exit 1; fi
    sleep 0.1
done
if [[ ! -S "$XDG_RUNTIME_DIR/$WAYLAND_DISPLAY" ]]; then cat "$probe_output/weston.log"; exit 1; fi
python3 "$probe_dir/native_layer.py" --java-home "$probe_java_home" --output "$probe_output" --toolkit wayland
