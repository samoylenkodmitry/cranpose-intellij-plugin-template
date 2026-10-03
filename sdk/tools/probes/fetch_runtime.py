#!/usr/bin/env python3
"""Fetch the pinned JetBrains SDK used by the native presentation probes."""

import argparse
import hashlib
import os
from pathlib import Path
import platform
import tarfile
import urllib.request

VERSION = "25.0.4.1"
BUILD = "623.69"


def checksum(path):
    with path.open("rb") as contents:
        return hashlib.file_digest(contents, "sha512").hexdigest()


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    target = {"Windows": "windows", "Linux": "linux", "Darwin": "osx"}[platform.system()]
    arch = "aarch64" if platform.machine().lower() in ("arm64", "aarch64") else "x64"
    name = f"jbrsdk-{VERSION}-{target}-{arch}-b{BUILD}.tar.gz"
    url = "https://cache-redirector.jetbrains.com/intellij-jbr/" + name
    args.output.mkdir(parents=True, exist_ok=True)
    archive = args.output / name
    with urllib.request.urlopen(url + ".checksum", timeout=60) as response:
        expected = response.read().decode().split()[0]
    if not archive.exists() or checksum(archive) != expected:
        urllib.request.urlretrieve(url, archive)
    if checksum(archive) != expected:
        raise RuntimeError("JetBrains runtime checksum mismatch")
    with tarfile.open(archive) as contents:
        contents.extractall(args.output, filter="data")
    homes = list(args.output.glob("*/bin/java.exe" if target == "windows" else "*/bin/java"))
    if target == "osx":
        homes = list(args.output.glob("*/Contents/Home/bin/java"))
    if len(homes) != 1:
        raise RuntimeError(f"Expected one runtime, found {homes}")
    home = homes[0].resolve().parent.parent
    print(home)
    if "GITHUB_ENV" in os.environ:
        with open(os.environ["GITHUB_ENV"], "a") as environment:
            environment.write(f"CRANPOSE_PROBE_JAVA_HOME={home}\n")


if __name__ == "__main__":
    main()
