# -*- coding: utf-8 -*-
"""Пятница.exe — один файл, который запускает всё.

Зачем он нужен: раньше «Пятница.exe» была просто копией ``pythonw.exe``, и
запускалась она только с аргументами (``Пятница.exe main.py --background``) —
то есть двойной клик по ней ничего не делал. Этот лаунчер собирается
PyInstaller-ом и работает сам по себе:

  * находит папку проекта (рядом с собой или в стандартных местах);
  * находит интерпретатор Python (виртуальное окружение проекта, затем
    обычные установки, затем PATH);
  * запускает ``main.py --background`` без окна консоли и сразу выходит —
    ассистент остаётся жить в фоне, как при запуске с ярлыка.

Если что-то не найдено, показывается окно с понятным текстом, а не молчание:
запуск из Проводника не оставляет консоли, куда можно было бы написать ошибку.

Пересборка::

    python build_exe.py
"""

from __future__ import annotations

import ctypes
import os
import shutil
import subprocess
import sys
import time
import urllib.error
import urllib.request
from pathlib import Path

APP_NAME = "Пятница"
MAIN_SCRIPT = "main.py"
API_HEALTH = "http://127.0.0.1:8765/api/health"

# Флаги Windows: отдельная группа процессов и никакого окна консоли.
DETACHED_PROCESS = 0x00000008
CREATE_NO_WINDOW = 0x08000000

# Сколько ждём, чтобы понять «процесс жив или упал на старте».
STARTUP_CHECK_SEC = 3.0

# Где ещё может лежать проект, если exe скопировали в другое место.
FALLBACK_DIRS = (
    Path.home() / "Documents" / "Pyatnitsa-Vosk",
    Path.home() / "Pyatnitsa-Vosk",
    Path("C:/Pyatnitsa-Vosk"),
)


# ------------------------------------------------------------------- журнал
#
# Из Проводника у процесса нет консоли: любая ошибка исчезает вместе с ним.
# Поэтому лаунчер пишет короткий журнал рядом с логами ассистента.

def log(message: str, root: Path | None = None) -> None:
    """Пишет строку в data/launcher.log. Молча, если файл недоступен."""
    base = root or project_root()
    if base is None:
        return
    try:
        folder = base / "data"
        folder.mkdir(parents=True, exist_ok=True)
        stamp = time.strftime("%Y-%m-%d %H:%M:%S")
        with (folder / "launcher.log").open("a", encoding="utf-8") as handle:
            handle.write(f"{stamp}  {message}\n")
    except OSError:
        pass


def api_alive(timeout: float = 0.6) -> bool:
    """Уже работает другой экземпляр? Тогда второй только покажет окно."""
    try:
        with urllib.request.urlopen(API_HEALTH, timeout=timeout) as response:
            return response.status == 200
    except (urllib.error.URLError, OSError, ValueError):
        return False


# --------------------------------------------------------------- поиск путей

def _exe_dir() -> Path:
    """Папка, где лежит сам exe (для onefile — не временная распаковка)."""
    if getattr(sys, "frozen", False):
        return Path(sys.executable).resolve().parent
    return Path(__file__).resolve().parent


def project_root() -> Path | None:
    """Папка с main.py: рядом с exe, на уровень выше или в стандартных местах."""
    here = _exe_dir()
    for candidate in (here, here.parent, *FALLBACK_DIRS):
        try:
            if (candidate / MAIN_SCRIPT).exists():
                return candidate
        except OSError:
            continue
    return None


def _version_key(folder: Path) -> tuple[int, ...]:
    """«Python314» -> (3, 14). Нужно, чтобы 3.14 шёл раньше 3.9."""
    digits = "".join(ch if ch.isdigit() else " " for ch in folder.name).split()
    return tuple(int(part) for part in digits) if digits else (0,)


