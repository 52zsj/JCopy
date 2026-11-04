#!/usr/bin/env python3
"""生成 JCopy 的图标资源（png / icns / ico / tray）。

为什么要有这个脚本而不是直接放图片：
- macOS 图标必须留白。1024 的画布上内容只占 824（四周各留 100），圆角半径 185。
  图形铺满整张画布的话，在启动台/程序坞里会比别的应用明显大一圈。
- Windows 的任务栏与开始菜单会按 16/24/32/48 取图，.ico 里必须带这些小尺寸，
  只放一张 256 会被缩得很糊，甚至看起来"没有图标"。

用法：
    python3 scripts/make-icon.py
"""
from __future__ import annotations

import pathlib
import subprocess
import tempfile

from PIL import Image, ImageDraw, ImageFilter

# ---------- 设计参数 ----------
SIZE = 1024          # 最终画布
SS = 4               # 超采样倍数，缩下来边缘才干净
W = SIZE * SS

# macOS Big Sur 之后的图标网格：内容区 824，四周留 100，圆角 185
PAD = 100 * SS
CORNER = 185 * SS

BG_TOP = (58, 112, 250)      # #3A70FA
BG_BOTTOM = (26, 58, 168)    # #1A3AA8

CARD_W, CARD_H, CARD_R = 400, 470, 58
CARD_FRONT = (312, 292)       # 左上角；中心正好落在画布中线
CARD_BACK_OFFSET = (34, -34)  # 后一张向右上错开，露出角，形成"一叠"的观感

LINE_COLOR = (183, 204, 242)
LINE_H, LINE_R = 34, 17
LINES = [(352, 452, 300), (352, 512, 300), (352, 572, 190)]  # (x, y, 宽)

OUT = pathlib.Path(__file__).resolve().parent.parent / "src-tauri" / "icons"


def s(v):
    """把 1024 坐标系里的值放大到超采样画布。"""
    return int(round(v * SS))


def gradient_box(box: tuple[int, int, int, int]) -> Image.Image:
    """竖向线性渐变，裁到指定方框。"""
    w = box[2] - box[0]
    h = box[3] - box[1]
    img = Image.new("RGB", (w, h))
    d = ImageDraw.Draw(img)
    for y in range(h):
        t = y / max(h - 1, 1)
        color = tuple(int(BG_TOP[i] + (BG_BOTTOM[i] - BG_TOP[i]) * t) for i in range(3))
        d.line([(0, y), (w, y)], fill=color)
    return img


def rounded_mask(size: tuple[int, int], radius: int) -> Image.Image:
    m = Image.new("L", size, 0)
    ImageDraw.Draw(m).rounded_rectangle([0, 0, size[0] - 1, size[1] - 1], radius=radius, fill=255)
    return m


def card(color: tuple[int, int, int, int], offset: tuple[int, int]) -> Image.Image:
    """一张卡片（含柔和投影），返回整幅画布大小的图层。"""
    layer = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    x = s(CARD_FRONT[0] + offset[0])
    y = s(CARD_FRONT[1] + offset[1])
    w, h, r = s(CARD_W), s(CARD_H), s(CARD_R)
    ImageDraw.Draw(layer).rounded_rectangle([x, y, x + w, y + h], radius=r, fill=color)
    return layer


def shadow(layer: Image.Image, blur: int, dy: int, alpha: int) -> Image.Image:
    """把图层自身的形状拿去做投影。"""
    alpha_ch = layer.split()[3].point(lambda v: alpha if v > 0 else 0)
    sh = Image.new("RGBA", layer.size, (0, 0, 0, 0))
    sh.putalpha(alpha_ch)
    sh = sh.filter(ImageFilter.GaussianBlur(blur))
    shifted = Image.new("RGBA", layer.size, (0, 0, 0, 0))
    shifted.paste(sh, (0, dy), sh)
    return shifted


