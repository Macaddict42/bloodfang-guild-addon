"""PNG (any size, square or not) -> 64x64 32-bit uncompressed TGA for ## IconTexture. Needs Pillow."""
import sys
from PIL import Image

src, dst = sys.argv[1], sys.argv[2]
im = Image.open(src).convert("RGBA")
w, h = im.size
side = max(w, h)
canvas = Image.new("RGBA", (side, side), (0, 0, 0, 0))  # pad to square, centred, transparent
canvas.paste(im, ((side - w) // 2, (side - h) // 2))
canvas.resize((64, 64), Image.LANCZOS).save(dst, format="TGA", rle=False)
chk = Image.open(dst)
assert chk.size == (64, 64) and chk.mode == "RGBA", (chk.size, chk.mode)
print(f"icon: {dst} 64x64 RGBA TGA from {w}x{h}")
