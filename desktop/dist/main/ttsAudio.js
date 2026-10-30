"use strict";
/**
 * Мост «Python уже синтезировал фразу» → Electron получает байты WAV.
 *
 * Идея: Piper в Python кладёт результат в data/tts_cache/<sha1>.wav, где ключ —
 *     sha1("piper|текст|путь_к_модели|speed|volume")[:20]
 * (tts.py: _cache_key + _speak_piper). Мы считаем ровно такой же ключ в Node и
 * читаем файл. Так визуализатор получает НАСТОЯЩИЙ спектр Piper, а Python
 * править не нужно вообще.
 *
 * Ограничения, о которых честно стоит знать:
 *  - работает только при TTS_USE_CACHE = true и TTS_ENGINE = piper;
 *  - если Python перед озвучкой чистит текст (markdown и т.п.), ключ не совпадёт —
 *    тогда renderer автоматически переключается на уровни из SSE.
 */
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.readTtsSettings = readTtsSettings;
exports.cacheKeyFor = cacheKeyFor;
exports.cacheFileFor = cacheFileFor;
exports.waitForWav = waitForWav;
const node_crypto_1 = __importDefault(require("node:crypto"));
const node_fs_1 = __importDefault(require("node:fs"));
const node_path_1 = __importDefault(require("node:path"));
const config_1 = require("./config");
const log_1 = require("./log");
const SETTINGS_FILE = "pyatnitsa_settings.json";
/** До переименования настройки лежали под этим именем — читаем как запасной. */
const LEGACY_SETTINGS_FILE = "jarvis_settings.json";
/** Достаёт числовой литерал из JSON как есть, сохраняя «1.0». */
function rawNumber(text, key, fallback) {
    const match = new RegExp(`"${key}"\\s*:\\s*(-?[0-9][0-9.eE+-]*)`).exec(text);
    return match ? match[1] : fallback;
}
function readTtsSettings() {
    const file = node_path_1.default.join((0, config_1.pythonRoot)(), SETTINGS_FILE);
    let text;
    try {
        text = node_fs_1.default.readFileSync(file, "utf8");
    }
    catch {
        try {
            text = node_fs_1.default.readFileSync(node_path_1.default.join((0, config_1.pythonRoot)(), LEGACY_SETTINGS_FILE), "utf8");
        }
        catch {
            (0, log_1.log)("debug", "pyatnitsa_settings.json не найден — мост аудио выключен");
            return null;
        }
    }
    let parsed = {};
    try {
        parsed = JSON.parse(text);
    }
    catch {
        (0, log_1.log)("warn", "pyatnitsa_settings.json не разобрался как JSON");
    }
    // Голос выбирается именем (PIPER_VOICE), а путь выводится из него — ровно
    // как в voices.active_path(). Пустой PIPER_MODEL означает «по имени голоса»:
    // раньше здесь был жёстко прописан dmitri, и после перехода на женский голос
    // ключ кэша перестал бы совпадать, а визуализатор — находить звук.
    const storedModel = typeof parsed.PIPER_MODEL === "string" ? parsed.PIPER_MODEL.trim() : "";
    const voiceName = typeof parsed.PIPER_VOICE === "string" ? parsed.PIPER_VOICE.trim().toLowerCase() : "";
    const safeVoice = /^[a-z0-9_-]+$/.test(voiceName) ? voiceName : "irina";
    const modelPath = storedModel || node_path_1.default.join((0, config_1.pythonRoot)(), "data", "voices", `ru_RU-${safeVoice}-medium.onnx`);
    return {
        modelPath,
        // Запасное значение повторяет config.PIPER_SPEED: если в настройках ключа
        // нет, Python хеширует именно это число.
        speed: rawNumber(text, "PIPER_SPEED", "1.2"),
        volume: rawNumber(text, "PIPER_VOLUME", "1.0"),
        useCache: /"TTS_USE_CACHE"\s*:\s*true/.test(text),
    };
}
/** Точная копия tts._cache_key для движка piper. */
function cacheKeyFor(text, settings) {
    const raw = ["piper", text, settings.modelPath, settings.speed, settings.volume].join("|");
    return node_crypto_1.default.createHash("sha1").update(raw, "utf8").digest("hex").slice(0, 20);
}
function cacheFileFor(text, settings) {
    return node_path_1.default.join((0, config_1.ttsCacheDir)(), `${cacheKeyFor(text, settings)}.wav`);
}
/** Читает WAV, если он уже полностью записан (Python пишет через .part + rename). */
function tryRead(file) {
    try {
        const stat = node_fs_1.default.statSync(file);
        if (stat.size < 1000)
            return null;
        const buffer = node_fs_1.default.readFileSync(file);
        if (buffer.length < 44)
            return null;
        if (buffer.toString("ascii", 0, 4) !== "RIFF")
            return null;
        if (buffer.toString("ascii", 8, 12) !== "WAVE")
            return null;
        return buffer;
    }
    catch {
        return null;
    }
}
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
/**
 * Ждёт появления файла: синтез новой фразы занимает ~1.5–2 с, и событие
 * «assistant сказал» приходит раньше, чем Piper дописал WAV.
 */
async function waitForWav(text, timeoutMs = 7000) {
    const settings = readTtsSettings();
    if (!settings)
        return null;
    if (!settings.useCache) {
        (0, log_1.log)("debug", "TTS_USE_CACHE выключен — аудио-мост недоступен");
        return null;
    }
    const candidates = [text, text.trim()].filter((value, index, all) => value && all.indexOf(value) === index);
    const files = candidates.map((value) => cacheFileFor(value, settings));
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        for (const file of files) {
            const buffer = tryRead(file);
            if (buffer)
                return buffer;
        }
        await sleep(120);
    }
    return null;
}
//# sourceMappingURL=ttsAudio.js.map