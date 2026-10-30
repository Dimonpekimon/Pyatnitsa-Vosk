"use strict";
/**
 * Контракт между main-процессом и renderer-ом.
 *
 * Один файл на обе стороны — чтобы имена каналов и типы не разъезжались.
 * Renderer не имеет доступа к Node (contextIsolation: true), поэтому всё
 * общение идёт через узкий мост, описанный в preload.ts.
 */
Object.defineProperty(exports, "__esModule", { value: true });
exports.DEFAULT_API_BASE = exports.CH = void 0;
/** Каналы IPC. Префиксы: pyatnitsa: — данные от Python, ui: — команды интерфейса. */
exports.CH = {
    // main → renderer
    event: "pyatnitsa:event", // событие шины Python (message/state/level)
    audio: "pyatnitsa:audio", // готовый WAV в base64
    sidecar: "pyatnitsa:sidecar", // состояние процесса Python
    uiMode: "pyatnitsa:ui-mode", // compact | dialog
    overlayMode: "pyatnitsa:overlay-mode", // normal | dim
    // renderer → main (invoke)
    getConfig: "app:get-config",
    sendMessage: "pyatnitsa:message",
    interrupt: "pyatnitsa:interrupt",
    reset: "pyatnitsa:reset",
    playStub: "audio:play-stub",
    playTts: "audio:play-tts",
    // renderer → main (send, без ответа — важно для mousemove)
    setIgnoreMouse: "ui:ignore-mouse",
    setUiMode: "ui:set-mode",
    setOverlayMode: "ui:set-overlay-mode",
    quit: "app:quit",
};
exports.DEFAULT_API_BASE = "http://127.0.0.1:8765";
//# sourceMappingURL=ipc.js.map