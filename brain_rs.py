# -*- coding: utf-8 -*-
"""Мост к «мозгу» на Rust (`brain-rs`).

Модуль повторяет публичный интерфейс `brain.Brain`, поэтому подключается
одной заменой импорта::

    # было
    from brain import Brain, BrainUnavailable
    # стало
    from brain_rs import Brain, BrainUnavailable

Внутри — один долгоживущий процесс `pyatnitsa-brain stdio`, общение построчным
JSON. Так модель и HTTP-соединение живут в Rust, а Python остаётся только
оркестратором: история диалога, ретраи моделей и разбор ошибок уже там.

Собрать бинарь::

    cd brain-rs && cargo build --release

Путь можно задать переменной окружения ``PYATNITSA_BRAIN_BIN``.
"""

from __future__ import annotations

import json
import os
import queue
import subprocess
import sys
import threading
import time
from pathlib import Path

__all__ = [
    "Brain",
    "BrainUnavailable",
    "VERSION",
    "binary_path",
    "available",
]

VERSION = "0.1.0"

_EXE = "pyatnitsa-brain.exe" if os.name == "nt" else "pyatnitsa-brain"
_ROOT = Path(__file__).resolve().parent


class BrainUnavailable(RuntimeError):
    """Мозг недоступен: нет бинаря, нет ключа, нет сети или исчерпан лимит.

    Тот же класс ошибки, что и в `brain.py`, — вызывающий код ловит её, не
    задумываясь о том, на чём написан мозг.
    """


def binary_path() -> Path:
    """Ищет собранный бинарь: env → bin/ → cargo target → PATH."""
    override = os.environ.get("PYATNITSA_BRAIN_BIN", "").strip()
    candidates = []
    if override:
        candidates.append(Path(override))
    candidates += [
        _ROOT / "bin" / _EXE,
        _ROOT / "brain-rs" / "target" / "release" / _EXE,
        _ROOT / "brain-rs" / "target" / "debug" / _EXE,
    ]
    for candidate in candidates:
        if candidate.is_file():
            return candidate

    from shutil import which

    found = which(_EXE)
    if found:
        return Path(found)

    raise BrainUnavailable(
        "Бинарь мозга на Rust не найден. Соберите его:\n"
        "    cd brain-rs && cargo build --release\n"
        "или укажите путь в PYATNITSA_BRAIN_BIN."
    )


def available() -> bool:
    """Есть ли собранный бинарь — без исключений, для мягкой деградации."""
    try:
        binary_path()
        return True
    except BrainUnavailable:
        return False


