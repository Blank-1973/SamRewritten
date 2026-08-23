#!/usr/bin/env python3
"""Build assets/installer.ico - the unboxing icon for the NSIS installer.

Run from the repo root:  python3 assets/generate_installer_icon.py
Add --preview PATH to also write a contact sheet of every frame.

256/128/64 carry the illustration; 32 and 16 fall back to the plain app icon,
which is the only thing that stays readable at those sizes. Frames are written
largest-first, PNG at 256 and 32bpp DIB below, matching assets/icon.ico.
"""
from PIL import Image, ImageDraw
import struct, io

SRC   = 'assets/icon_256.png'
DEST  = 'assets/installer.ico'
S     = 8                     # supersample factor
SIZES = [256, 128, 64, 32, 16]
PLAIN_BELOW = 64              # sizes below this use the plain app icon

EDGE     = (142, 92, 48, 255)
TAN_TOP  = (226, 180, 120, 255)
TAN_SIDE = (196, 146, 90, 255)
TAN_FLAP = (212, 164, 104, 255)
TAN_IN   = (150, 105, 63, 255)
MEDAL_BOX = (38, 10, 217, 189)   # the sunburst, without the ribbon tails

# Geometry approved as "a2-light", scaled ~6% to fill the tile, plus Aero-ish lighting.
CFG = dict(rim_y=0.545, hw=0.334, hd=0.106, H=0.2915, flap=0.154,
           art=0.70, art_y=0.045, tilt=-6, stroke=0.013, fx=0.75, fy=0.45,
           grad=9, gloss=26)


def medal():
    src = Image.open(SRC).convert('RGBA').crop(MEDAL_BOX)
    m = Image.new('L', src.size, 0)
    ImageDraw.Draw(m).ellipse([0, 0, src.size[0] - 1, src.size[1] - 1], fill=255)
    src.putalpha(Image.composite(src.getchannel('A'), Image.new('L', src.size, 0), m))
    return src


def render_box(size):
    n = size * S
    im = Image.new('RGBA', (n, n), (0, 0, 0, 0))
    d = ImageDraw.Draw(im)
    lw = max(1, int(CFG['stroke'] * n))
    P = lambda pts: [(x * n, y * n) for x, y in pts]

    def poly(pts, fill, lit=False):
        """Vertical gradient inside the shape, then the outline on top."""
        pp = P(pts)
        xs, ys = [q[0] for q in pp], [q[1] for q in pp]
        x0, y0 = int(min(xs)), int(min(ys))
        w, h = max(1, int(max(xs)) - x0 + 1), max(1, int(max(ys)) - y0 + 1)
        top = tuple(min(255, c + CFG['grad']) for c in fill[:3])
        bot = tuple(max(0, c - CFG['grad']) for c in fill[:3])
        col = Image.new('RGB', (1, h))
        for i in range(h):
            t = i / (h - 1) if h > 1 else 0
            col.putpixel((0, i), tuple(int(top[k] + (bot[k] - top[k]) * t) for k in range(3)))
        col = col.resize((w, h), Image.BILINEAR).convert('RGBA')
        mask = Image.new('L', (w, h), 0)
        ImageDraw.Draw(mask).polygon([(q[0] - x0, q[1] - y0) for q in pp], fill=255)
        im.paste(col, (x0, y0), mask)
        if lit:                                    # faint sheen over the upper half
            sh = Image.new('RGBA', (w, h), (0, 0, 0, 0))
            sd = ImageDraw.Draw(sh)
            for i in range(h):
                a = int(CFG['gloss'] * max(0.0, 1 - i / (h * 0.55)))
                if a:
                    sd.line([(0, i), (w, i)], fill=(255, 255, 255, a))
            layer = Image.new('RGBA', im.size, (0, 0, 0, 0))
            layer.paste(sh, (x0, y0), mask)
            im.alpha_composite(layer)
        d.polygon(pp, outline=EDGE, width=lw)

    cx, cy = 0.5, CFG['rim_y']
    hw, hd, H, fl = CFG['hw'], CFG['hd'], CFG['H'], CFG['flap']
    L, B, R, F = (cx - hw, cy), (cx, cy - hd), (cx + hw, cy), (cx, cy + hd)
    flap = lambda a, b, ox, oy: [a, b, (b[0] + ox, b[1] + oy), (a[0] + ox, a[1] + oy)]

    poly(flap(L, B, -fl * 0.75, -fl * 0.75), TAN_FLAP)     # flaps behind
    poly(flap(B, R,  fl * 0.75, -fl * 0.75), TAN_FLAP)
    poly([L, B, R, F], TAN_IN)                             # interior

    a = int(n * CFG['art'])
    art = medal().resize((a, a), Image.LANCZOS).rotate(CFG['tilt'], resample=Image.BICUBIC, expand=True)
    im.alpha_composite(art, (int(n * 0.5 - art.width / 2), int(n * CFG['art_y'])))

    poly([L, F, (F[0], F[1] + H), (L[0], L[1] + H)], TAN_TOP, lit=True)
    poly([F, R, (R[0], R[1] + H), (F[0], F[1] + H)], TAN_SIDE, lit=True)
    poly(flap(R, F,  fl * CFG['fx'], fl * CFG['fy']), TAN_SIDE)
    poly(flap(F, L, -fl * CFG['fx'], fl * CFG['fy']), TAN_FLAP)
    return im.resize((size, size), Image.LANCZOS)


