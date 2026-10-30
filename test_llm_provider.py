# -*- coding: utf-8 -*-
"""Тесты второго провайдера мозга — OpenAI-совместимого шлюза.

Провод настоящий (requests), но шлюз поддельный: локальный HTTP-сервер на
127.0.0.1, который отвечает как ``/v1/models`` и ``/v1/chat/completions`` и
запоминает, что ему прислали. Сети, кроме loopback, нет.

Проверяется ровно то, что легко сломать незаметно:

* ``LLM_PROVIDER="openai"`` берёт базу, модель и ключ из настроек;
* системный промпт и история уходят как ``role: system/user/assistant``;
* ответ достаётся из ``choices[0].message.content`` (строкой и списком частей);
* 401 даёт понятное «ключ не принят», 402 — «нет средств»;
* 404 модели переключает на запасную;
* режим по умолчанию (Gemini) не изменился: payload остаётся ``contents``.

Запуск: ``python test_llm_provider.py`` (или ``python -m unittest``).
"""

from __future__ import annotations

import json
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

import brain
import config


class _Handler(BaseHTTPRequestHandler):
    """Мок шлюза: поведение задаётся через server.state."""

    def log_message(self, *args):  # тишина в консоли
        pass

    def _reply(self, status: int, payload: dict) -> None:
        body = json.dumps(payload, ensure_ascii=False).encode("utf-8")
        self.send_response(status)
        self.send_header("Content-Type", "application/json")
        self.send_header("Content-Length", str(len(body)))
        self.end_headers()
        self.wfile.write(body)

    def _authorized(self) -> bool:
        expected = self.server.state["key"]
        return self.headers.get("Authorization") == f"Bearer {expected}"

    def do_GET(self):
        if self.path.endswith("/models"):
            if not self._authorized():
                self._reply(401, {"error": {"message": "invalid api key"}})
                return
            self._reply(200, {"object": "list",
                              "data": [{"id": "mock-main"}, {"id": "mock-backup"}]})
            return
        self._reply(404, {"error": {"message": "not found"}})

    def do_POST(self):
        length = int(self.headers.get("Content-Length") or 0)
        raw = self.rfile.read(length) if length else b"{}"
        try:
            payload = json.loads(raw.decode("utf-8"))
        except ValueError:
            payload = {}
        self.server.state["requests"].append(payload)

        if not self._authorized():
            self._reply(401, {"error": {"message": "invalid api key"}})
            return

        status = self.server.state["status"]
        if status == 402:
            if self.server.state.get("error_style") == "string":
                # Так отвечает aiai.by: error — не объект, а строка.
                self._reply(402, {"error": "insufficient balance: have 0.0090 byn"})
                return
            self._reply(402, {"error": {"code": "INSUFFICIENT_BALANCE",
                                        "message": "Недостаточно средств"}})
            return
        if status == 404:
            self._reply(404, {"error": {"message": "model not found"}})
            return

        model = payload.get("model")
        if model not in ("mock-main", "mock-backup"):
            self._reply(404, {"error": {"message": f"model {model} not found"}})
            return

        content = self.server.state["content"]
        self._reply(200, {"choices": [{"message": {"role": "assistant",
                                                  "content": content}}]})