class _Worker:
    """Один процесс `pyatnitsa-brain stdio` и очередь его ответов."""

    _EOF = object()

    def __init__(self, settings: str | None = None, debug: bool = False):
        self._settings = settings
        self._debug = debug
        self._lock = threading.Lock()
        self._next_id = 0
        self._process: subprocess.Popen | None = None
        self._replies: queue.Queue = queue.Queue()
        self._reader: threading.Thread | None = None

    # -- процесс -----------------------------------------------------
    def _spawn(self) -> None:
        command = [str(binary_path())]
        if self._settings:
            command += ["--settings", str(self._settings)]
        if self._debug:
            command.append("--debug")
        command.append("stdio")

        creationflags = 0
        if os.name == "nt":
            # Без лишнего окна консоли поверх оверлея.
            creationflags = getattr(subprocess, "CREATE_NO_WINDOW", 0)

        try:
            self._process = subprocess.Popen(
                command,
                stdin=subprocess.PIPE,
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
                text=True,
                encoding="utf-8",
                errors="replace",
                bufsize=1,
                creationflags=creationflags,
            )
        except OSError as exc:
            raise BrainUnavailable(f"Не удалось запустить мозг на Rust: {exc}") from exc

        self._replies = queue.Queue()
        self._reader = threading.Thread(
            target=self._read_stdout, name="brain-rs-reader", daemon=True
        )
        self._reader.start()
        threading.Thread(
            target=self._drain_stderr, name="brain-rs-stderr", daemon=True
        ).start()

    def _read_stdout(self) -> None:
        assert self._process is not None
        stream = self._process.stdout
        if stream is None:
            self._replies.put(self._EOF)
            return
        for line in stream:
            line = line.strip()
            if line:
                self._replies.put(line)
        self._replies.put(self._EOF)

    def _drain_stderr(self) -> None:
        """stderr вычитывается всегда, иначе пайп переполнится и процесс встанет."""
        assert self._process is not None
        stream = self._process.stderr
        if stream is None:
            return
        for line in stream:
            if self._debug:
                sys.stderr.write(f"[brain-rs] {line}")

    def _alive(self) -> bool:
        return self._process is not None and self._process.poll() is None

    def close(self) -> None:
        process, self._process = self._process, None
        if process is None:
            return
        try:
            if process.stdin:
                process.stdin.close()
            process.wait(timeout=3)
        except Exception:  # noqa: BLE001
            try:
                process.kill()
            except Exception:  # noqa: BLE001
                pass

    # -- запросы -----------------------------------------------------
    def request(self, op: str, timeout: float, **fields) -> dict:
        with self._lock:
            try:
                return self._request_locked(op, timeout, fields)
            except (BrokenPipeError, OSError):
                # Процесс умер между запросами — поднимаем заново и пробуем ещё раз.
                self.close()
                return self._request_locked(op, timeout, fields)

    def _request_locked(self, op: str, timeout: float, fields: dict) -> dict:
        if not self._alive():
            self._spawn()

        self._next_id += 1
        request_id = self._next_id
        payload = {"id": request_id, "op": op}
        payload.update(fields)

        assert self._process is not None and self._process.stdin is not None
        try:
            self._process.stdin.write(json.dumps(payload, ensure_ascii=False) + "\n")
            self._process.stdin.flush()
        except (BrokenPipeError, OSError) as exc:
            raise BrainUnavailable("Мозг на Rust неожиданно завершился.") from exc

        deadline = time.monotonic() + timeout
        while True:
            left = deadline - time.monotonic()
            if left <= 0:
                raise BrainUnavailable(
                    f"Мозг на Rust не ответил за {timeout:.0f} с."
                )
            try:
                raw = self._replies.get(timeout=left)
            except queue.Empty:
                raise BrainUnavailable(
                    f"Мозг на Rust не ответил за {timeout:.0f} с."
                ) from None

            if raw is self._EOF:
                self.close()
                raise BrainUnavailable("Мозг на Rust завершился, не ответив.")

            try:
                reply = json.loads(raw)
            except ValueError:
                continue

            # Ответы на отменённые по таймауту запросы просто пропускаем.
            if reply.get("id") not in (request_id, None):
                continue
            return reply


def _key_present() -> bool:
    """Есть ли ключ LLM. Спрашивать Rust ради этого не нужно."""
    try:
        import config  # тот же конфиг, что читает brain.py

        provider = str(getattr(config, "LLM_PROVIDER", "gemini") or "gemini").lower()
        if provider == "openai" and str(getattr(config, "LLM_API_KEY", "") or "").strip():
            return True
        if str(getattr(config, "GEMINI_API_KEY", "") or "").strip():
            return True
    except Exception:  # noqa: BLE001
        pass
    return bool(
        os.environ.get("GEMINI_API_KEY", "").strip()
        or os.environ.get("LLM_API_KEY", "").strip()
    )


