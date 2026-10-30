# -*- coding: utf-8 -*-
"""Проверка ключа LLM: какому сервису он принадлежит и какие модели доступны.

Зачем: у Пятницы два режима работы с нейросетью — родной Gemini и любой
OpenAI-совместимый шлюз (``LLM_PROVIDER = "openai"``). Ключ вида ``sk-...``
ничего не говорит о том, где его выдали, а угадывать по ошибке «401» долго.
Скрипт перебирает известные шлюзы и показывает, какой из них ключ принял.

    python llm_check.py sk-xxxx
    python llm_check.py sk-xxxx --base https://api.polza.ai/v1 --model openai/gpt-5-mini
    python llm_check.py sk-xxxx --fast        # только авторизация, без ответа модели

403 и 400 в ответе — это тоже «ключ принят»: шлюз узнал его и отказал по другой
причине (регион, модель, параметры). «Не наш» — это 401 и 403 с явным текстом
про ключ, поэтому вердикт печатается вместе с фрагментом ответа.
"""

from __future__ import annotations

import argparse
import json
import sys
from concurrent.futures import ThreadPoolExecutor

import requests

# (название, база до /v1, дешёвая модель для проверки)
PROVIDERS = [
    ("aiai.by", "https://api.aiai.by/v1", "gemini-3.5-flash-lite"),
    ("polza.ai", "https://api.polza.ai/v1", "openai/gpt-5-mini"),
    ("OpenRouter", "https://openrouter.ai/api/v1", "google/gemini-2.5-flash"),
    ("VseGPT", "https://api.vsegpt.ru/v1", "openai/gpt-4o-mini"),
    ("ProxyAPI", "https://api.proxyapi.ru/openai/v1", "gpt-4o-mini"),
    ("1app.ru", "https://1app.ru/v1", "gpt-4o-mini"),
    ("AITunnel", "https://api.aitunnel.ru/v1", "gpt-4o-mini"),
    ("BotHub", "https://bothub.chat/api/v2/openai/v1", "gpt-4o-mini"),
    ("GPTunnel", "https://gptunnel.ru/v1", "gpt-4o-mini"),
    ("NeuroAPI", "https://neuroapi.host/v1", "gpt-4o-mini"),
    ("OpenAI", "https://api.openai.com/v1", "gpt-4o-mini"),
    ("DeepSeek", "https://api.deepseek.com/v1", "deepseek-chat"),
    ("Groq", "https://api.groq.com/openai/v1", "llama-3.3-70b-versatile"),
    ("Mistral", "https://api.mistral.ai/v1", "mistral-small-latest"),
    ("Together", "https://api.together.xyz/v1", "meta-llama/Llama-3.3-70B-Instruct-Turbo"),
    ("SambaNova", "https://api.sambanova.ai/v1", "DeepSeek-V3.1"),
    ("Cerebras", "https://api.cerebras.ai/v1", "llama-3.3-70b"),
    ("xAI", "https://api.x.ai/v1", "grok-4"),
    ("Perplexity", "https://api.perplexity.ai", "sonar"),
    ("Novita", "https://api.novita.ai/v3/openai", "deepseek/deepseek-v3"),
    ("aihubmix", "https://aihubmix.com/v1", "gpt-4o-mini"),
    ("ppio", "https://api.ppio.com/openai/v1", "deepseek/deepseek-chat"),
    ("aimlapi", "https://api.aimlapi.com/v1", "gpt-4o-mini"),
    ("redpill", "https://api.redpill.ai/v1", "openai/gpt-5-mini"),
    ("Cloud.ru", "https://foundation-models.api.cloud.ru/v1", "GigaChat"),
    ("302.ai", "https://api.302.ai/v1", "gpt-4o-mini"),
    ("gpt.ge", "https://api.gpt.ge/v1", "gpt-4o-mini"),
    ("closeai", "https://api.closeai-proxy.xyz/v1", "gpt-4o-mini"),
    ("burn.hair", "https://burn.hair/v1", "gpt-4o-mini"),
    ("ohmygpt", "https://api.ohmygpt.com/v1", "gpt-4o-mini"),
    ("nano-gpt", "https://nano-gpt.com/api/v1", "gpt-4o-mini"),
    ("deepinfra", "https://api.deepinfra.com/v1/openai", "meta-llama/Meta-Llama-3.1-8B-Instruct"),
    ("chutes", "https://llm.chutes.ai/v1", "deepseek-ai/DeepSeek-V3"),
    ("featherless", "https://api.featherless.ai/v1", "meta-llama/Meta-Llama-3.1-8B-Instruct"),
    ("venice", "https://api.venice.ai/api/v1", "venice-uncensored"),
    ("nvidia-nim", "https://integrate.api.nvidia.com/v1", "meta/llama-3.1-8b-instruct"),
    ("llm7", "https://api.llm7.io/v1", "gpt-4o-mini"),
    ("z.ai", "https://api.z.ai/api/paas/v4", "glm-4.6"),
    ("moonshot", "https://api.moonshot.cn/v1", "moonshot-v1-8k"),
    ("minimax", "https://api.minimax.chat/v1", "MiniMax-Text-01"),
    ("stepfun", "https://api.stepfun.com/v1", "step-2-16k"),
    ("hunyuan", "https://api.hunyuan.cloud.tencent.com/v1", "hunyuan-turbos-latest"),
    ("baichuan", "https://api.baichuan-ai.com/v1", "Baichuan4"),
    ("siliconflow", "https://api.siliconflow.cn/v1", "Qwen/Qwen2.5-7B-Instruct"),
    ("dashscope", "https://dashscope-intl.aliyuncs.com/compatible-mode/v1", "qwen-plus"),
    ("openai-proxy.org", "https://api.openai-proxy.org/v1", "gpt-4o-mini"),
    ("gptsapi", "https://api.gptsapi.net/v1", "gpt-4o-mini"),
    ("chatanywhere", "https://api.chatanywhere.tech/v1", "gpt-4o-mini"),
    ("gptapi.us", "https://api.gptapi.us/v1", "gpt-4o-mini"),
    ("requesty", "https://router.requesty.ai/v1", "openai/gpt-4o-mini"),
    ("hyperbolic", "https://api.hyperbolic.xyz/v1", "meta-llama/Llama-3.3-70B-Instruct"),
    ("friendli", "https://api.friendli.ai/serverless/v1", "meta-llama-3.1-8b-instruct"),
    ("upstage", "https://api.upstage.ai/v1", "solar-pro"),
    ("arliai", "https://api.arliai.com/v1", "Llama-3.3-70B-Instruct"),
    ("nebius", "https://api.studio.nebius.ai/v1", "meta-llama/Llama-3.3-70B-Instruct"),
    ("Ollama (локально)", "http://127.0.0.1:11434/v1", "llama3.1"),
]

