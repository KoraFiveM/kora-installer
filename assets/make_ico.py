#!/usr/bin/env python3
"""Build assets/kora.ico (PNG-compressed entries) from the Kora brand art.

16/32/48 come from the hand-prepared small renders (ico_16/32/48.png); 64/128/256 are
downscaled from kora-logo-512.png with ffmpeg (lanczos). Python stdlib only.
Usage: python3 assets/make_ico.py   (run from anywhere)
"""
import os, struct, subprocess, tempfile

here = os.path.dirname(os.path.abspath(__file__))
master = os.path.join(here, "kora-logo-512.png")
images = []  # (size, png bytes)
for s in (16, 32, 48):
    images.append((s, open(os.path.join(here, f"ico_{s}.png"), "rb").read()))
with tempfile.TemporaryDirectory() as t:
    for s in (64, 128, 256):
        out = os.path.join(t, f"{s}.png")
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", master, "-vf", f"scale={s}:{s}:flags=lanczos",
                        "-pix_fmt", "rgba", out], check=True)
        images.append((s, open(out, "rb").read()))

data = struct.pack("<HHH", 0, 1, len(images))
offset = 6 + 16 * len(images)
body = b""
for size, png in images:
    dim = 0 if size >= 256 else size
    data += struct.pack("<BBBBHHII", dim, dim, 0, 0, 1, 32, len(png), offset + len(body))
    body += png
open(os.path.join(here, "kora.ico"), "wb").write(data + body)
print("wrote kora.ico with sizes", [s for s, _ in images])
