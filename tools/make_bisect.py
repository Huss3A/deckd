"""Builds captures/bisect1.zip from the stock app's layout, swapping in pieces of ours cell by cell.

  0_0  stock image, unchanged                      (control: must show)
  1_0  stock image + Text "Hello"                  (does Text break icons?)
  2_0  OUR pencil PNG, stock-style uuid filename   (is our PNG content ok?)
  0_1  OUR sigma PNG, plain filename "Images/sigma.png"  (does the filename matter?)
  1_1  OUR scissors PNG, uuid filename + Text "Mine"
"""
import json, zipfile, uuid

STOCK = r"D:\My Deck App\captures\zip_1_cmd0001.zip"
OURS = r"D:\My Deck App\captures\layout_glyph2.zip"
OUT = r"D:\My Deck App\captures\bisect1.zip"

stock = zipfile.ZipFile(STOCK)
ours = zipfile.ZipFile(OURS)
manifest = json.loads(stock.read("manifest.json"))
files = {n: stock.read(n) for n in stock.namelist() if n.startswith("Images/") and not n.endswith("/")}

def swap(cell, png_name, filename, text=""):
    files[filename] = ours.read(png_name)
    vp = manifest[cell]["ViewParam"][0]
    vp["Icon"] = filename
    vp["Text"] = text

manifest["1_0"]["ViewParam"][0]["Text"] = "Hello"
swap("2_0", "Images/0_0.png", f"Images/{uuid.uuid4()}.png")
swap("0_1", "Images/1_0.png", "Images/sigma.png")
swap("1_1", "Images/2_0.png", f"Images/{uuid.uuid4()}.png", "Mine")

with zipfile.ZipFile(OUT, "w", zipfile.ZIP_DEFLATED) as z:
    d = zipfile.ZipInfo("Images/")
    d.external_attr = (0o40777 << 16) | 0x10
    z.writestr(d, b"")
    used = {v["ViewParam"][0].get("Icon") for v in manifest.values()}
    for name, data in files.items():
        if name in used:
            z.writestr(name, data)
    z.writestr("manifest.json", json.dumps(manifest, separators=(",", ":")))
print("wrote", OUT, "entries:", len(zipfile.ZipFile(OUT).namelist()))
