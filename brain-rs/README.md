# brain-rs — «мозг» Пятницы на Rust

Порт `brain.py`: тот же протокол, те же настройки, те же тексты ошибок, но без
Python в горячем пути. Поддерживаются оба провайдера — родной Gemini
(`generateContent`) и любой OpenAI-совместимый шлюз (`/v1/chat/completions`:
polza.ai, VseGPT, ProxyAPI, OpenRouter, локальный LiteLLM).

```
brain-rs/
├── src/
│   ├── brain.rs          диалог, история, цепочка запасных моделей
│   ├── config.rs         pyatnitsa_settings.json + переменные окружения
│   ├── providers/        gemini.rs, openai.rs — сборка запросов и разбор ответов
│   ├── text.rs           снятие markdown перед озвучкой
│   ├── vision.rs         подготовка картинки (сжатие, base64)
│   ├── error.rs          BrainError: Unavailable / NextModel / Io
│   ├── lib.rs            публичный API
│   └── main.rs           CLI: ask, vision, check, models, repl, stdio
└── tests/gateway.rs      проверки на локальном шлюзе-заглушке
```

## Сборка

```bash
cd brain-rs
cargo build --release        # target/release/pyatnitsa-brain.exe, ~4 МБ
cargo test                   # 45 проверок, сеть не нужна
```

Требуется Rust 1.75+. TLS — `rustls`, поэтому OpenSSL на машине не нужен.

## CLI

```bash
pyatnitsa-brain check                     # ключ, модель, доступность
pyatnitsa-brain ask "Сколько будет 2+2?"
pyatnitsa-brain ask --no-remember "разовый вопрос"
pyatnitsa-brain ask --image shot.png "Что на экране?"
pyatnitsa-brain models                    # что доступно этому ключу
pyatnitsa-brain repl                      # диалог в терминале
pyatnitsa-brain --settings path.json ask "…"
```

`--json` переводит вывод в машинный вид, `--debug` печатает подробный лог в stderr.

## Мост в Python

`brain_rs.py` повторяет интерфейс `brain.Brain`, поэтому подключение — замена
одного импорта:

```python
from brain_rs import Brain, BrainUnavailable   # было: from brain import ...

brain = Brain()                 # или Brain(settings="pyatnitsa_settings.json")
brain.ask("привет")             # -> str, markdown уже снят
brain.ask_with_image("что тут?", "shot.png")
brain.check()                   # -> (ok, message)
brain.reset()
```

Внутри — один долгоживущий процесс `pyatnitsa-brain stdio` и построчный JSON:

```
→ {"id": 1, "op": "ask", "prompt": "привет", "remember": true}
← {"id": 1, "ok": true, "answer": "Здравствуйте, сэр.", "model": "gemini-3.5-flash"}
```

Операции: `ping`, `ask`, `check`, `models`, `reset`, `history`. Картинку можно
передать как `image_path` — файл прочитает и сожмёт Rust, base64 через пайп не
гоняется. Процесс поднимается лениво и перезапускается, если умер.

Путь к бинарю: `PYATNITSA_BRAIN_BIN` → `bin/` → `brain-rs/target/release` → `PATH`.
Таймаут запроса: `PYATNITSA_BRAIN_TIMEOUT` (по умолчанию 90 с).

Проверка моста без сети:

```bash
python brain_rs.py --selftest
python test_brain_rs.py          # сквозной тест через локальный шлюз-заглушку
```

## Что перенесено один в один

| Поведение | Детали |
|---|---|
| Провайдеры | `gemini` (по умолчанию) и `openai`; выбор по `LLM_PROVIDER` |
| Ключ | `LLM_API_KEY` для шлюза, иначе `GEMINI_API_KEY`; ещё переменные окружения |
| Цепочка моделей | основная → `GEMINI_FALLBACK_MODELS`, 404/5xx/таймаут → следующая |
| `thinkingConfig` | при 400 запрос повторяется без него, флаг запоминается |
| 429 | одна пауза 1,5 с и повтор, потом понятное «лимит исчерпан» |
| Блокировка региона | `User location is not supported` → подсказка про VPN |
| Прокси | при `ProxyError` — повтор напрямую, без прокси |
| История | кольцевой буфер `LLM_HISTORY_SIZE`, пары «вопрос-ответ» |
| Зрение | сжатие до 1600 px и 3,5 МБ, JPEG q85, `remember=False` |
| Текст | снятие markdown, `—` в пробелах, схлопывание пробелов |
| Ошибки | те же формулировки, что в `brain.py` — их читает пользователь |

## Отличия от brain.py

- **Одиночное подчёркивание** (`_курсив_`) теперь снимается. В `brain.py`
  вычищалось только `__жирный__`, и синтезатор читал подчёркивания вслух.
  Границы слова проверяются, поэтому `foo_bar_baz` не портится.
- `requests` заменён на `reqwest` + `rustls`; поведение прокси сохранено
  (два клиента: с прокси и без).
- Сжатие картинок — крейт `image` вместо Pillow; Python-зависимость не нужна.
- Мост не держит HTTP-соединение в Python: процесс живёт между вызовами,
  поэтому `keep-alive` и TLS-сессия переиспользуются.

## Статус

Экспериментальная ветка. `brain.py` остаётся рабочим по умолчанию: чтобы
переключиться, замените импорт на `brain_rs`. Если бинарь не собран, мост
скажет об этом понятным текстом, а не упадёт с `FileNotFoundError`.
