#!/usr/bin/env python3
"""Install an icon theme for lntrn-code from a JetBrains "Atom Material
Icons" plugin zip or its a-file-icon-idea-*.jar, into
~/.lantern/icons/atom-material/ (per machine, like the fonts):

    scripts/fetch-icons.py ~/Downloads/Atom_Material_Icons-210.0.2

The theme is MIT licensed (Elior "Mallowigi" Boukhobza); its notice is
kept beside the icons."""
import io, pathlib, sys, zipfile

src = pathlib.Path(sys.argv[1]).expanduser() if len(sys.argv) > 1 else None
if src is None:
    sys.exit(__doc__)
jar = None
if src.is_dir():
    jars = list(src.rglob("a-file-icon-idea-*.jar"))
    if not jars:
        sys.exit(f"no a-file-icon-idea-*.jar under {src}")
    jar = zipfile.ZipFile(jars[0])
elif src.suffix == ".jar":
    jar = zipfile.ZipFile(src)
else:
    outer = zipfile.ZipFile(src)
    inner = [n for n in outer.namelist() if n.endswith(".jar") and "a-file-icon-idea" in n]
    if not inner:
        sys.exit("no a-file-icon-idea jar in the zip")
    jar = zipfile.ZipFile(io.BytesIO(outer.read(inner[0])))

out = pathlib.Path.home() / ".lantern" / "icons" / "atom-material"
(out / "files").mkdir(parents=True, exist_ok=True)
(out / "folders").mkdir(parents=True, exist_ok=True)
n = 0
for name in jar.namelist():
    if name.startswith("assets/icons/files/") and name.endswith(".svg"):
        (out / "files" / name.rsplit("/", 1)[1]).write_bytes(jar.read(name)); n += 1
    elif name.startswith("assets/icons/folders/") and name.endswith(".svg"):
        (out / "folders" / name.rsplit("/", 1)[1]).write_bytes(jar.read(name)); n += 1
for table in ("icon_associations.xml", "folder_associations.xml"):
    (out / table).write_bytes(jar.read(f"iconGenerator/{table}"))
lic = jar.read("iconGenerator/icon_associations.xml").decode().split("-->")[0]
(out / "LICENSE.txt").write_text(lic.replace("<!--", "").replace("  ~ ", "").replace("  ~", "").strip() + "\n")
print(f"installed {n} icons + 2 tables to {out}")
