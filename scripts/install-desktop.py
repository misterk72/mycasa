#!/usr/bin/env python3
"""Install the MyCasa launcher and icon for the current Linux user."""
import os
from pathlib import Path
import shutil
import subprocess
import sys

root = Path(__file__).resolve().parent.parent
binary = Path(sys.argv[1]).resolve() if len(sys.argv) > 1 else root / "target/release/mycasa"
if not binary.is_file() or not os.access(binary, os.X_OK):
    sys.exit(f"Executable missing: {binary}. Run cargo build --release first.")
data = Path(os.environ.get("XDG_DATA_HOME") or Path.home() / ".local/share")
if not data.is_absolute():
    sys.exit("XDG_DATA_HOME must be an absolute path")
applications = data / "applications"
icons = data / "icons"
applications.mkdir(parents=True, exist_ok=True)
icons.mkdir(parents=True, exist_ok=True)
shutil.copyfile(root / "assets/mycasa.png", icons / "mycasa.png")
# Escape both the Exec quoting layer and the desktop-entry string layer.
executable = str(binary).replace("%", "%%")
for char in ('\\', '"', '`', '$'):
    executable = executable.replace(char, '\\' + char)
executable = executable.replace('\\', '\\\\').replace('\n', '\\n').replace('\r', '\\r')
icon_path = str(icons / "mycasa.png").replace("\\", "\\\\").replace("\n", "\\n").replace("\r", "\\r")
launcher = applications / "mycasa.desktop"
launcher.write_text(f'''[Desktop Entry]
Type=Application
Name=MyCasa
Comment=Photo library and viewer
Comment[fr]=Photothèque et visionneuse
Exec="{executable}"
Icon={icon_path}
Terminal=false
Categories=Graphics;Photography;Viewer;
StartupWMClass=mycasa
StartupNotify=true
''')
if shutil.which("update-desktop-database"):
    subprocess.run(["update-desktop-database", str(applications)], check=True)
print(f"Installed {launcher}")
