"""生成应用图标源图（1024x1024 PNG）。

设计：深色圆角方块 + 额度环形仪表（62% 填充）+ 中心点。
跑完源图后由 `npx tauri icon` 派生出各平台尺寸。

用法：
    python tools/make_icon.py
"""

from PIL import Image, ImageDraw
from pathlib import Path

# 用 4 倍超采样再缩回去，边缘不会毛
SS = 4
SIZE = 1024
BG = (27, 26, 36, 255)          # 深色底
TRACK = (56, 51, 95, 255)       # 环形轨道
FILL = (167, 157, 240, 255)     # 已用额度（亮紫）
DOT = (133, 183, 235, 255)      # 中心点（蓝）
USED_RATIO = 0.62               # 环形填充比例


def main() -> None:
    canvas = SIZE * SS
    img = Image.new("RGBA", (canvas, canvas), (0, 0, 0, 0))
    draw = ImageDraw.Draw(img)

    # 圆角背景
    draw.rounded_rectangle([0, 0, canvas - 1, canvas - 1], radius=230 * SS, fill=BG)

    cx = cy = canvas // 2
    outer, inner = 322 * SS, 232 * SS

    # 轨道环 -> 挖空中心 -> 画进度弧 -> 再挖空中心
    draw.ellipse([cx - outer, cy - outer, cx + outer, cy + outer], fill=TRACK)
    draw.ellipse([cx - inner, cy - inner, cx + inner, cy + inner], fill=BG)
    draw.pieslice(
        [cx - outer, cy - outer, cx + outer, cy + outer],
        start=-90,
        end=-90 + 360 * USED_RATIO,
        fill=FILL,
    )
    draw.ellipse([cx - inner, cy - inner, cx + inner, cy + inner], fill=BG)

    # 中心点
    r = 74 * SS
    draw.ellipse([cx - r, cy - r, cx + r, cy + r], fill=DOT)

    img = img.resize((SIZE, SIZE), Image.LANCZOS)

    out = Path(__file__).resolve().parent.parent / "app-icon.png"
    img.save(out)
    print(f"wrote {out}")


if __name__ == "__main__":
    main()
