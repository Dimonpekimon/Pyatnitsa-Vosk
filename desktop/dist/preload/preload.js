"use strict";
/**
 * Мост между изолированным renderer-ом и main-процессом.
 *
 * contextIsolation: true + nodeIntegration: false означают, что у страницы нет
 * ни require, ни process, ни fs. Всё, что ей разрешено, перечислено здесь —
 * это единственная точка расширения прав.
 */
Object.defineProperty(exports, "__esModule", { value: true });
const electron_1 = require("electron");
const ipc_1 = require("../shared/ipc");
/** Подписка с отпиской: React-эффект должен уметь снимать слушателя. */
function subscribe(channel, callback) {
    const listener = (_event, payload) => callback(payload);
    electron_1.ipcRenderer.on(channel, listener);
    return () => {
        electron_1.ipcRenderer.removeListener(channel, listener);
    };
}
const bridge = {
    getConfig: () => electron_1.ipcRenderer.invoke(ipc_1.CH.getConfig),
    sendMessage: (text) => electron_1.ipcRenderer.invoke(ipc_1.CH.sendMessage, text),
    interrupt: () => electron_1.ipcRenderer.send(ipc_1.CH.interrupt),
    reset: () => electron_1.ipcRenderer.invoke(ipc_1.CH.reset),
    playStub: () => electron_1.ipcRenderer.invoke(ipc_1.CH.playStub),
    playTts: (text) => electron_1.ipcRenderer.invoke(ipc_1.CH.playTts, text),
    // send, а не invoke: эти вызовы идут десятки раз в секунду (mousemove),
    // и ждать ответа на каждый незачем.
    setIgnoreMouse: (ignore) => electron_1.ipcRenderer.send(ipc_1.CH.setIgnoreMouse, ignore),
    setUiMode: (mode) => electron_1.ipcRenderer.send(ipc_1.CH.setUiMode, mode),
    setOverlayMode: (mode) => electron_1.ipcRenderer.send(ipc_1.CH.setOverlayMode, mode),
    quit: () => electron_1.ipcRenderer.send(ipc_1.CH.quit),
    onEvent: (cb) => subscribe(ipc_1.CH.event, cb),
    onAudio: (cb) => subscribe(ipc_1.CH.audio, cb),
    onSidecar: (cb) => subscribe(ipc_1.CH.sidecar, cb),
    onUiMode: (cb) => subscribe(ipc_1.CH.uiMode, cb),
    onOverlayMode: (cb) => subscribe(ipc_1.CH.overlayMode, cb),
};
electron_1.contextBridge.exposeInMainWorld("pyatnitsa", bridge);
//# sourceMappingURL=preload.js.map