class Brain:
    """Rust-мозг с интерфейсом `brain.Brain`."""

    def __init__(self, model: str | None = None, *, settings: str | None = None,
                 debug: bool = False):
        self._model = model
        self._timeout = float(os.environ.get("PYATNITSA_BRAIN_TIMEOUT", "90"))
        self._worker = _Worker(settings=settings, debug=debug)

    # -- служебное ---------------------------------------------------
    def _ask_timeout(self) -> float:
        try:
            import config

            return max(self._timeout, float(getattr(config, "GEMINI_TIMEOUT", 60)) + 20)
        except Exception:  # noqa: BLE001
            return self._timeout

    def _call(self, op: str, timeout: float | None = None, **fields) -> dict:
        reply = self._worker.request(op, timeout or self._timeout, **fields)
        if not reply.get("ok"):
            raise BrainUnavailable(str(reply.get("error") or "Мозг ответил ошибкой."))
        return reply

    def close(self) -> None:
        """Останавливает процесс мозга. Вызывать при выходе из приложения."""
        self._worker.close()

    def __enter__(self) -> "Brain":
        return self

    def __exit__(self, *_exc) -> None:
        self.close()

    def __del__(self) -> None:
        try:
            self.close()
        except Exception:  # noqa: BLE001
            pass

    # -- свойства ----------------------------------------------------
    @property
    def model(self) -> str:
        return self._model or ""

    @property
    def vision_enabled(self) -> bool:
        try:
            import config

            return bool(getattr(config, "GEMINI_VISION_MODEL", "") or self._model)
        except Exception:  # noqa: BLE001
            return bool(self._model)

    def key_present(self) -> bool:
        return _key_present()

    def vision_available(self) -> bool:
        return self.vision_enabled and self.key_present()

    # -- диалог ------------------------------------------------------
    def ask(self, prompt: str, *, remember: bool = True,
            image_b64: str | None = None, image_mime: str = "image/png",
            model: str | None = None, temperature: float | None = None) -> str:
        fields: dict = {"prompt": prompt or "", "remember": bool(remember)}
        if image_b64:
            fields["image_b64"] = image_b64
            fields["image_mime"] = image_mime or "image/png"
        if model or self._model:
            fields["model"] = model or self._model
        if temperature is not None:
            fields["temperature"] = float(temperature)

        reply = self._call("ask", timeout=self._ask_timeout(), **fields)
        return str(reply.get("answer") or "")

    def ask_with_image(self, prompt: str, image_path) -> str:
        """Картинку читает и сжимает Rust — base64 через пайп не гоняем."""
        path = Path(image_path)
        if not path.is_file():
            raise BrainUnavailable(f"Файл изображения не найден: {path}")
        reply = self._call(
            "ask",
            timeout=self._ask_timeout(),
            prompt=prompt,
            remember=False,
            image_path=str(path),
        )
        return str(reply.get("answer") or "")

    def reset(self) -> None:
        self._call("reset")

    def history(self) -> list:
        items = self._call("history").get("history") or []
        return [(str(item.get("q", "")), str(item.get("a", ""))) for item in items]

    # -- диагностика -------------------------------------------------
    def check(self, force: bool = False) -> tuple:
        """(ок, сообщение) — как в brain.py. Сеть проверяется один раз в N секунд."""
        try:
            reply = self._worker.request("check", 30.0)
        except BrainUnavailable as exc:
            return False, str(exc)
        ok = bool(reply.get("ok"))
        message = str(reply.get("message") or "")
        if ok and reply.get("model"):
            self._model = self._model or str(reply["model"])
        return ok, message

    def warmup(self) -> bool:
        ok, message = self.check()
        try:
            from utils import log

            log(f"Мозг (Rust): {message}", "debug" if ok else "warn")
        except Exception:  # noqa: BLE001
            pass
        return ok

    def available_models(self, force: bool = False) -> list:
        try:
            reply = self._worker.request("models", 30.0)
        except BrainUnavailable:
            return []
        return [str(model) for model in (reply.get("models") or [])]


def _selftest() -> int:
    """Проверка моста без обращения к сети: ping + версия бинаря."""
    try:
        path = binary_path()
    except BrainUnavailable as exc:
        print(exc)
        return 1
    print(f"бинарь: {path}")
    worker = _Worker()
    try:
        reply = worker.request("ping", 10.0)
    finally:
        worker.close()
    if not reply.get("pong"):
        print(f"плохой ответ: {reply}")
        return 1
    print(f"мозг отвечает, версия {reply.get('version')}")
    print(f"ключ LLM: {'есть' if _key_present() else 'НЕТ'}")
    return 0


if __name__ == "__main__":
    if "--selftest" in sys.argv:
        raise SystemExit(_selftest())
    print(__doc__)
    raise SystemExit(0)