PROMPT = "Ответь одним словом: привет"
TIMEOUT = 25

# Слова в ответе, по которым видно, о чём именно ругается шлюз.
KEY_WORDS = (
    "api key", "api_key", "apikey", "invalid api", "invalid token", "invalid authentication",
    "incorrect api key", "authentication fail", "authentication_error", "unauthorized",
    "unauthenticated", "invalid_credentials", "authorized_error", "invalid authorization",
    "user with this api key not found", "token is invalid", "token expired", "no such key",
    "недействительн", "неверн", "недопустим", "неправильн", "токен должен быть создан",
    "invalid key", "key secret",
)
MODEL_WORDS = (
    "model", "does not exist", "unavailable", "decommissioned", "end of life",
    "not supported", "quota", "limit", "balance", "insufficient", "средств", "лимит",
    "max_tokens", "messages", "invalid request", "bad request",
)
GEO_WORDS = ("region", "country", "location", "territory", "not available in your")


def classify(status: int, detail: str) -> str:
    """ok / auth (ключ принят, споткнулись о модель) / geo / no / err."""
    lowered = (detail or "").lower()
    if status == 200:
        return "ok"
    if status >= 500:
        return "err"
    if any(word in lowered for word in GEO_WORDS):
        return "geo"
    if any(word in lowered for word in KEY_WORDS):
        return "no"
    if any(word in lowered for word in MODEL_WORDS):
        return "auth"
    if status in (400, 404, 409, 422, 429):
        return "auth"
    return "no"


def _short(text: str, limit: int = 110) -> str:
    return (text or "").strip().replace("\n", " ")[:limit]


def _error_message(response) -> str:
    try:
        data = response.json()
    except ValueError:
        return _short(response.text)
    if isinstance(data, dict):
        error = data.get("error")
        if isinstance(error, dict):
            return _short(str(error.get("message") or error))
        if isinstance(error, str):
            return _short(error)
        if data.get("detail"):
            return _short(str(data["detail"]))
        if data.get("message"):
            return _short(str(data["message"]))
    return _short(response.text)