class GatewayTestCase(unittest.TestCase):
    """Общая обвязка: поднять мок и переключить настройки на него."""

    def setUp(self):
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), _Handler)
        self.server.state = {"key": "good-key", "status": 200,
                             "content": "привет", "requests": []}
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        port = self.server.server_address[1]

        self._saved = {name: getattr(config, name) for name in
                       ("LLM_PROVIDER", "LLM_API_BASE", "LLM_API_KEY", "LLM_MODEL",
                        "GEMINI_MODEL", "GEMINI_FALLBACK_MODELS", "LLM_SYSTEM_PROMPT")}
        config.LLM_PROVIDER = "openai"
        config.LLM_API_BASE = f"http://127.0.0.1:{port}/v1"
        config.LLM_API_KEY = "good-key"
        config.LLM_MODEL = "mock-main"
        config.GEMINI_MODEL = "gemini-3.5-flash"
        config.GEMINI_FALLBACK_MODELS = []
        config.LLM_SYSTEM_PROMPT = "Ты — Пятница."

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        for name, value in self._saved.items():
            setattr(config, name, value)

    # ------------------------------------------------------------------
    def test_provider_reads_settings(self):
        bot = brain.Brain()
        self.assertEqual(bot.provider, "openai")
        self.assertEqual(bot.model, "mock-main")
        self.assertEqual(bot.api_key, "good-key")
        self.assertTrue(bot.api_base.endswith("/v1"))

    def test_base_strips_completions_suffix(self):
        config.LLM_API_BASE = config.LLM_API_BASE + "/chat/completions"
        bot = brain.Brain()
        self.assertTrue(bot.api_base.endswith("/v1"))
        self.assertNotIn("chat/completions", bot.api_base)

    def test_check_reports_ready(self):
        ok, message = brain.Brain().check(force=True)
        self.assertTrue(ok, message)
        self.assertIn("mock-main", message)

    def test_ask_returns_text_and_keeps_history(self):
        bot = brain.Brain()
        answer = bot.ask("Скажи привет")
        self.assertEqual(answer, "привет")
        self.assertEqual(len(bot.history), 1)
        self.assertEqual(bot.history[0][0], "Скажи привет")

    def test_messages_carry_system_prompt_and_history(self):
        bot = brain.Brain()
        bot.ask("первый вопрос")
        bot.ask("второй вопрос")
        payload = self.server.state["requests"][-1]
        roles = [m["role"] for m in payload["messages"]]
        self.assertEqual(roles, ["system", "user", "assistant", "user"])
        self.assertEqual(payload["messages"][0]["content"], "Ты — Пятница.")
        self.assertEqual(payload["messages"][1]["content"], "первый вопрос")
        self.assertEqual(payload["model"], "mock-main")
        self.assertIn("max_tokens", payload)

    def test_plain_text_strips_markdown(self):
        self.server.state["content"] = "**привет** `сэр`"
        self.assertEqual(brain.Brain().ask("ну"), "привет сэр")

    def test_content_as_parts(self):
        self.server.state["content"] = [{"type": "text", "text": "часть один"},
                                        {"type": "text", "text": " и два"}]
        self.assertEqual(brain.Brain().ask("ну"), "часть один и два")

    def test_bad_key_is_reported(self):
        config.LLM_API_KEY = "wrong-key"
        with self.assertRaises(brain.BrainUnavailable) as ctx:
            brain.Brain().ask("ну")
        self.assertIn("ключ", str(ctx.exception).lower())

    def test_no_balance_is_reported(self):
        self.server.state["status"] = 402
        with self.assertRaises(brain.BrainUnavailable) as ctx:
            brain.Brain().ask("ну")
        self.assertIn("средств", str(ctx.exception))

    def test_no_balance_as_plain_string(self):
        """aiai.by кладёт error строкой — раньше это давало AttributeError."""
        self.server.state["status"] = 402
        self.server.state["error_style"] = "string"
        with self.assertRaises(brain.BrainUnavailable) as ctx:
            brain.Brain().ask("ну")
        message = str(ctx.exception)
        self.assertIn("средств", message)
        self.assertIn("insufficient balance", message)

    def test_unknown_model_falls_back(self):
        config.LLM_MODEL = "gone"
        config.GEMINI_FALLBACK_MODELS = ["mock-backup"]
        self.assertEqual(brain.Brain().ask("ну"), "привет")

    def test_image_goes_as_data_url(self):
        bot = brain.Brain()
        bot.ask("что на фото", image_b64="QUJD", image_mime="image/png")
        content = self.server.state["requests"][-1]["messages"][-1]["content"]
        self.assertIsInstance(content, list)
        self.assertEqual(content[0]["type"], "text")
        self.assertEqual(content[1]["image_url"]["url"], "data:image/png;base64,QUJD")

    def test_vision_uses_main_model(self):
        bot = brain.Brain()
        self.assertTrue(bot.vision_enabled)
        self.assertEqual(bot.model, "mock-main")


class GeminiDefaultTestCase(unittest.TestCase):
    """Режим по умолчанию обязан остаться прежним."""

    def setUp(self):
        self._saved = {name: getattr(config, name) for name in
                       ("LLM_PROVIDER", "GEMINI_API_BASE", "GEMINI_MODEL")}
        config.LLM_PROVIDER = "gemini"
        config.GEMINI_MODEL = "gemini-3.5-flash"

    def tearDown(self):
        for name, value in self._saved.items():
            setattr(config, name, value)

    def test_default_provider_is_gemini(self):
        bot = brain.Brain()
        self.assertEqual(bot.provider, "gemini")
        self.assertEqual(bot.model, "gemini-3.5-flash")
        self.assertIn("generativelanguage", bot.api_base)

    def test_gemini_payload_unchanged(self):
        bot = brain.Brain()
        payload = bot._payload("привет", image_b64=None, image_mime="image/png",
                               temperature=None)
        self.assertIn("contents", payload)
        self.assertIn("systemInstruction", payload)
        self.assertIn("generationConfig", payload)
        self.assertNotIn("messages", payload)

    def test_unknown_provider_falls_back_to_gemini(self):
        config.LLM_PROVIDER = "ollama"
        self.assertEqual(brain.Brain().provider, "gemini")


if __name__ == "__main__":
    unittest.main(verbosity=2)