def frame(size):
    if size < PLAIN_BELOW:
        return Image.open(SRC).convert('RGBA').resize((size, size), Image.LANCZOS)
    return render_box(size)


def _dib(img):
    w, h = img.size
    px = img.load()
    xor = bytearray()
    for y in range(h - 1, -1, -1):
        for x in range(w):
            r, g, b, a = px[x, y]
            xor += bytes((b, g, r, a))
    rowbytes = ((w + 31) // 32) * 4
    mask = bytearray()
    for y in range(h - 1, -1, -1):
        row = bytearray(rowbytes)
        for x in range(w):
            if px[x, y][3] < 128:
                row[x // 8] |= 0x80 >> (x % 8)
        mask += row
    hdr = struct.pack('<IiiHHIIiiII', 40, w, h * 2, 1, 32, 0, len(xor) + len(mask), 0, 0, 0, 0)
    return bytes(hdr) + bytes(xor) + bytes(mask)


def write_ico(path, frames):
    blobs = []
    for f in frames:
        if f.size[0] >= 256:
            b = io.BytesIO(); f.save(b, format='PNG'); blobs.append(b.getvalue())
        else:
            blobs.append(_dib(f))
    out = struct.pack('<HHH', 0, 1, len(frames))
    off = 6 + 16 * len(frames)
    for f, blob in zip(frames, blobs):
        w = 0 if f.size[0] >= 256 else f.size[0]
        out += struct.pack('<BBBBHHII', w, w, 0, 0, 1, 32, len(blob), off)
        off += len(blob)
    open(path, 'wb').write(out + b''.join(blobs))


if __name__ == '__main__':
    import sys
    frames = [frame(s) for s in SIZES]
    write_ico(DEST, frames)
    print(f'wrote {DEST}')

    if '--preview' not in sys.argv:
        raise SystemExit
    PREV = sys.argv[sys.argv.index('--preview') + 1]
    sheet = Image.new('RGBA', (800, 300), (120, 120, 120, 255))
    dd = ImageDraw.Draw(sheet)
    sheet.paste(frames[0], (20, 20), frames[0])
    dd.rectangle([20, 20, 275, 275], outline=(255, 60, 60, 255), width=1)
    x = 300
    for f, z in zip(frames[1:], (1, 2, 3, 4)):
        im = f.resize((f.size[0] * z,) * 2, Image.NEAREST)
        sheet.paste(im, (x, 20), im)
        dd.rectangle([x, 20, x + im.size[0] - 1, 20 + im.size[1] - 1], outline=(255, 60, 60, 255), width=1)
        x += im.size[0] + 12
    sheet.save(PREV)
    print(f'wrote {PREV}')
