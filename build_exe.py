# -*- coding: utf-8 -*-
"""Сборка «Пятница.exe» — одного файла, который запускает всё.

Что делает сборка:

  * PyInstaller упаковывает ``launcher.py`` в один exe без окна консоли;
  * иконка берётся из ``assets/icon.ico``;
  * готовый файл кладётся в корень проекта под именем ``Пятница.exe``.

Имя на время сборки — латиницей (``Pyatnitsa``): PyInstaller пишет часть
служебных файлов путями, и кириллица в имени там иногда превращается в
кракозябры. Переименование после сборки ничего не ломает.

Запуск::

    python build_exe.py            # собрать
    python build_exe.py --clean    # собрать заново, без кэша
"""

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent
BUILD_NAME = "Pyatnitsa"                 # имя на время сборки (ASCII)
TARGET_NAME = "Пятница.exe"              # как файл называется у пользователя
ICON = BASE_DIR / "assets" / "icon.ico"
LAUNCHER = BASE_DIR / "launcher.py"
DIST_DIR = BASE_DIR / "build" / "launcher"
WORK_DIR = BASE_DIR / "build" / "work"


def build(clean: bool = False) -> int:
    if not LAUNCHER.exists():
        print(f"[!] Нет {LAUNCHER.name} — собирать нечего")
        return 1
    try:
        import PyInstaller  # noqa: F401
    except ImportError:
        print("Нужен PyInstaller: pip install pyinstaller")
        return 1

    command = [
        sys.executable, "-m", "PyInstaller",
        "--noconfirm",
        "--onefile",
        "--windowed",                     # без окна консоли
        "--name", BUILD_NAME,
        "--distpath", str(DIST_DIR),
        "--workpath", str(WORK_DIR),
        "--specpath", str(BASE_DIR / "build"),
        "--log-level", "WARN",
    ]
    if clean:
        command.append("--clean")
    if ICON.exists():
        command += ["--icon", str(ICON)]
    else:
        print(f"[i] Иконки нет ({ICON}) — соберу без неё")
    command.append(str(LAUNCHER))

    print("Собираю exe…")
    result = subprocess.run(command, cwd=str(BASE_DIR), check=False)
    if result.returncode != 0:
        print(f"[!] Сборка не удалась, код {result.returncode}")
        return result.returncode

    built = DIST_DIR / f"{BUILD_NAME}.exe"
    if not built.exists():
        print(f"[!] PyInstaller не оставил {built}")
        return 1

    target = BASE_DIR / TARGET_NAME
    try:
        if target.exists():
            target.unlink()
        shutil.copy2(built, target)
    except OSError as exc:
        print(f"[!] Не удалось положить {TARGET_NAME}: {exc}")
        return 1

    size_mb = target.stat().st_size / (1024 * 1024)
    print(f"Готово: {target}  ({size_mb:.1f} МБ)")
    print("Двойной клик запускает ассистента — без аргументов и без консоли.")
    return 0


def main() -> int:
    parser = argparse.ArgumentParser(description="Сборка Пятница.exe")
    parser.add_argument("--clean", action="store_true", help="без кэша сборки")
    args = parser.parse_args()
    return build(clean=args.clean)


if __name__ == "__main__":
    sys.exit(main())
