"use strict";
/**
 * Пути, сохранённые настройки окна и способ запуска Python-сайдкара.
 *
 * Ничего не импортирует из renderer-а, чтобы main оставался автономным.
 */
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.apiBase = exports.isDev = void 0;
exports.pythonRoot = pythonRoot;
exports.dataDir = dataDir;
exports.ttsCacheDir = ttsCacheDir;
exports.apiPort = apiPort;
exports.loadPrefs = loadPrefs;
exports.savePrefs = savePrefs;
exports.pythonCandidates = pythonCandidates;
exports.sidecarEnabled = sidecarEnabled;
const node_fs_1 = __importDefault(require("node:fs"));
const node_path_1 = __importDefault(require("node:path"));
const electron_1 = require("electron");
const ipc_1 = require("../shared/ipc");
const log_1 = require("./log");
exports.isDev = !electron_1.app.isPackaged;
/**
 * Папка python-части ассистента.
 * dev:  <проект>/desktop/dist/main → <проект>
 * prod: <установка>/resources/python (кладётся electron-builder-ом)
 */
function pythonRoot() {
    if (electron_1.app.isPackaged)
        return node_path_1.default.join(process.resourcesPath, "python");
    return node_path_1.default.resolve(__dirname, "..", "..", "..");
}
function dataDir() {
    return node_path_1.default.join(pythonRoot(), "data");
}
function ttsCacheDir() {
    return node_path_1.default.join(dataDir(), "tts_cache");
}
/** Адрес HTTP-API Python-ядра. Порт можно переопределить переменной окружения. */
exports.apiBase = (process.env.PYATNITSA_API ?? ipc_1.DEFAULT_API_BASE).replace(/\/+$/, "");
function apiPort() {
    try {
        return Number(new URL(exports.apiBase).port) || 8765;
    }
    catch {
        return 8765;
    }
}
const DEFAULT_PREFS = { overlayMode: "normal", uiMode: "compact" };
function prefsFile() {
    return node_path_1.default.join(electron_1.app.getPath("userData"), "overlay.json");
}
function loadPrefs() {
    try {
        const raw = node_fs_1.default.readFileSync(prefsFile(), "utf8");
        const parsed = JSON.parse(raw);
        return {
            overlayMode: parsed.overlayMode === "dim" ? "dim" : "normal",
            uiMode: parsed.uiMode === "dialog" ? "dialog" : "compact",
        };
    }
    catch {
        return { ...DEFAULT_PREFS };
    }
}
function savePrefs(prefs) {
    try {
        node_fs_1.default.mkdirSync(node_path_1.default.dirname(prefsFile()), { recursive: true });
        node_fs_1.default.writeFileSync(prefsFile(), JSON.stringify(prefs, null, 2), "utf8");
    }
    catch (error) {
        (0, log_1.log)("warn", `Не удалось сохранить настройки окна: ${String(error)}`);
    }
}
function pythonArgs() {
    // Окно запускает ядро без своего же окна: --no-desktop. API поднимается на
    // том же порту, по которому к нему подключится этот Electron.
    const args = ["main.py", "--no-desktop", "--api-port", String(apiPort())];
    const extra = (process.env.PYATNITSA_PYTHON_ARGS ?? "").trim();
    if (extra)
        args.push(...extra.split(/\s+/));
    return args;
}
/**
 * Кандидаты на запуск по очереди.
 *
 * Порядок важен: в PATH часто оказывается «чужой» python (менеджеры версий,
 * Microsoft Store, другой проект), в котором нет requests/vosk. Поэтому сначала
 * явный путь, затем venv проекта, и только потом общие имена.
 */
function pythonCandidates() {
    const args = pythonArgs();
    const list = [];
    const push = (command, prefix = []) => {
        if (!command)
            return;
        const duplicate = list.some((item) => item.command === command && item.prefix.join(" ") === prefix.join(" "));
        if (!duplicate)
            list.push({ command, prefix, args });
    };
    const explicit = process.env.PYATNITSA_PYTHON?.trim();
    if (explicit)
        push(explicit);
    // Виртуальное окружение внутри проекта — самый предсказуемый вариант.
    push(node_path_1.default.join(pythonRoot(), ".venv", "Scripts", "python.exe"));
    push("python");
    push("py", ["-3"]);
    return list;
}
/** Сайдкар можно отключить: приложение тогда работает как «пустой» оверлей. */
function sidecarEnabled() {
    return process.env.PYATNITSA_NO_SIDECAR !== "1";
}
//# sourceMappingURL=config.js.map