import argparse
import pathlib
import plistlib
import shutil
import subprocess
import tempfile
import tomllib
import zipfile


def package(target):
    root = pathlib.Path(__file__).resolve().parent.parent
    version = tomllib.loads((root / "desktop/Cargo.toml").read_text())["package"]["version"]
    windows = "windows" in target
    macos = "apple-darwin" in target
    executable = "raindrop-desktop.exe" if windows else "raindrop-desktop"
    binary = root / "desktop/target" / target / "release" / executable
    if not binary.is_file():
        raise FileNotFoundError(binary)
    output = root / "dist"
    output.mkdir(exist_ok=True)
    name = f"raindrop-desktop-{version}-{target}"
    with tempfile.TemporaryDirectory() as directory:
        staging = pathlib.Path(directory) / name
        staging.mkdir()
        if macos:
            contents = staging / "Raindrop Desktop.app/Contents"
            (contents / "MacOS").mkdir(parents=True)
            (contents / "Resources").mkdir()
            shutil.copy2(binary, contents / "MacOS/raindrop-desktop")
            with (contents / "Info.plist").open("wb") as destination:
                plistlib.dump({
                    "CFBundleName": "Raindrop Desktop",
                    "CFBundleDisplayName": "Raindrop Desktop",
                    "CFBundleIdentifier": "io.github.ca-x.raindrop-desktop",
                    "CFBundleExecutable": "raindrop-desktop",
                    "CFBundlePackageType": "APPL",
                    "CFBundleShortVersionString": version,
                    "CFBundleVersion": version,
                    "CFBundleIconFile": "raindrop.icns",
                    "LSMinimumSystemVersion": "14.0",
                    "NSHighResolutionCapable": True,
                }, destination)
            iconset = pathlib.Path(directory) / "raindrop.iconset"
            iconset.mkdir()
            for size in [16, 32, 128, 256, 512]:
                for scale in [1, 2]:
                    pixels = size * scale
                    suffix = "@2x" if scale == 2 else ""
                    subprocess.run(["sips", "-z", str(pixels), str(pixels), str(root / "web/public/brand/raindrop-logo-512.png"), "--out", str(iconset / f"icon_{size}x{size}{suffix}.png")], check=True, capture_output=True)
            subprocess.run(["iconutil", "-c", "icns", str(iconset), "-o", str(contents / "Resources/raindrop.icns")], check=True)
        else:
            shutil.copy2(binary, staging / executable)
            shutil.copy2(root / "web/public/brand/raindrop-logo-512.png", staging / "raindrop-desktop.png")
            if not windows:
                (staging / "raindrop-desktop.desktop").write_text("[Desktop Entry]\nType=Application\nName=Raindrop Desktop\nExec=raindrop-desktop\nIcon=raindrop-desktop\nTerminal=false\nCategories=Network;News;\n", encoding="utf-8")
        readme = (root / "desktop/README.md").read_text(encoding="utf-8")
        readme = readme.replace("../docs/", "docs/").replace("../README.md#界面预览", "https://github.com/ca-x/raindrop#界面预览")
        (staging / "README.md").write_text(readme, encoding="utf-8")
        (staging / "docs/assets/screenshots").mkdir(parents=True)
        shutil.copy2(root / "docs/desktop-storage.md", staging / "docs/desktop-storage.md")
        for screenshot in (root / "docs/assets/screenshots").glob("native-desktop-*.png"):
            shutil.copy2(screenshot, staging / "docs/assets/screenshots" / screenshot.name)
        shutil.copy2(root / "LICENSE", staging / "LICENSE")
        shutil.copy2(root / "vendor/THIRD_PARTY_NOTICES", staging / "THIRD_PARTY_NOTICES")
        with zipfile.ZipFile(output / f"{name}.zip", "w", zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(staging.rglob("*")):
                archive.write(path, path.relative_to(staging.parent))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("--target", required=True, choices=["x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "x86_64-apple-darwin", "aarch64-apple-darwin", "x86_64-pc-windows-msvc"])
    package(parser.parse_args().target)
