#!/usr/bin/env python3
"""Build and exercise a native GPU layer inside an AWT window on Windows or Linux."""

import argparse
import json
from pathlib import Path
import platform
import shlex
import subprocess
import sys


def run(command, timeout=90):
    result = subprocess.run(command, text=True, capture_output=True, timeout=timeout)
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    result.check_returncode()
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--java-home", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--toolkit", choices=["x11", "wayland"])
    args = parser.parse_args()
    source = Path(__file__).resolve().parent
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    java_home = args.java_home.resolve()
    windows = sys.platform == "win32"
    library = output / ("native_layer_probe.dll" if windows else "libnative_layer_probe.so")
    include = java_home / "include"
    if windows:
        run(["cl", "/nologo", "/std:c++17", "/EHsc", "/LD", "/I" + str(include),
             "/I" + str(include / "win32"), str(source / "NativeLayerProbe.cpp"),
             "/Fo" + str(output / "native_layer_probe.obj"), "/link", "/OUT:" + str(library),
             "/LIBPATH:" + str(java_home / "lib"), "jawt.lib", "d3d11.lib", "dxgi.lib", "dcomp.lib"])
    else:
        flags = shlex.split(run(["pkg-config", "--cflags", "--libs", "x11", "xfixes", "xrender", "egl", "glesv2", "wayland-client", "wayland-egl"]))
        run(["c++", "-std=c++17", "-shared", "-fPIC", "-Wall", "-Wextra", "-Wno-unused-parameter",
             "-I" + str(include), "-I" + str(include / "linux"), str(source / "NativeLayerProbe.cpp"),
             "-L" + str(java_home / "lib"), "-Wl,-rpath," + str(java_home / "lib"),
             "-ljawt", *flags, "-o", str(library)])
    command = [str(java_home / "bin" / ("java.exe" if windows else "java")),
               "--enable-native-access=ALL-UNNAMED"]
    if args.toolkit:
        command.append("-Dawt.toolkit.name=" + ("WLToolkit" if args.toolkit == "wayland" else "XToolkit"))
    report = {"platform": platform.platform(), "javaHome": str(java_home), "toolkit": args.toolkit}
    try:
        report["result"] = run(command + [str(source / "NativeLayerProbe.java"), str(library)])
        report["passed"] = True
    except Exception as failure:
        report["passed"] = False
        report["error"] = str(failure)
        if isinstance(failure, subprocess.CalledProcessError):
            report["stdout"] = failure.stdout
            report["stderr"] = failure.stderr
        raise
    finally:
        (output / "native-layer.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
