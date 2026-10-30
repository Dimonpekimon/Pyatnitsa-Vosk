/**
 * Подсказка «нужен ли оклик».
 *
 * Проблема, которую она решает: ядро принимает первую команду только после
 * слова «Пятница», а следующие DIALOG_FOLLOW_UP_SEC секунд — уже без него.
 * Со стороны человека это невидимо, поэтому он говорит «Пятница» на всякий
 * случай и повторяет команду дважды.
 *
 * Отсюда два состояния и два вида:
 *  - compact — маленькая пилюля в углу визуализатора, видно боковым зрением;
 *  - dialog  — строка под шапкой с той же мыслью словами и справкой «?».
 *
 * Отсчёт ведётся локально по `until` из события: ядро присылает только момент
 * закрытия окна, а тикать 4 раза в секунду по шине незачем.
 */

import { useEffect, useState } from "react";
import type { JSX } from "react";

interface Props {
  /** compact — пилюля поверх визуализатора, dialog — строка в диалоговой панели. */
  variant: "compact" | "dialog";
  /** Момент закрытия окна (epoch, мс); 0 — окно закрыто, нужен оклик. */
  until: number;
  /** Нужен ли оклик вообще: `--no-wake` снимает требование совсем. */
  wakeRequired: boolean;
  /** Есть ли голосовой ввод: в текстовом режиме подсказка бессмысленна. */
  voice?: boolean;
  /** Длина окна в секундах — для текста справки (по умолчанию 12). */
  windowSec?: number;
}

/** Сколько секунд осталось до `until` (0 — окно закрыто или уже истекло). */
function secondsLeft(until: number): number {
  if (!until) return 0;
  return Math.max(0, Math.ceil((until - Date.now()) / 1000));
}

/** Правила оклика — коротко, чтобы справка влезала в окно без прокрутки. */
function rules(windowSec: number): string[] {
  return [
    "Первая команда — с обращением: «Пятница, который час». Обращение и команда могут быть одной фразой.",
    "Сказали только «Пятница» — ответит «Да, сэр?» и подождёт команду 8 секунд. Повторять обращение не нужно.",
    `После ответа ${windowSec} секунд можно говорить без оклика; каждая реплика продлевает окно.`,
    "Тишина до конца окна или слово «хватит» — окно закрывается, снова нужен оклик.",
    "В окне жду команды и продолжения («а завтра?», «ещё громче»). Посторонняя фраза закрывает окно.",
  ];
}

export default function WakeCue({ variant, until, wakeRequired, voice = true, windowSec = 12 }: Props): JSX.Element | null {
  const [left, setLeft] = useState<number>(() => secondsLeft(until));
  const [rulesOpen, setRulesOpen] = useState(false);

  useEffect(() => {
    setLeft(secondsLeft(until));
    if (!until) return;
    const timer = window.setInterval(() => {
      const next = secondsLeft(until);
      setLeft(next);
      if (next <= 0) window.clearInterval(timer);
    }, 250);
    return () => window.clearInterval(timer);
  }, [until]);

  // В текстовом режиме микрофона нет — говорить «скажи «Пятница»» некому.
  if (!voice) return null;

  const open = wakeRequired && left > 0;

  let tone: string;
  let text: string;
  let title: string;

  if (!wakeRequired) {
    tone = "cue--open";
    text = variant === "compact" ? "оклик не нужен" : "Оклик не нужен — говорите сразу.";
    title = "Ассистент запущен с --no-wake: слово «Пятница» не требуется.";
  } else if (open) {
    tone = "cue--open";
    text = variant === "compact"
      ? `без оклика · ${left} с`
      : `Можно без оклика — ещё ${left} с. Говорите сразу.`;
    title = `Окно разговора открыто после ответа (${windowSec} с): продолжение принимается без слова «Пятница», пока не станет тихо.`;
  } else {
    tone = "cue--closed";
    text = variant === "compact" ? "скажи «Пятница»" : "Сейчас нужен оклик: «Пятница, …» — дальше можно без него.";
    title = `Скажите «Пятница» и команду одной фразой. После ответа ${windowSec} с можно говорить без обращения.`;
  }

  if (variant === "compact") {
    return (
      <div className={`cue cue--compact ${tone}`} title={title}>
        <span className="cue__dot" />
        <span className="cue__text">{text}</span>
      </div>
    );
  }

  return (
    <div className="cue-wrap">
      <div className={`cue cue--dialog ${tone}`} title={title}>
        <span className="cue__dot" />
        <span className="cue__text">{text}</span>
        <button
          type="button"
          className={`cue__help ${rulesOpen ? "cue__help--on" : ""}`}
          title="Правила оклика"
          aria-expanded={rulesOpen}
          onClick={() => setRulesOpen((prev) => !prev)}
        >
          ?
        </button>
      </div>

      {rulesOpen && (
        <ul className="cue__rules">
          {rules(windowSec).map((rule) => (
            <li key={rule}>{rule}</li>
          ))}
        </ul>
      )}
    </div>
  );
}
