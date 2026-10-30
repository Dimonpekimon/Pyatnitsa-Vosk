"use strict";
/**
 * Сайдкар: Python-ядро ассистента как дочерний процесс.
 *
 * Electron не переписывает Python — он его запускает и общается по HTTP.
 * Если Python не установлен, приложение не падает: остаётся оверлей-заглушка.
 */
Object.defineProperty(exports, "__esModule", { value: true });
exports.Sidecar = void 0;
const node_child_process_1 = require("node:child_process");
const log_1 = require("./log");
const config_1 = require("./config");
/** Модуль, по которому проверяем, что у интерпретатора есть зависимости проекта. */
const PROBE_MODULE = "requests";
class Sidecar {
    onStatus;
    child = null;
    stopping = false;
    candidateIndex = 0;
    failures = 0;
    restartTimer = null;
    constructor(onStatus) {
        this.onStatus = onStatus;
    }
    get running() {
        return this.child !== null;
    }
    start() {
        if (!(0, config_1.sidecarEnabled)()) {
            this.onStatus({ state: "offline", detail: "сайдкар отключён (PYATNITSA_NO_SIDECAR=1)" });
            return;
        }
        if (this.child)
            return;
        this.stopping = false;
        this.spawnCandidate();
    }
    /**
     * Проверяет, что интерпретатор видит зависимости проекта.
     *
     * Без этой проверки в PATH легко поймать «чужой» python: сайдкар падает на
     * `import requests`, Electron перезапускает его по кругу, и в интерфейсе
     * просто «ядро не на связи» без внятной причины.
     */
    probe(candidate) {
        const result = (0, node_child_process_1.spawnSync)(candidate.command, [...candidate.prefix, "-c", `import ${PROBE_MODULE}`], { cwd: (0, config_1.pythonRoot)(), timeout: 8000, windowsHide: true, encoding: "utf8" });
        if (result.error) {
            (0, log_1.log)("debug", `Проба «${candidate.command}»: ${result.error.message}`);
            return false;
        }
        if (result.status !== 0) {
            (0, log_1.log)("debug", `«${candidate.command}» не видит ${PROBE_MODULE}: ${(result.stderr || "").trim().split("\n").pop()}`);
            return false;
        }
        return true;
    }
    /** Пробует кандидатов по очереди: явный путь → venv → python → py -3. */
    spawnCandidate() {
        const candidates = (0, config_1.pythonCandidates)();
        const total = candidates.length;
        let chosen = null;
        while (this.candidateIndex < total) {
            const candidate = candidates[this.candidateIndex];
            if (this.probe(candidate)) {
                chosen = candidate;
                break;
            }
            this.candidateIndex += 1;
        }
        if (!chosen) {
            // Ни один не подошёл — запускаем последнего, чтобы в логе был настоящий
            // traceback Python, а не только наше «не нашёл».
            chosen = candidates[candidates.length - 1];
            (0, log_1.log)("error", `Ни один интерпретатор не видит «${PROBE_MODULE}». Поставьте зависимости: pip install -r requirements.txt`);
        }
        const command = chosen.command;
        const args = [...chosen.prefix, ...chosen.args];
        const cwd = (0, config_1.pythonRoot)();
        (0, log_1.log)("info", `Запускаю сайдкар: ${command} ${args.join(" ")} (cwd=${cwd})`);
        this.onStatus({ state: "starting", detail: `${command} ${args[0]}` });
        let settled = false;
        let child;
        try {
            child = (0, node_child_process_1.spawn)(command, args, {
                cwd,
                windowsHide: true,
                // stdin — открытый канал, а не /dev/null: в режиме --mode text ядро
                // читает input(), и на закрытом stdin получало бы EOF в цикле,
                // крутя процессор на 100 %.
                stdio: ["pipe", "pipe", "pipe"],
                env: { ...process.env, PYTHONIOENCODING: "utf-8", PYTHONUNBUFFERED: "1" },
            });
        }
        catch (error) {
            (0, log_1.log)("error", `spawn упал: ${String(error)}`);
            this.candidateIndex += 1;
            this.spawnCandidate();
            return;
        }
        this.child = child;
        child.on("spawn", () => {
            settled = true;
            this.failures = 0;
            (0, log_1.log)("info", `Сайдкар запущен, pid=${child.pid}`);
            // «ready» объявит main после успешного /api/health.
        });
        child.on("error", (error) => {
            if (settled) {
                (0, log_1.log)("error", `Ошибка сайдкара: ${error.message}`);
                return;
            }
            // ENOENT — этого интерпретатора нет, пробуем следующего кандидата.
            (0, log_1.log)("warn", `Не удалось запустить «${command}»: ${error.message}`);
            this.child = null;
            this.candidateIndex += 1;
            this.spawnCandidate();
        });
        child.stdout?.setEncoding("utf8");
        child.stdout?.on("data", (chunk) => this.forward(chunk, "out"));
        child.stderr?.setEncoding("utf8");
        child.stderr?.on("data", (chunk) => this.forward(chunk, "err"));
        child.on("exit", (code, signal) => {
            this.child = null;
            if (this.stopping) {
                (0, log_1.log)("info", "Сайдкар остановлен");
                return;
            }
            (0, log_1.log)("warn", `Сайдкар завершился: code=${code} signal=${signal}`);
            this.failures += 1;
            if (this.failures > 3) {
                this.onStatus({ state: "error", detail: `Python падает (код ${code})` });
                return;
            }
            this.onStatus({ state: "offline", detail: "перезапуск…" });
            this.restartTimer = setTimeout(() => {
                this.restartTimer = null;
                if (!this.stopping)
                    this.spawnCandidate();
            }, 3000);
        });
    }
    forward(chunk, stream) {
        for (const line of chunk.split(/\r?\n/)) {
            const text = line.trimEnd();
            if (!text)
                continue;
            if (stream === "err" && /traceback|error|ошибк/i.test(text))
                (0, log_1.log)("warn", `py: ${text}`);
            else
                (0, log_1.log)("debug", `py: ${text}`);
        }
    }
    stop() {
        this.stopping = true;
        if (this.restartTimer) {
            clearTimeout(this.restartTimer);
            this.restartTimer = null;
        }
        const child = this.child;
        if (!child?.pid)
            return;
        this.child = null;
        (0, log_1.log)("info", `Останавливаю сайдкар pid=${child.pid}`);
        if (process.platform === "win32") {
            // Python мог запустить потоки/подпроцессы — валим всё дерево.
            (0, node_child_process_1.spawn)("taskkill", ["/pid", String(child.pid), "/T", "/F"], { windowsHide: true });
        }
        else {
            child.kill("SIGTERM");
        }
    }
}
exports.Sidecar = Sidecar;
//# sourceMappingURL=sidecar.js.map