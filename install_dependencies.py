# -*- coding: utf-8 -*-
"""Установка всех зависимостей Пятницы одной командой.

Скрипт делает то, что раньше приходилось собирать по кускам из README:

1. проверяет версию Python и разрядность;
2. ставит пакеты из ``requirements.txt`` (pip);
3. проверяет, что каждый нужный модуль действительно импортируется;
4. скачивает голос Piper, если его модели ещё нет;
5. проверяет модель распознавания речи (Vosk) — при ``--models`` качает её;
6. собирает настольное окно (``desktop/``): ``npm install`` и ``npm run build``.

Запуск::

    python install_dependencies.py              # поставить всё, что нужно
    python install_dependencies.py --check      # только отчёт, ничего не менять
    python install_dependencies.py --core       # только pip-пакеты
    python install_dependencies.py --models     # ещё и скачать модель Vosk (45 МБ)
    python install_dependencies.py --no-desktop # без сборки Electron-окна

Тот же запуск двойным щелчком — ``install.bat``.
"""

from __future__ import annotations

import argparse
import importlib.util
import shutil
import subprocess
import sys
from pathlib import Path

BASE_DIR = Path(__file__).resolve().parent
REQUIREMENTS = BASE_DIR / "requirements.txt"

#: (имя модуля для import, пакет для pip, обязателен ли, зачем нужен)
PACKAGES = (
    ("numpy", "numpy", True, "звук, обработка аудио и тембра"),
    ("requests", "requests", True, "погода, курсы валют, веб-поиск"),
    ("sounddevice", "sounddevice", True, "микрофон"),
    ("soundfile", "soundfile", True, "чтение WAV для распознавания"),
    ("soxr", "soxr", True, "перересемплинг 44.1/48 кГц -> 16 кГц"),
    ("vosk", "vosk", True, "распознавание речи (Vosk)"),
    ("piper", "piper-tts", True, "основной синтез речи"),
    ("edge_tts", "edge-tts", True, "резервный синтез речи"),
    ("pyttsx3", "pyttsx3", True, "офлайн-резерв синтеза (SAPI5)"),
    ("win32com", "pywin32", True, "системные вызовы Windows"),
    ("pycaw", "pycaw", True, "громкость"),
    ("comtypes", "comtypes", True, "громкость (обёртка COM)"),
    ("screen_brightness_control", "screen-brightness-control", True, "яркость экрана"),
    ("PIL", "pillow", True, "скриншоты и анализ экрана"),
    ("pyperclip", "pyperclip", True, "буфер обмена"),
    ("ddgs", "ddgs", True, "поиск в интернете"),
    ("av", "av", False, "разбор MP3 от edge-tts"),
    ("pyautogui", "pyautogui", False, "управление окнами из Python"),
    ("wikipedia", "wikipedia", False, "запасной источник для поиска"),
    ("telegram", "python-telegram-bot", False, "управление с телефона"),
    ("winrt", "winrt-Windows.Media.Ocr", False, "чтение текста с экрана (OCR)"),
)


def _line(text: str = "") -> None:
    print(text, flush=True)


def _head(title: str) -> None:
    _line()
    _line("=" * 62)
    _line(f"  {title}")
    _line("=" * 62)


def check_python() -> bool:
    """Проверяет версию и разрядность интерпретатора."""
    _head("Python")
    version = f"{sys.version_info.major}.{sys.version_info.minor}.{sys.version_info.micro}"
    _line(f"  Интерпретатор: {sys.executable}")
    _line(f"  Версия:        {version}  ({'64' if sys.maxsize > 2**32 else '32'} бита)")
    ok = True
    if sys.version_info < (3, 10):
        _line("  [!] Нужен Python 3.10 или новее — иначе часть пакетов не встанет.")
        ok = False
    if sys.maxsize <= 2**32:
        _line("  [!] 32-битный Python: модель распознавания не поместится в память.")
        ok = False
    if ok:
        _line("  [ok] Версия подходит.")
    return ok


def run_pip(extra_args: list) -> bool:
    """Ставит пакеты из requirements.txt через pip текущего интерпретатора."""
    if not REQUIREMENTS.exists():
        _line(f"  [!] Нет файла {REQUIREMENTS.name} — ставить нечего.")
        return False
    command = [sys.executable, "-m", "pip", "install", "--disable-pip-version-check",
               "-r", str(REQUIREMENTS)] + extra_args
    _line("  " + " ".join(command))
    try:
        return subprocess.call(command, cwd=str(BASE_DIR)) == 0
    except OSError as exc:
        _line(f"  [!] Не удалось запустить pip: {exc}")
        return False


def missing_packages() -> list:
    """Какие модули из списка не импортируются прямо сейчас."""
    result = []
    for module, package, required, purpose in PACKAGES:
        if importlib.util.find_spec(module) is None:
            result.append((module, package, required, purpose))
    return result


def report_packages() -> bool:
    """Печатает таблицу «модуль на месте / отсутствует» и возвращает True, если всё есть."""
    _head("Пакеты Python")
    absent = {item[0] for item in missing_packages()}
    critical_missing = []
    for module, package, required, purpose in PACKAGES:
        if module in absent:
            mark = "[!]" if required else "[-]"
            _line(f"  {mark} {module:26} нет — {purpose} (pip install {package})")
            if required:
                critical_missing.append(package)
        else:
            _line(f"  [ok] {module:25} {purpose}")
    if critical_missing:
        _line()
        _line("  Не хватает обязательного: " + ", ".join(sorted(set(critical_missing))))
        return False
    _line()
    _line("  Все обязательные пакеты на месте.")
    return True


