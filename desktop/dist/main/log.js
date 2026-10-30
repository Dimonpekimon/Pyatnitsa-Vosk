"use strict";
/**
 * Мини-логгер main-процесса.
 *
 * В dev пишем в консоль, в собранном приложении дублируем в файл — иначе
 * ошибки запуска Python-сайдкара увидеть негде (у .exe нет консоли).
 */
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
exports.log = log;
const node_fs_1 = __importDefault(require("node:fs"));
const node_path_1 = __importDefault(require("node:path"));
const electron_1 = require("electron");
let logFile = null;
function file() {
    if (logFile)
        return logFile;
    try {
        const dir = electron_1.app.getPath("logs");
        node_fs_1.default.mkdirSync(dir, { recursive: true });
        logFile = node_path_1.default.join(dir, "pyatnitsa-overlay.log");
    }
    catch {
        logFile = null;
    }
    return logFile;
}
function log(level, message) {
    const line = `[${new Date().toISOString().slice(11, 19)}] ${level.toUpperCase()} ${message}`;
    if (level === "error")
        console.error(line);
    else if (level === "warn")
        console.warn(line);
    else
        console.log(line);
    const target = file();
    if (!target)
        return;
    try {
        node_fs_1.default.appendFileSync(target, line + "\n", "utf8");
    }
    catch {
        /* лог не должен ронять приложение */
    }
}
//# sourceMappingURL=log.js.map