def python_candidates(root: Path):
    """Интерпретаторы в порядке предпочтения. Первым — без окна консоли."""
    # 1. Окружение самого проекта: там стоят vosk, piper и остальное.
    for venv in (".venv", "venv", "env"):
        for exe in ("pythonw.exe", "python.exe"):
            candidate = root / venv / "Scripts" / exe
            if candidate.exists():
                yield candidate

    # 2. Обычные установки Python, от новых версий к старым.
    bases: list[Path] = []
    local = os.environ.get("LOCALAPPDATA")
    if local:
        bases.append(Path(local) / "Programs" / "Python")
    for env in ("ProgramFiles", "ProgramFiles(x86)"):
        value = os.environ.get(env)
        if value:
            bases.append(Path(value) / "Python")
    installs: list[Path] = []
    for base in bases:
        try:
            if base.is_dir():
                installs.extend(p for p in base.glob("Python3*") if p.is_dir())
        except OSError:
            continue
    for folder in sorted(installs, key=_version_key, reverse=True):
        for exe in ("pythonw.exe", "python.exe"):
            candidate = folder / exe
            if candidate.exists():
                yield candidate

    # 3. Что найдётся в PATH.
    for name in ("pythonw", "python"):
        found = shutil.which(name)
        if found:
            yield Path(found)


# ------------------------------------------------------------------ сообщения

def _message(text: str, title: str = APP_NAME, error: bool = True) -> None:
    """Окно с сообщением: из Проводника консоли нет, иначе ошибка не видна."""
    flags = 0x10 if error else 0x40          # MB_ICONERROR / MB_ICONINFORMATION
    try:
        ctypes.windll.user32.MessageBoxW(None, text, title, flags)
    except Exception:  # noqa: BLE001
        print(f"{title}: {text}")


# --------------------------------------------------------------------- запуск

def launch(root: Path, python: Path) -> bool:
    """Запускает main.py в фоне. True — ассистент работает (или уже работал)."""
    already = api_alive()
    command = [str(python), str(root / MAIN_SCRIPT), "--background"]
    flags = DETACHED_PROCESS | CREATE_NO_WINDOW
    log(f"запуск: {' '.join(command)} (уже работает: {already})", root)
    try:
        process = subprocess.Popen(
            command,
            cwd=str(root),
            creationflags=flags,
            stdin=subprocess.DEVNULL,
            stdout=subprocess.DEVNULL,
            stderr=subprocess.DEVNULL,
            close_fds=True,
        )
    except OSError as exc:
        log(f"Popen не удался: {exc!r}", root)
        return False

    # Если экземпляр уже работал, этот процесс только покажет окно и сразу
    # выйдет — это нормальный исход, ждать его бессмысленно.
    if already:
        log(f"окно показано существующего экземпляра (pid={process.pid})", root)
        return True

    # Иначе проверяем, что процесс не умер на старте: иначе из Проводника
    # двойной клик выглядел бы как «ничего не происходит».
    deadline = time.monotonic() + STARTUP_CHECK_SEC
    while time.monotonic() < deadline:
        code = process.poll()
        if code is not None:
            log(f"процесс завершился с кодом {code}", root)
            return False
        time.sleep(0.15)

    log(f"ассистент запущен, pid={process.pid}", root)
    return True


def main() -> int:
    root = project_root()
    if root is None:
        log("не найдена папка проекта (рядом нет main.py)", None)
        _message(
            "Не нашёл папку проекта: рядом с Пятница.exe нет main.py.\n\n"
            "Положите Пятница.exe в папку Pyatnitsa-Vosk (рядом с main.py) "
            "и запустите снова."
        )
        return 1

    log("--- запуск лаунчера ---", root)
    tried: list[str] = []
    for python in python_candidates(root):
        tried.append(str(python))
        if launch(root, python):
            return 0

    log(f"все интерпретаторы не подошли: {tried}", root)
    _message(
        "Не удалось запустить ассистента.\n\n"
        f"Папка проекта: {root}\n"
        f"Пробовал: {len(tried)} интерпретатор(ов)\n\n"
        "Причина записана в data\\launcher.log и data\\pyatnitsa.log.\n"
        "Если Python не установлен — запустите install.bat в папке проекта."
    )
    return 1


if __name__ == "__main__":
    sys.exit(main())