def ensure_voice(download: bool = True) -> bool:
    """Проверяет активный голос Piper; при download — скачивает, если его нет."""
    _head("Голос Piper")
    try:
        import voices
    except Exception as exc:  # noqa: BLE001
        _line(f"  [!] Модуль voices не загрузился: {exc}")
        return False
    name = voices.active_name()
    title = voices.VOICE_CATALOG[name]["title"]
    if voices.is_installed(name):
        _line(f"  [ok] Голос «{title}» уже скачан.")
        return True
    if not download:
        _line(f"  [-] Голоса «{title}» нет — он скачается при первом запуске.")
        return True
    _line(f"  Качаю голос «{title}» ({name})…")
    try:
        voices.ensure(name, on_log=lambda message: _line("    " + message))
    except Exception as exc:  # noqa: BLE001
        _line(f"  [!] Не удалось скачать голос: {exc}")
        return False
    _line("  [ok] Голос готов.")
    return True


def asr_model_status(download: bool) -> bool:
    """Проверяет модель распознавания речи Vosk; при download — качает её."""
    _head("Модель распознавания речи (Vosk)")
    try:
        import config
        import speech
    except Exception as exc:  # noqa: BLE001
        _line(f"  [!] Модули распознавания не загрузились: {exc}")
        return False
    path = speech.model_path()
    _line(f"  Папка модели: {path}")
    if speech.model_ready(path):
        _line("  [ok] Модель уже скачана.")
        return True
    if not download:
        _line("  [-] Модели нет. Она скачается сама при первом запуске (~45 МБ).")
        _line("      Скачать сейчас: python install_dependencies.py --models")
        return True
    _line("  Качаю модель (~45 МБ)…")
    try:
        speech.ensure_model(path, allow_download=True)
    except Exception as exc:  # noqa: BLE001
        _line(f"  [!] Не удалось загрузить модель: {exc}")
        return False
    _line("  [ok] Модель готова.")
    return True


def build_desktop() -> bool:
    """Собирает настольное окно Electron (desktop/)."""
    _head("Настольное окно (Electron)")
    desktop = BASE_DIR / "desktop"
    if not (desktop / "package.json").exists():
        _line("  [-] Папки desktop/ нет — окно не нужно, пропускаю.")
        return True
    npm = shutil.which("npm")
    if npm is None:
        _line("  [-] npm не найден. Окно не собрать, но голосом ассистент работает.")
        _line("      Поставьте Node.js с https://nodejs.org и запустите скрипт снова.")
        return True
    for args, title in ((["install"], "npm install"), (["run", "build"], "npm run build")):
        _line(f"  {title}…")
        try:
            code = subprocess.call([npm] + args, cwd=str(desktop))
        except OSError as exc:
            _line(f"  [!] {title} не запустился: {exc}")
            return False
        if code != 0:
            _line(f"  [!] {title} завершился с кодом {code}.")
            return False
    _line("  [ok] Окно собрано.")
    return True


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        description="Установка зависимостей голосового ассистента Пятница")
    parser.add_argument("--check", action="store_true",
                        help="только отчёт: ничего не устанавливать")
    parser.add_argument("--core", action="store_true",
                        help="только пакеты Python (без голоса и окна)")
    parser.add_argument("--models", action="store_true",
                        help="скачать модель распознавания речи (~226 МБ)")
    parser.add_argument("--no-voices", action="store_true", help="не скачивать голос")
    parser.add_argument("--no-desktop", action="store_true",
                        help="не собирать настольное окно")
    parser.add_argument("--upgrade", action="store_true",
                        help="обновить уже установленные пакеты до последних версий")
    args = parser.parse_args(argv)

    _line()
    _line("  ПЯТНИЦА — УСТАНОВКА ЗАВИСИМОСТЕЙ")
    _line(f"  Папка проекта: {BASE_DIR}")

    if args.check:
        ok = check_python()
        ok = report_packages() and ok
        ensure_voice(download=False)
        asr_model_status(download=False)
        _line()
        _line("  Проверка закончена. Ничего не изменено.")
        return 0 if ok else 1

    ok = check_python()
    _head("Пакеты Python (pip)")
    pip_args = ["--upgrade"] if args.upgrade else []
    if run_pip(pip_args):
        _line("  [ok] pip отработал без ошибок.")
    else:
        _line("  [!] pip вернул ошибку — смотрите вывод выше.")
        ok = False
    ok = report_packages() and ok

    if not args.core:
        if not args.no_voices:
            ok = ensure_voice() and ok
        ok = asr_model_status(download=args.models) and ok
        if not args.no_desktop:
            ok = build_desktop() and ok

    _head("ИТОГ")
    if ok:
        _line("  Всё готово. Запуск: run_pyatnitsa.bat или python main.py")
        _line("  Первый запуск может занять время: грузятся модели.")
    else:
        _line("  Часть шагов не удалась — смотрите пометки [!] выше.")
        _line("  Ассистент запустится, но соответствующие команды работать не будут.")
    _line()
    return 0 if ok else 1


if __name__ == "__main__":
    sys.exit(main())
