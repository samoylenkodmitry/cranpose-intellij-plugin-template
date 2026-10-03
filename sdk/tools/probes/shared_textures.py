#!/usr/bin/env python3
"""Check an IDE's GPU image interop and validate Metal presentation on macOS."""

import argparse
import json
from pathlib import Path
import platform
import subprocess
import sys
from zipfile import ZipFile


def run(command, timeout=30):
    result = subprocess.run(command, text=True, capture_output=True, timeout=timeout)
    print(result.stdout, end="")
    print(result.stderr, end="", file=sys.stderr)
    result.check_returncode()
    return result.stdout


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--ide", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    home = args.ide.resolve()
    if (home / "Contents").is_dir():
        home /= "Contents"
    runtime = home / "jbr"
    if (runtime / "Contents/Home").is_dir():
        runtime /= "Contents/Home"
    java = runtime / "bin" / ("java.exe" if sys.platform == "win32" else "java")
    api = None
    for jar in (home / "lib").glob("*.jar"):
        with ZipFile(jar) as archive:
            if "com/jetbrains/JBR.class" in archive.namelist():
                api = jar
                break
    if api is None:
        raise RuntimeError("IDE does not bundle the JetBrains Runtime API")
    args.output.mkdir(parents=True, exist_ok=True)
    source = Path(__file__).resolve().parent
    command = [str(java), "--enable-native-access=ALL-UNNAMED", "-cp", str(api)]
    capabilities = run(command + [str(source / "SharedTextureCapabilities.java")])
    report = {"platform": platform.platform(), "ide": str(home), "capabilities": capabilities,
              "presentation": "not supported by this runtime"}
    if "sharedTextures=true" in capabilities and sys.platform == "darwin":
        library = args.output.resolve() / "libshared_texture_probe.dylib"
        run(["clang", "-fobjc-arc", "-dynamiclib", "-framework", "Metal", "-framework", "Foundation",
             "-I", str(runtime / "include"), "-I", str(runtime / "include/darwin"),
             str(source / "SharedTextureProbe.m"), "-o", str(library)])
        report["presentation"] = run(command + [str(source / "SharedTextureProbe.java"), str(library)])
    (args.output / "capabilities.json").write_text(json.dumps(report, indent=2) + "\n")


if __name__ == "__main__":
    main()
