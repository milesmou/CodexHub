"""开发工具：抓取运行中的应用窗口截图，用于验证界面渲染。

用 stdlib 的 ctypes 调用 user32 把窗口置顶，再用 Pillow 抓屏。

用法：
    python tools/capture_window.py [输出路径]
"""

import ctypes
import sys
from ctypes import wintypes
from pathlib import Path

from PIL import ImageGrab

user32 = ctypes.windll.user32

SW_RESTORE = 9
TARGET_TITLE = "Codex Hub"

VK_MENU = 0x12  # Alt
KEYEVENTF_KEYUP = 0x0002

user32.keybd_event.restype = None


class RECT(ctypes.Structure):
    _fields_ = [
        ("left", ctypes.c_long),
        ("top", ctypes.c_long),
        ("right", ctypes.c_long),
        ("bottom", ctypes.c_long),
    ]


def find_window() -> int:
    """按标题找窗口，找不到就遍历一遍再试。"""
    hwnd = user32.FindWindowW(None, TARGET_TITLE)
    if hwnd:
        return hwnd

    found = []

    @ctypes.WINFUNCTYPE(ctypes.c_bool, wintypes.HWND, wintypes.LPARAM)
    def enum_proc(h, _lparam):
        length = user32.GetWindowTextLengthW(h)
        if length:
            buf = ctypes.create_unicode_buffer(length + 1)
            user32.GetWindowTextW(h, buf, length + 1)
            title = buf.value
            if title and ("Codex" in title or "账号管家" in title):
                found.append((h, title))
        return True

    user32.EnumWindows(enum_proc, 0)
    for h, title in found:
        print(f"  候选窗口: '{title}' hwnd={h}")
    return found[0][0] if found else 0


def force_foreground(hwnd: int) -> bool:
    """把窗口提到前台，成功返回 True（ALT 技巧绕开抢焦点限制）。"""
    user32.ShowWindow(hwnd, SW_RESTORE)
    user32.keybd_event(VK_MENU, 0, 0, 0)
    user32.SetForegroundWindow(hwnd)
    user32.keybd_event(VK_MENU, 0, KEYEVENTF_KEYUP, 0)
    ctypes.windll.kernel32.Sleep(1200)
    return user32.GetForegroundWindow() == hwnd


def main() -> int:
    out = Path(sys.argv[1] if len(sys.argv) > 1 else "screenshot.png").resolve()

    hwnd = find_window()
    if not hwnd:
        print("没有找到应用窗口，确认程序是否在运行")
        return 1

    if not force_foreground(hwnd):
        # 抢不到前台时截出来的是「盖在上面的那个窗口」，
        # 不提示的话很容易误以为界面就是这样
        print("警告：应用没能抢到前台，截图可能被其他窗口遮挡，结果不可信。")

    rect = RECT()
    user32.GetWindowRect(hwnd, ctypes.byref(rect))
    box = (rect.left, rect.top, rect.right, rect.bottom)
    print(f"窗口位置: {box[0]},{box[1]} 尺寸 {box[2] - box[0]}x{box[3] - box[1]}")

    if box[2] <= box[0] or box[3] <= box[1]:
        print("窗口尺寸异常")
        return 1

    img = ImageGrab.grab(bbox=box, all_screens=True)
    img.save(out)
    print(f"已保存: {out}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