def build() -> Image.Image:
    canvas = Image.new("RGBA", (W, W), (0, 0, 0, 0))

    # 1. 圆角渐变底
    base = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    grad = gradient_box((PAD, PAD, W - PAD, W - PAD)).convert("RGBA")
    base.paste(grad, (PAD, PAD))
    base.putalpha(rounded_mask((W, W), CORNER).crop((0, 0, W, W)))

    # 只保留内容区里的部分（圆角遮罩已经限制形状，这里再裁一次更保险）
    box_mask = Image.new("L", (W, W), 0)
    ImageDraw.Draw(box_mask).rounded_rectangle(
        [PAD, PAD, W - PAD - 1, W - PAD - 1], radius=CORNER, fill=255
    )
    base.putalpha(box_mask)
    canvas.alpha_composite(base)

    # 1.5 顶部高光：从白 12% 往下淡出，给方块的顶部一点体积感
    gloss = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    gd = ImageDraw.Draw(gloss)
    gh = int((W - 2 * PAD) * 0.46)
    for gy in range(gh):
        a = int(30 * (1 - gy / gh))
        gd.line([(PAD, PAD + gy), (W - PAD, PAD + gy)], fill=(255, 255, 255, a))
    gloss.putalpha(Image.composite(gloss.split()[3], Image.new("L", (W, W), 0), box_mask))
    canvas.alpha_composite(gloss)

    # 2. 后一张卡片（半透明）+ 投影
    back = card((86, 132, 255, 255), CARD_BACK_OFFSET)
    canvas.alpha_composite(shadow(back, s(18), s(10), 90))
    canvas.alpha_composite(back)

    # 3. 前一张卡片 + 投影
    front = card((255, 255, 255, 255), (0, 0))
    canvas.alpha_composite(shadow(front, s(16), s(14), 110))
    canvas.alpha_composite(front)

    # 4. 卡片里的文字线
    lines = Image.new("RGBA", (W, W), (0, 0, 0, 0))
    d = ImageDraw.Draw(lines)
    for lx, ly, lw in LINES:
        d.rounded_rectangle(
            [s(lx), s(ly), s(lx + lw), s(ly + LINE_H)],
            radius=s(LINE_R),
            fill=LINE_COLOR + (255,),
        )
    canvas.alpha_composite(lines)

    return canvas.resize((SIZE, SIZE), Image.LANCZOS)


def save_pngs(master: Image.Image) -> None:
    master.save(OUT / "icon.png")
    for name, size in [("32x32.png", 32), ("128x128.png", 128), ("128x128@2x.png", 256)]:
        master.resize((size, size), Image.LANCZOS).save(OUT / name)


def save_icns(master: Image.Image) -> None:
    """iconutil 需要 iconset 目录，每档都要 1x 与 2x。"""
    with tempfile.TemporaryDirectory() as tmp:
        iconset = pathlib.Path(tmp) / "JCopy.iconset"
        iconset.mkdir()
        specs = [
            (16, "icon_16x16.png"), (32, "icon_16x16@2x.png"),
            (32, "icon_32x32.png"), (64, "icon_32x32@2x.png"),
            (128, "icon_128x128.png"), (256, "icon_128x128@2x.png"),
            (256, "icon_256x256.png"), (512, "icon_256x256@2x.png"),
            (512, "icon_512x512.png"), (1024, "icon_512x512@2x.png"),
        ]
        for px, name in specs:
            master.resize((px, px), Image.LANCZOS).save(iconset / name)
        subprocess.run(
            ["iconutil", "-c", "icns", str(iconset), "-o", str(OUT / "icon.icns")],
            check=True,
        )


def save_ico(master: Image.Image) -> None:
    """小尺寸一并塞进 ico：Windows 任务栏、开始菜单、资源管理器都按需取。"""
    sizes = [16, 24, 32, 48, 64, 128, 256]
    master.save(OUT / "icon.ico", format="ICO", sizes=[(n, n) for n in sizes])


def save_tray(master: Image.Image) -> None:
    """菜单栏图标：macOS 用模板图（纯黑 + alpha），系统按明暗自动染色。

    菜单栏只有 22pt。这里不能画"叠加两张卡片"——两个实心矩形重叠后几乎填满
    整张画布，缩到 22pt 就是一块黑方块。必须留足镂空：描边为主，实心只用于
    小细节（夹子、文字线）。
    """
    px = 44  # 22pt @2x
    tray = Image.new("RGBA", (px, px), (0, 0, 0, 0))
    d = ImageDraw.Draw(tray)
    ink = (0, 0, 0, 255)
    # 剪贴板主体：只描边，中间留空
    d.rounded_rectangle([8, 10, 36, 39], radius=4, outline=ink, width=3)
    # 顶部夹子
    d.rounded_rectangle([17, 5, 27, 12], radius=2, fill=ink)
    # 两条内容线，一长一短
    d.rounded_rectangle([14, 20, 30, 23], radius=1, fill=ink)
    d.rounded_rectangle([14, 28, 25, 31], radius=1, fill=ink)
    tray.save(OUT / "tray.png")


def main() -> None:
    OUT.mkdir(parents=True, exist_ok=True)
    master = build()
    save_pngs(master)
    save_icns(master)
    save_ico(master)
    save_tray(master)
    for f in ("icon.png", "icon.icns", "icon.ico", "32x32.png", "128x128.png", "128x128@2x.png", "tray.png"):
        p = OUT / f
        print(f"  {f}: {p.stat().st_size / 1024:.0f} KB")


if __name__ == "__main__":
    main()
