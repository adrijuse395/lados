#!/usr/bin/env python3
"""Assemble the Lados browser extension for each store.

The shared sources live in `browser-extension/`; only the manifest differs:
  - Firefox (MV2):      browser-extension/manifest.json
  - Chrome/Edge (MV3):  browser-extension/manifest.chrome.json

Usage:  python3 scripts/build-extension.py

Output (all under browser-extension/build/, git-ignored):
  build/firefox/        ready to "Load Temporary Add-on" in Firefox
  build/chrome/         ready to "Load unpacked" in Chrome/Edge
  build/lados-firefox.zip
  build/lados-chrome.zip
"""
import os
import shutil
import zipfile

ROOT = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
SRC = os.path.join(ROOT, "browser-extension")
OUT = os.path.join(SRC, "build")
SHARED = ["background.js", "content.js", "popup.html", "popup.js"]

TARGETS = {
    "firefox": "manifest.json",
    "chrome": "manifest.chrome.json",
}


def assemble(name: str, manifest: str) -> str:
    dest = os.path.join(OUT, name)
    shutil.rmtree(dest, ignore_errors=True)
    os.makedirs(dest)
    for file in SHARED:
        shutil.copy(os.path.join(SRC, file), dest)
    shutil.copytree(os.path.join(SRC, "icons"), os.path.join(dest, "icons"))
    shutil.copy(os.path.join(SRC, manifest), os.path.join(dest, "manifest.json"))
    return dest


def zip_dir(folder: str, zip_path: str) -> None:
    with zipfile.ZipFile(zip_path, "w", zipfile.ZIP_DEFLATED) as archive:
        for root, _dirs, files in os.walk(folder):
            for file in files:
                full = os.path.join(root, file)
                rel = os.path.relpath(full, folder).replace("\\", "/")
                archive.write(full, rel)


def main() -> None:
    os.makedirs(OUT, exist_ok=True)
    for name, manifest in TARGETS.items():
        folder = assemble(name, manifest)
        zip_path = os.path.join(OUT, f"lados-{name}.zip")
        zip_dir(folder, zip_path)
        print(f"{name}: {folder}  ->  {zip_path}")


if __name__ == "__main__":
    main()
