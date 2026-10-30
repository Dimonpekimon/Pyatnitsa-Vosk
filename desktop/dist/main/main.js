"use strict";
/**
 * Главный процесс: прозрачное окно-оверлей, два режима прозрачности,
 * click-through, трей, горячие клавиши и связь с Python-ядром.
 *
 * Роли файлов:
 *   config.ts   — пути, настройки окна, как запускать Python
 *   sidecar.ts  — сам дочерний процесс
 *   sse.ts      — поток событий от ядра
 *   ttsAudio.ts — готовый WAV от Piper для визуализатора
 *   main.ts     — окно + IPC + меню (этот файл)
 */
var __importDefault = (this && this.__importDefault) || function (mod) {
    return (mod && mod.__esModule) ? mod : { "default": mod };
};
Object.defineProperty(exports, "__esModule", { value: true });
const node_fs_1 = __importDefault(require("node:fs"));
const node_path_1 = __importDefault(require("node:path"));
const electron_1 = require("electron");
const ipc_1 = require("../shared/ipc");
const config_1 = require("./config");
const log_1 = require("./log");
const sidecar_1 = require("./sidecar");
const sse_1 = require("./sse");
const ttsAudio_1 = require("./ttsAudio");
// AudioContext в Electron стартует «спящим» до жеста пользователя; для оверлея
// это неприемлемо — звук должен играть по событию из Python.
electron_1.app.commandLine.appendSwitch("autoplay-policy", "no-user-gesture-required");
electron_1.app.setAppUserModelId("ru.dima.pyatnitsa.desktop");
// Аварийный выключатель для машин, где драйвер роняет GPU-процесс (в логе это
// «GPU process exited unexpectedly» + «GPU process isn't usable. Goodbye.»).
// Живопись Canvas на CPU медленнее, но окно хотя бы открывается.
if (process.env.PYATNITSA_DISABLE_GPU === "1") {
    electron_1.app.disableHardwareAcceleration();
}
const DEV_URL = process.env.PYATNITSA_DEV_URL ?? "http://127.0.0.1:5273";
/** Грузить страницу с Vite-сервера (HMR) или из готовой сборки. */
const useDevServer = process.env.PYATNITSA_DEV === "1";
const MARGIN = 24; // отступ от края экрана
const DIM_OPACITY = 0.35; // «полупрозрачный» режим
const SIZES = {
    compact: { width: 320, height: 168 },
    dialog: { width: 430, height: 470 },
};
let win = null;
let tray = null;
let prefs = { overlayMode: "normal", uiMode: "compact" };
let sidecar = null;
let sse = null;
let apiReady = false;
let quitting = false;
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
// ------------------------------------------------------------------ окно
function createWindow() {
    const { workArea } = electron_1.screen.getPrimaryDisplay();
    const size = SIZES[prefs.uiMode];
    const x = Math.round(workArea.x + workArea.width - size.width - MARGIN);
    const y = Math.round(workArea.y + workArea.height - size.height - MARGIN);
    win = new electron_1.BrowserWindow({
        width: size.width,
        height: size.height,
        x,
        y,
        show: false,
        frame: false, // без рамки и заголовка
        transparent: true, // фон окна полностью прозрачный
        backgroundColor: "#00000000",
        hasShadow: false, // тень Windows нарисовала бы прямоугольник
        resizable: false,
        movable: true,
        minimizable: false,
        maximizable: false,
        fullscreenable: false,
        skipTaskbar: true, // оверлею не место на панели задач
        alwaysOnTop: true,
        title: "Пятница — оверлей",
        webPreferences: {
            preload: node_path_1.default.join(__dirname, "..", "preload", "preload.js"),
            contextIsolation: true,
            nodeIntegration: false,
            sandbox: false,
            // Без этого requestAnimationFrame замирает, когда окно не в фокусе,
            // и визуализатор «застывает» на последнем кадре.
            backgroundThrottling: false,
            spellcheck: false,
        },
    });
    // 'screen-saver' — самый высокий уровень: окно остаётся поверх полноэкранных
    // приложений. Просто alwaysOnTop:true этого не даёт.
    win.setAlwaysOnTop(true, "screen-saver");
    win.setVisibleOnAllWorkspaces(true, { visibleOnFullScreen: true });
    // Стартовое состояние: окно «сквозное», клики уходят на рабочий стол.
    // forward:true — окно продолжает получать mousemove, по нему и работает
    // определение «курсор над интерактивной зоной».
    win.setIgnoreMouseEvents(true, { forward: true });
    win.once("ready-to-show", () => {
        win?.showInactive(); // showInactive: не отбираем фокус у активного окна
        applyOverlayMode(prefs.overlayMode, false);
        sendToRenderer(ipc_1.CH.uiMode, prefs.uiMode);
    });
    win.on("closed", () => {
        win = null;
    });
    // Никаких новых окон: внешние ссылки не должны открываться внутри оверлея.
    win.webContents.setWindowOpenHandler(() => ({ action: "deny" }));
    const builtFile = node_path_1.default.join(__dirname, "..", "renderer", "index.html");
    // Ошибки из renderer-а иначе видны только в DevTools, а в собранном
    // приложении их не увидеть вовсе.
    win.webContents.on("did-finish-load", () => (0, log_1.log)("info", "Страница загружена"));
    win.webContents.on("did-fail-load", (_event, code, description, url, isMainFrame) => {
        // Vite не запущен — не показываем ERR_CONNECTION_REFUSED, а грузим сборку.
        if (useDevServer && isMainFrame && !win?.webContents.getURL().startsWith("file:")) {
            (0, log_1.log)("warn", `Vite недоступен (${code} ${description}) — гружу dist/renderer`);
            void win?.loadFile(builtFile);
            return;
        }
        (0, log_1.log)("error", `Не загрузилась страница ${url}: ${code} ${description}`);
    });
    win.webContents.on("console-message", ((...args) => {
        // Electron менял сигнатуру: (event, level, message, line, sourceId) →
        // (event, details). Поддерживаем обе, чтобы лог не зависел от версии.
        const details = args[1];
        if (details && typeof details === "object") {
            (0, log_1.log)("debug", `renderer[${details.level}]: ${details.message ?? ""}`);
        }
        else {
            (0, log_1.log)("debug", `renderer: ${String(args[2] ?? "")}`);
        }
    }));
    if (useDevServer) {
        void win.loadURL(DEV_URL);
    }
    else {
        void win.loadFile(builtFile);
    }
}
function sendToRenderer(channel, payload) {
    if (!win || win.isDestroyed())
        return;
    win.webContents.send(channel, payload);
}
/** Обычный режим — 100 % непрозрачности, полупрозрачный — 35 %. */
function applyOverlayMode(mode, remember = true) {
    prefs.overlayMode = mode;
    if (remember)
        (0, config_1.savePrefs)(prefs);
    if (win && !win.isDestroyed()) {
        win.setOpacity(mode === "dim" ? DIM_OPACITY : 1);
    }
    sendToRenderer(ipc_1.CH.overlayMode, mode);
    rebuildTrayMenu();
}
/** Компактный (только столбики) ↔ диалог (история + ввод). */
function applyUiMode(mode, remember = true) {
    prefs.uiMode = mode;
    if (remember)
        (0, config_1.savePrefs)(prefs);
    if (win && !win.isDestroyed()) {
        const bounds = win.getBounds();
        const size = SIZES[mode];
        const { workArea } = electron_1.screen.getDisplayMatching(bounds);
        // Якорь — правый нижний угол: окно растёт вверх и влево, не «убегая».
        let x = bounds.x + bounds.width - size.width;
        let y = bounds.y + bounds.height - size.height;
        x = Math.min(Math.max(x, workArea.x), workArea.x + workArea.width - size.width);
        y = Math.min(Math.max(y, workArea.y), workArea.y + workArea.height - size.height);
        win.setBounds({ x, y, width: size.width, height: size.height });
        // В диалоге пользователь явно хочет взаимодействовать — снимаем сквозной
        // режим сразу, не дожидаясь mousemove. При сворачивании renderer вернёт его.
        if (mode === "dialog")
            win.setIgnoreMouseEvents(false);
    }
    sendToRenderer(ipc_1.CH.uiMode, mode);
    rebuildTrayMenu();
}
function toggleOverlayMode() {
    applyOverlayMode(prefs.overlayMode === "dim" ? "normal" : "dim");
}
function toggleUiMode() {
    applyUiMode(prefs.uiMode === "dialog" ? "compact" : "dialog");
}
function toggleVisible() {
    if (!win)
        return;
    if (win.isVisible())
        win.hide();
    else {
        win.showInactive();
        win.setAlwaysOnTop(true, "screen-saver");
    }
}
// --------------------------------------------------------------- трей и меню
function assetPath(name) {
    // dev: <проект>/desktop/assets, prod: <установка>/resources/assets
    if (electron_1.app.isPackaged)
        return node_path_1.default.join(process.resourcesPath, "assets", name);
    return node_path_1.default.resolve(__dirname, "..", "..", "assets", name);
}
function trayIcon() {
    const file = assetPath("tray.png");
    const image = electron_1.nativeImage.createFromPath(file);
    if (image.isEmpty())
        (0, log_1.log)("warn", `Иконка трея не найдена: ${file}`);
    return image;
}
function createTray() {
    const icon = trayIcon();
    if (icon.isEmpty())
        return; // без иконки Tray на Windows бросает исключение
    tray = new electron_1.Tray(icon);
    tray.on("click", toggleVisible);
    rebuildTrayMenu();
}
function rebuildTrayMenu() {
    if (!tray)
        return;
    const menu = electron_1.Menu.buildFromTemplate([
        { label: "Показать / скрыть", click: toggleVisible },
        { type: "separator" },
        {
            label: "Обычный режим",
            type: "radio",
            checked: prefs.overlayMode === "normal",
            click: () => applyOverlayMode("normal"),
        },
        {
            label: "Полупрозрачный режим",
            type: "radio",
            checked: prefs.overlayMode === "dim",
            click: () => applyOverlayMode("dim"),
        },
        { type: "separator" },
        {
            label: "Диалог",
            type: "radio",
            checked: prefs.uiMode === "dialog",
            click: () => applyUiMode("dialog"),
        },
        {
            label: "Компактный вид",
            type: "radio",
            checked: prefs.uiMode === "compact",
            click: () => applyUiMode("compact"),
        },
        { type: "separator" },
        { label: "Прервать речь", click: () => void interrupt() },
        { type: "separator" },
        { label: "Выход", click: () => electron_1.app.quit() },
    ]);
    tray.setContextMenu(menu);
    tray.setToolTip(`Пятница — ${apiReady ? "ядро на связи" : "ядро не отвечает"}`);
}
function registerShortcuts() {
    const bind = (accelerator, action) => {
        if (!electron_1.globalShortcut.register(accelerator, action)) {
            (0, log_1.log)("warn", `Горячая клавиша занята: ${accelerator}`);
        }
    };
    bind("Control+Alt+J", toggleOverlayMode);
    bind("Control+Alt+U", toggleUiMode);
    bind("Control+Alt+Space", () => void interrupt());
}
// ------------------------------------------------------------- связь с Python
async function waitForApi(timeoutMs = 90_000) {
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        try {
            const res = await fetch(`${config_1.apiBase}/api/health`, { signal: AbortSignal.timeout(1500) });
            if (res.ok)
                return true;
        }
        catch {
            /* ядро ещё грузит модели — это нормально, ждём */
        }
        await sleep(500);
    }
    return false;
}
function publishSidecar(status) {
    sendToRenderer(ipc_1.CH.sidecar, status);
}
async function connectToCore() {
    publishSidecar({ state: "starting", detail: "ждём /api/health" });
    apiReady = await waitForApi();
    if (!apiReady) {
        (0, log_1.log)("error", "Python-ядро не поднялось за отведённое время");
        publishSidecar({ state: "error", detail: "ядро не отвечает" });
        rebuildTrayMenu();
        return;
    }
    (0, log_1.log)("info", `Ядро на связи: ${config_1.apiBase}`);
    publishSidecar({ state: "ready", detail: config_1.apiBase });
    rebuildTrayMenu();
    sse = new sse_1.SseClient(`${config_1.apiBase}/api/stream`, (event) => sendToRenderer(ipc_1.CH.event, event), (open) => {
        if (!open) {
            (0, log_1.log)("warn", "Поток событий отключился — переподключаюсь");
            publishSidecar({ state: "starting", detail: "поток событий переподключается" });
        }
        else {
            (0, log_1.log)("info", "Поток событий подключён");
            publishSidecar({ state: "ready", detail: config_1.apiBase });
        }
    });
    sse.start();
}
async function postJson(pathname, body) {
    try {
        const res = await fetch(`${config_1.apiBase}${pathname}`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(body ?? {}),
            signal: AbortSignal.timeout(5000),
        });
        if (res.ok)
            return { ok: true };
        const data = (await res.json().catch(() => ({})));
        return { ok: false, error: data.error ?? `HTTP ${res.status}` };
    }
    catch (error) {
        return { ok: false, error: error instanceof Error ? error.message : String(error) };
    }
}
async function interrupt() {
    await postJson("/api/interrupt");
}
// ------------------------------------------------------------------- IPC
function stubPath() {
    const candidates = [
        node_path_1.default.join(__dirname, "..", "renderer", "test-voice.wav"), // прод: Vite скопировал public/
        node_path_1.default.join((0, config_1.pythonRoot)(), "desktop", "src", "renderer", "public", "test-voice.wav"), // dev
    ];
    return candidates.find((file) => node_fs_1.default.existsSync(file)) ?? candidates[0];
}
function readAudioFile(file, label) {
    try {
        const buffer = node_fs_1.default.readFileSync(file);
        if (buffer.length < 1000)
            return null;
        return { base64: buffer.toString("base64"), mime: "audio/wav", label };
    }
    catch {
        return null;
    }
}
function registerIpc() {
    electron_1.ipcMain.handle(ipc_1.CH.getConfig, () => ({
        apiBase: config_1.apiBase,
        overlayMode: prefs.overlayMode,
        uiMode: prefs.uiMode,
        dev: config_1.isDev,
        ttsBridge: true,
    }));
    electron_1.ipcMain.handle(ipc_1.CH.sendMessage, async (_event, raw) => {
        const text = typeof raw === "string" ? raw.trim() : "";
        if (!text)
            return { ok: false, error: "пустой текст" };
        return postJson("/api/message", { text });
    });
    electron_1.ipcMain.handle(ipc_1.CH.reset, () => postJson("/api/reset"));
    electron_1.ipcMain.on(ipc_1.CH.interrupt, () => void interrupt());
    // Тестовая заглушка: тот же путь, что и у настоящего WAV.
    electron_1.ipcMain.handle(ipc_1.CH.playStub, () => {
        const file = stubPath();
        const payload = readAudioFile(file, "stub");
        if (!payload)
            (0, log_1.log)("warn", `Не нашёл тестовый WAV: ${file}`);
        return payload;
    });
    // Настоящий Piper: ждём, пока Python допишет файл в кэш.
    electron_1.ipcMain.handle(ipc_1.CH.playTts, async (_event, raw) => {
        const text = typeof raw === "string" ? raw : "";
        if (!text)
            return null;
        const buffer = await (0, ttsAudio_1.waitForWav)(text);
        if (!buffer) {
            (0, log_1.log)("debug", "WAV для фразы не появился — визуализатор возьмёт уровни из SSE");
            return null;
        }
        return { base64: buffer.toString("base64"), mime: "audio/wav", label: "piper" };
    });
    // Самый частый вызов: приходит на каждое движение мыши.
    electron_1.ipcMain.on(ipc_1.CH.setIgnoreMouse, (_event, raw) => {
        if (!win || win.isDestroyed())
            return;
        win.setIgnoreMouseEvents(raw === true, { forward: true });
    });
    electron_1.ipcMain.on(ipc_1.CH.setUiMode, (_event, raw) => {
        applyUiMode(raw === "dialog" ? "dialog" : "compact");
    });
    electron_1.ipcMain.on(ipc_1.CH.setOverlayMode, (_event, raw) => {
        applyOverlayMode(raw === "dim" ? "dim" : "normal");
    });
    electron_1.ipcMain.on(ipc_1.CH.quit, () => electron_1.app.quit());
}
// ------------------------------------------------------------------ запуск
const gotLock = electron_1.app.requestSingleInstanceLock();
if (!gotLock) {
    electron_1.app.quit();
}
else {
    electron_1.app.on("second-instance", () => {
        if (!win)
            return;
        win.showInactive();
        win.setAlwaysOnTop(true, "screen-saver");
    });
    void electron_1.app.whenReady().then(async () => {
        prefs = (0, config_1.loadPrefs)();
        registerIpc();
        createWindow();
        createTray();
        registerShortcuts();
        (0, log_1.log)("info", `Запуск: dev=${config_1.isDev}, api=${config_1.apiBase}, python=${(0, config_1.pythonRoot)()}`);
        sidecar = new sidecar_1.Sidecar(publishSidecar);
        sidecar.start();
        // Не ждём: окно уже на экране, а модели Vosk/Piper грузятся несколько секунд.
        void connectToCore();
    });
}
electron_1.app.on("window-all-closed", () => {
    // Оверлей без окна бессмысленен — выходим, как обычное приложение.
    electron_1.app.quit();
});
// Слабый/кривой драйвер видеокарты иногда роняет renderer при создании
// GL-контекста. Один раз перезапускаемся в программном режиме — на глаз
// разница только в плавности анимации.
electron_1.app.on("render-process-gone", (_event, _contents, details) => {
    (0, log_1.log)("error", `Renderer упал: ${details.reason}`);
    if (quitting)
        return; // при закрытии приложения падение renderer-а — норма
    const alreadySoftware = process.argv.includes("--disable-gpu");
    if (!alreadySoftware && details.reason === "crashed") {
        (0, log_1.log)("warn", "Перезапуск с --disable-gpu");
        electron_1.app.relaunch({ args: [...process.argv.slice(1), "--disable-gpu"] });
        electron_1.app.exit(0);
    }
});
electron_1.app.on("before-quit", () => {
    if (quitting)
        return;
    quitting = true;
    electron_1.globalShortcut.unregisterAll();
    sse?.stop();
    sse = null;
    sidecar?.stop();
    sidecar = null;
    (0, log_1.log)("info", "Приложение закрывается");
});
//# sourceMappingURL=main.js.map