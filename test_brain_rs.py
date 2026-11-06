# -*- coding: utf-8 -*-
"""Сквозная проверка мозга на Rust: Python-мост → бинарь → HTTP-шлюз.

Сеть и ключ не нужны: поднимается локальный шлюз-заглушка на 127.0.0.1,
который отвечает как OpenAI-совместимый сервис. Проверяется то, что нельзя
проверить юнит-тестами Rust: JSON-протокол моста, история диалога, снятие
markdown и цепочка запасных моделей.

Запуск:
    python test_brain_rs.py
"""

from __future__ import annotations

import json
import os
import sys
import tempfile
import threading
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.path.insert(0, str(ROOT))

CALLS: list = []


class _Gateway(BaseHTTPRequestHandler):
    """Минимальный /v1: список моделей + chat/completions."""

    def log_message(self, *args):  # noqa: D102 - тишина в консоли
        pass

    def _json(self, code: int, payload: dict) -> None:
        body = json.dumps(payload).encode("utf-8")
        self.send_response(code)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def do_GET(self) -> None:  # noqa: N802 - имя задаёт http.server
        if self.path.startswith("/v1/models"):
            self._json(200, {"data": [{"id": "mock-model"}]})
        else:
            self._json(404, {"error": {"message": "нет такого маршрута"}})

    def do_POST(self) -> None:  # noqa: N802
        length = int(self.headers.get("Content-Length") or 0)
        data = json.loads(self.rfile.read(length) or b"{}")
        CALLS.append(data)
        # Ключ должен доехать заголовком, а не параметром URL.
        assert self.headers.get("Authorization") == "Bearer test-key", self.headers
        if data.get("model") == "missing-model":
            self._json(404, {"error": {"message": "model missing-model not found"}})
            return
        self._json(200, {
            "choices": [{"message": {"content": "**Готово**, `сэр`. Строка 1."}}]
        })


def _settings(port: int, **extra) -> Path:
    payload = {
        "LLM_PROVIDER": "openai",
        "LLM_API_BASE": f"http://127.0.0.1:{port}/v1",
        "LLM_API_KEY": "test-key",
        "LLM_MODEL": "mock-model",
        "LLM_SYSTEM_PROMPT": "Ты — Пятница.",
        "GEMINI_MAX_TOKENS": 200,
        "LLM_HISTORY_SIZE": 4,
        "GEMINI_RECHECK_SEC": 0,
        "GEMINI_FALLBACK_MODELS": [],
    }
    payload.update(extra)
    handle = tempfile.NamedTemporaryFile(
        "w", suffix=".json", delete=False, encoding="utf-8"
    )
    with handle as fh:
        json.dump(payload, fh, ensure_ascii=False)
    return Path(handle.name)


def main() -> int:
    from brain_rs import Brain, BrainUnavailable, available

    if not available():
        print("ПРОПУСК: бинарь не собран. Нужно: cd brain-rs && cargo build --release")
        return 0

    server = HTTPServer(("127.0.0.1", 0), _Gateway)
    port = server.server_address[1]
    threading.Thread(target=server.serve_forever, daemon=True).start()

    results: list = []

    def check(name: str, ok: bool, detail: str = "") -> None:
        results.append(ok)
        tail = "" if ok or not detail else f"  ← {detail}"
        print(("OK    " if ok else "ПРОВАЛ ") + name + tail)

    try:
        settings = _settings(port)
        with Brain(settings=str(settings)) as brain:
            answer = brain.ask("привет")
            check("markdown снят перед озвучкой",
                  answer == "Готово, сэр. Строка 1.", repr(answer))

            brain.ask("а помнишь?")
            messages = CALLS[-1]["messages"]
            roles = [item["role"] for item in messages]
            check("system-промпт уходит первым",
                  messages[0]["role"] == "system" and "Пятница" in messages[0]["content"],
                  str(roles))
            check("история уходит парами",
                  roles[1:] == ["user", "assistant", "user"], str(roles))

            ok, message = brain.check()
            check("check подтверждает готовность", ok, message)
            check("модель определилась", brain.model == "mock-model", brain.model)
            check("список моделей", brain.available_models() == ["mock-model"])

            brain.reset()
            check("reset чистит историю", brain.history() == [])

            try:
                brain.ask("сломайся", model="missing-model")
                check("без запасной модели — понятная ошибка", False)
            except BrainUnavailable as exc:
                check("без запасной модели — понятная ошибка",
                      "missing-model" in str(exc), str(exc))

        # Умолчание конфига содержит запасную модель — мозг обязан её взять.
        relaxed = _settings(port)
        data = json.loads(relaxed.read_text(encoding="utf-8"))
        data.pop("GEMINI_FALLBACK_MODELS", None)
        relaxed.write_text(json.dumps(data, ensure_ascii=False), encoding="utf-8")

        before = len(CALLS)
        with Brain(settings=str(relaxed)) as brain:
            answer = brain.ask("сломайся", model="missing-model")
            attempted = [call["model"] for call in CALLS[before:]]
            check("запасная модель спасает запрос", answer.startswith("Готово"), answer)
            check("порядок: основная, затем запасная",
                  attempted == ["missing-model", "gemini-3.5-flash-lite"], str(attempted))
    finally:
        server.shutdown()

    passed = sum(1 for item in results if item)
    print(f"\nИтог: {passed}/{len(results)} проверок пройдено.")
    return 0 if passed == len(results) else 1


if __name__ == "__main__":
    raise SystemExit(main())
