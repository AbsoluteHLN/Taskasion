"""生成 src-tauri/icons/icon.ico(纯标准库,无 PIL)。

图形:靛蓝→紫渐变圆角方块 + 白色对勾,3x 超采样抗锯齿,多尺寸 BMP-in-ICO。
用法: python scripts/gen_icon.py
"""

from __future__ import annotations

import math
import struct
from pathlib import Path

SIZES = [16, 24, 32, 48, 64, 128, 256]
SS = 3  # 每轴超采样倍数

C0 = (0x58, 0x65, 0xF2)  # 左上 靛蓝
C1 = (0x8B, 0x5C, 0xF6)  # 右下 紫


def lerp(a: tuple, b: tuple, t: float) -> tuple:
    return tuple(round(a[i] + (b[i] - a[i]) * t) for i in range(3))


def seg_dist(px: float, py: float, ax: float, ay: float, bx: float, by: float) -> float:
    vx, vy = bx - ax, by - ay
    wx, wy = px - ax, py - ay
    L2 = vx * vx + vy * vy
    t = 0.0 if L2 == 0 else max(0.0, min(1.0, (wx * vx + wy * vy) / L2))
    dx, dy = wx - t * vx, wy - t * vy
    return math.hypot(dx, dy)


def render(size: int) -> bytes:
    """返回该尺寸的 32bpp 自下而上 BGRA 像素数据。"""
    half = size / 2
    radius = 0.24 * size
    check_w = 0.10 * size
    pts = ((0.30 * size, 0.52 * size), (0.44 * size, 0.66 * size), (0.74 * size, 0.34 * size))
    px = bytearray(size * size * 4)
    for y in range(size):
        for x in range(size):
            bg = chk = 0
            for sy in range(SS):
                for sx in range(SS):
                    # 采样点位于像素内 (x, y) 的细网格上
                    fx, fy = x + (sx + 0.5) / SS, y + (sy + 0.5) / SS
                    # 圆角方块内部判定
                    cx = min(max(fx, radius), size - radius)
                    cy = min(max(fy, radius), size - radius)
                    inside = math.hypot(fx - cx, fy - cy) <= radius
                    if not inside:
                        continue
                    bg += 1
                    d = min(
                        seg_dist(fx, fy, *pts[0], *pts[1]),
                        seg_dist(fx, fy, *pts[1], *pts[2]),
                    )
                    if d <= check_w / 2:
                        chk += 1
            bg_cov, chk_cov = bg / (SS * SS), chk / (SS * SS)
            t = (x / size + y / size) / 2
            base = lerp(C0, C1, t)
            col = [round(base[i] * (1 - chk_cov) + 255 * chk_cov) for i in range(3)]
            alpha = round(255 * bg_cov)
            # 自下而上 BGRA
            row = size - 1 - y
            off = (row * size + x) * 4
            px[off:off + 4] = bytes((col[2], col[1], col[0], alpha))
    return bytes(px)


def bmp_entry(size: int, pixels: bytes) -> bytes:
    """BITMAPINFOHEADER + XOR + AND 掩码(不透明)。"""
    and_stride = ((size + 31) // 32) * 4
    and_mask = b"\x00" * (and_stride * size)
    header = struct.pack("<IiiHHIIiiII", 40, size, size * 2, 1, 32, 0,
                         len(pixels) + len(and_mask), 0, 0, 0, 0)
    return header + pixels + and_mask


def main() -> None:
    images = []
    for s in SIZES:
        data = bmp_entry(s, render(s))
        images.append((s, data))
        print(f"  {s}x{s}: {len(data)} bytes")

    count = len(images)
    out = struct.pack("<HHH", 0, 1, count)
    offset = 6 + 16 * count
    for s, data in images:
        out += struct.pack("<BBBBHHII", s % 256, s % 256, 0, 0, 1, 32, len(data), offset)
        offset += len(data)
    out += b"".join(data for _, data in images)

    target = Path(__file__).resolve().parent.parent / "src-tauri" / "icons" / "icon.ico"
    target.write_bytes(out)
    print(f"written {target} ({len(out)} bytes)")


if __name__ == "__main__":
    main()