def check(name: str, base: str, model: str, key: str, fast: bool = False) -> dict:
    """Один шлюз. Возвращает вердикт: ok / auth / no / err."""
    session = requests.Session()
    session.trust_env = True
    headers = {"Authorization": f"Bearer {key}", "Content-Type": "application/json"}
    result = {"name": name, "base": base, "verdict": "no", "detail": "", "models": []}

    if fast:
        try:
            response = session.get(f"{base}/models", headers=headers, timeout=12)
        except Exception as exc:  # noqa: BLE001
            result.update(verdict="err", detail=exc.__class__.__name__)
            return result
        detail = _error_message(response)
        verdict = classify(response.status_code, detail)
        if verdict == "no":
            result["detail"] = detail
            return result
        if response.status_code >= 400:
            result.update(verdict="auth", detail=f"{response.status_code}: {detail}")
            return result
        try:
            data = response.json()
        except ValueError:
            result.update(verdict="auth", detail="не JSON")
            return result
        items = data.get("data") if isinstance(data, dict) else data
        result["models"] = [str(i.get("id")) for i in (items or [])
                            if isinstance(i, dict) and i.get("id")][:8]
        result.update(verdict="auth", detail="список моделей получен")
        return result

    payload = {"model": model, "max_tokens": 16,
               "messages": [{"role": "user", "content": PROMPT}]}
    try:
        response = session.post(f"{base}/chat/completions", headers=headers,
                                json=payload, timeout=TIMEOUT)
    except Exception as exc:  # noqa: BLE001
        result.update(verdict="err", detail=exc.__class__.__name__)
        return result

    if response.status_code == 200:
        try:
            data = response.json()
        except ValueError:
            data = {}
        choices = data.get("choices") or [{}]
        message = (choices[0].get("message") or {}).get("content")
        answer = message if isinstance(message, str) else json.dumps(message, ensure_ascii=False)
        result.update(verdict="ok", detail=_short(answer or "пустой ответ"))
        return result

    detail = _error_message(response)
    result.update(verdict=classify(response.status_code, detail),
                  detail=f"{response.status_code}: {detail}")
    if result["verdict"] == "auth":
        _confirm_auth(session, base, headers, result)
    return result


def _confirm_auth(session, base: str, headers: dict, result: dict) -> None:
    """Проверяет, что дело правда в ключе, а не в модели или открытом доступе.

    400/404 про модель отвечают и без авторизации (llm7, публичный список NIM),
    поэтому «ключ принят» подтверждаем отдельно: /models без заголовка и с ним.
    """
    try:
        public = session.get(f"{base}/models", timeout=10)
    except Exception:  # noqa: BLE001
        return
    if public.status_code == 200:
        result.update(verdict="public", detail="шлюз отвечает без ключа — ключ не проверяется")
        return
    try:
        private = session.get(f"{base}/models", headers=headers, timeout=10)
    except Exception:  # noqa: BLE001
        return
    pdetail = _error_message(private)
    if private.status_code in (401, 403) and classify(private.status_code, pdetail) == "no":
        result.update(verdict="no", detail=f"{private.status_code}: {pdetail}")


def main() -> int:
    parser = argparse.ArgumentParser(description="Опознать API-ключ LLM")
    parser.add_argument("key", help="ключ, например sk-...")
    parser.add_argument("--base", help="проверить конкретный шлюз, напр. https://api.polza.ai/v1")
    parser.add_argument("--model", help="модель для проверки на этом шлюзе")
    parser.add_argument("--fast", action="store_true", help="только авторизация")
    parser.add_argument("--all", action="store_true", help="показывать и отказы")
    args = parser.parse_args()

    if args.base:
        targets = [(args.base, args.base, args.model or "gpt-4o-mini")]
    else:
        targets = PROVIDERS

    print(f"Проверяю ключ {args.key[:8]}…{args.key[-4:]} "
          f"({len(targets)} сервисов)\n")

    results = []
    with ThreadPoolExecutor(max_workers=8) as pool:
        futures = [pool.submit(check, name, base, model, args.key, args.fast)
                   for name, base, model in targets]
        for future in futures:
            results.append(future.result())

    hits = [r for r in results if r["verdict"] in ("ok", "auth")]
    for item in sorted(hits, key=lambda r: (r["verdict"] != "ok", r["name"])):
        mark = "КЛЮЧ ПРИНЯТ" if item["verdict"] == "auth" else "РАБОТАЕТ"
        print(f"[{mark}] {item['name']} — {item['base']}")
        print(f"           {item['detail']}")
        if item["models"]:
            print(f"           модели: {', '.join(item['models'])}")

    if not hits:
        print("Ни один известный шлюз ключ не принял.")

    geo = [r for r in results if r["verdict"] == "geo"]
    if geo:
        print("\nБлокируют регион — проверить можно только через VPN:")
        for item in geo:
            print(f"  {item['name']} — {item['base']}")

    public = [r for r in results if r["verdict"] == "public"]
    if public:
        print("\nОтвечают без ключа (ключ не проверяется):")
        for item in public:
            print(f"  {item['name']} — {item['base']}")

    if args.all:
        print("\nОстальные:")
        for item in results:
            if item["verdict"] in ("ok", "auth", "geo", "public"):
                continue
            print(f"  {item['name']:20} {item['verdict']:4} {item['detail']}")

    print("\nЕсли шлюз найден — пропишите в pyatnitsa_settings.json:")
    print('  "LLM_PROVIDER": "openai",')
    print('  "LLM_API_BASE": "<база из списка выше>",')
    print('  "LLM_MODEL": "<модель>",')
    print('  "LLM_API_KEY": "<ключ>"')
    return 0


if __name__ == "__main__":
    sys.exit(main())
