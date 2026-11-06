//! CLI мозга: разовый вопрос, диагностика, REPL и режим `stdio` для Python.
//!
//! Примеры:
//! ```text
//! pyatnitsa-brain check
//! pyatnitsa-brain ask "Сколько будет 2+2?"
//! pyatnitsa-brain ask --image shot.png "Что на экране?"
//! pyatnitsa-brain stdio        # построчный JSON, см. brain_rs.py
//! ```

use std::io::{BufRead, Write};
use std::path::PathBuf;
use std::process::ExitCode;

use pyatnitsa_brain::config::Settings;
use pyatnitsa_brain::{AskOptions, Brain, BrainError, VERSION};
use serde_json::{json, Value};

const USAGE: &str = "\
pyatnitsa-brain — мозг Пятницы на Rust

Использование:
  pyatnitsa-brain [--settings FILE] [--debug] <команда> [аргументы]

Команды:
  ask [--no-remember] [--model M] [--temperature T] [--image FILE] <вопрос>
  vision <файл> [вопрос]
  check [--force]
  models
  repl
  stdio

Флаги:
  --settings FILE   путь к pyatnitsa_settings.json
  --debug           подробный лог в stderr
  --json            машинный вывод
  --version         версия
  -h, --help        эта справка
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.iter().any(|arg| arg == "--version" || arg == "-V") {
        println!("pyatnitsa-brain {VERSION}");
        return ExitCode::SUCCESS;
    }
    if args.is_empty() || args.iter().any(|arg| arg == "-h" || arg == "--help") {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }

    let mut settings_path: Option<PathBuf> = None;
    let mut debug = false;
    let mut as_json = false;
    let mut rest: Vec<String> = Vec::new();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--settings" => {
                index += 1;
                if let Some(path) = args.get(index) {
                    settings_path = Some(PathBuf::from(path));
                }
            }
            "--debug" => debug = true,
            "--json" => as_json = true,
            other => rest.push(other.to_string()),
        }
        index += 1;
    }

    let Some(command) = rest.first().cloned() else {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    };
    let tail = &rest[1..];

    let settings = Settings::load(settings_path.as_deref());
    let mut brain = Brain::with_settings(settings);
    brain.debug = debug;

    let result = match command.as_str() {
        "ask" => cmd_ask(&mut brain, tail, as_json),
        "vision" => cmd_vision(&mut brain, tail, as_json),
        "check" => cmd_check(&mut brain, tail, as_json),
        "models" => cmd_models(&mut brain, as_json),
        "repl" => cmd_repl(&mut brain),
        "stdio" => cmd_stdio(&mut brain),
        other => {
            eprintln!("Неизвестная команда: {other}\n\n{USAGE}");
            return ExitCode::from(2);
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("{}", err.message());
            ExitCode::FAILURE
        }
    }
}

fn cmd_ask(brain: &mut Brain, args: &[String], as_json: bool) -> Result<(), BrainError> {
    let mut remember = true;
    let mut model: Option<String> = None;
    let mut temperature: Option<f64> = None;
    let mut image: Option<String> = None;
    let mut words: Vec<String> = Vec::new();

    let mut index = 0;
    while index < args.len() {
        match args[index].as_str() {
            "--no-remember" => remember = false,
            "--model" => {
                index += 1;
                model = args.get(index).cloned();
            }
            "--temperature" => {
                index += 1;
                temperature = args.get(index).and_then(|value| value.parse::<f64>().ok());
            }
            "--image" => {
                index += 1;
                image = args.get(index).cloned();
            }
            other => words.push(other.to_string()),
        }
        index += 1;
    }

    let prompt = words.join(" ");
    let mut options = AskOptions {
        remember,
        image_mime: "image/png".to_string(),
        model,
        temperature,
        image_b64: None,
    };

    if let Some(path) = image {
        let (data, mime) = pyatnitsa_brain::vision::prepare_image(std::path::Path::new(&path))?;
        options.image_b64 = Some(data);
        options.image_mime = mime;
    }

    let answer = brain.ask(&prompt, options)?;
    if as_json {
        print_json(&json!({"ok": true, "answer": answer, "model": brain.model()}));
    } else {
        println!("{answer}");
    }
    Ok(())
}

fn cmd_vision(brain: &mut Brain, args: &[String], as_json: bool) -> Result<(), BrainError> {
    let Some(path) = args.first() else {
        return Err(BrainError::Io(
            "Укажите файл: pyatnitsa-brain vision <файл> [вопрос]".to_string(),
        ));
    };
    let prompt = if args.len() > 1 {
        args[1..].join(" ")
    } else {
        "Что на этом изображении?".to_string()
    };
    let answer = brain.ask_with_image(&prompt, std::path::Path::new(path))?;
    if as_json {
        print_json(&json!({"ok": true, "answer": answer}));
    } else {
        println!("{answer}");
    }
    Ok(())
}

fn cmd_check(brain: &mut Brain, args: &[String], as_json: bool) -> Result<(), BrainError> {
    let force = args.iter().any(|arg| arg == "--force");
    let (ok, message) = brain.check(force);
    if as_json {
        print_json(&json!({
            "ok": ok,
            "message": message,
            "provider": brain.provider().as_str(),
            "model": brain.model(),
            "api_base": brain.api_base(),
        }));
    } else {
        println!("{message}");
    }
    if ok {
        Ok(())
    } else {
        Err(BrainError::Unavailable(message))
    }
}

fn cmd_models(brain: &mut Brain, as_json: bool) -> Result<(), BrainError> {
    let models = brain.available_models(true);
    if as_json {
        print_json(&json!({"ok": !models.is_empty(), "models": models}));
    } else if models.is_empty() {
        println!("{}", brain.last_error());
    } else {
        for model in models {
            println!("{model}");
        }
    }
    Ok(())
}

fn cmd_repl(brain: &mut Brain) -> Result<(), BrainError> {
    println!("Мозг на связи. Пустая строка или «выход» — конец. /reset — забыть историю.");
    let stdin = std::io::stdin();
    let mut line = String::new();
    loop {
        print!("> ");
        let _ = std::io::stdout().flush();
        line.clear();
        if stdin.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        let prompt = line.trim();
        if prompt.is_empty() || prompt.eq_ignore_ascii_case("exit") || prompt == "выход" {
            break;
        }
        if prompt == "/reset" {
            brain.reset();
            println!("История очищена.");
            continue;
        }
        match brain.ask_simple(prompt) {
            Ok(answer) => println!("{answer}"),
            Err(err) => eprintln!("{}", err.message()),
        }
    }
    Ok(())
}

/// Построчный JSON-протокол для моста `brain_rs.py`.
///
/// Запрос:  `{"id": 1, "op": "ask", "prompt": "...", "remember": true}`
/// Ответ:   `{"id": 1, "ok": true, "answer": "..."}`.
fn cmd_stdio(brain: &mut Brain) -> Result<(), BrainError> {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    let mut writer = stdout.lock();

    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        let request: Value = match serde_json::from_str(trimmed) {
            Ok(value) => value,
            Err(err) => {
                let reply = json!({"id": null, "ok": false, "error": format!("плохой JSON: {err}")});
                let _ = writeln!(writer, "{reply}");
                let _ = writer.flush();
                continue;
            }
        };

        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let op = request
            .get("op")
            .and_then(|value| value.as_str())
            .unwrap_or("ask");

        let reply = match op {
            "ping" => json!({"id": id, "ok": true, "pong": true, "version": VERSION}),
            "ask" => handle_ask(brain, &request, id),
            "check" => {
                let (ok, message) = brain.check(false);
                json!({"id": id, "ok": ok, "message": message, "model": brain.model()})
            }
            "models" => {
                let models = brain.available_models(false);
                json!({"id": id, "ok": !models.is_empty(), "models": models})
            }
            "reset" => {
                brain.reset();
                json!({"id": id, "ok": true})
            }
            "history" => {
                let items: Vec<Value> = brain
                    .history()
                    .iter()
                    .map(|(question, answer)| json!({"q": question, "a": answer}))
                    .collect();
                json!({"id": id, "ok": true, "history": items})
            }
            other => json!({"id": id, "ok": false, "error": format!("неизвестная операция: {other}")}),
        };

        let _ = writeln!(writer, "{reply}");
        let _ = writer.flush();
    }
    Ok(())
}

fn handle_ask(brain: &mut Brain, request: &Value, id: Value) -> Value {
    let prompt = request
        .get("prompt")
        .and_then(|value| value.as_str())
        .unwrap_or_default()
        .to_string();
    let mut options = AskOptions {
        remember: request
            .get("remember")
            .and_then(|value| value.as_bool())
            .unwrap_or(true),
        image_mime: request
            .get("image_mime")
            .and_then(|value| value.as_str())
            .unwrap_or("image/png")
            .to_string(),
        model: request
            .get("model")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
        temperature: request.get("temperature").and_then(|value| value.as_f64()),
        image_b64: request
            .get("image_b64")
            .and_then(|value| value.as_str())
            .map(|value| value.to_string()),
    };

    if let Some(path) = request.get("image_path").and_then(|value| value.as_str()) {
        match pyatnitsa_brain::vision::prepare_image(std::path::Path::new(path)) {
            Ok((data, mime)) => {
                options.image_b64 = Some(data);
                options.image_mime = mime;
            }
            Err(err) => {
                return json!({"id": id, "ok": false, "error": err.message(), "kind": "io"})
            }
        }
    }

    match brain.ask(&prompt, options) {
        Ok(answer) => json!({"id": id, "ok": true, "answer": answer, "model": brain.model()}),
        Err(BrainError::Unavailable(message)) => {
            json!({"id": id, "ok": false, "error": message, "kind": "unavailable"})
        }
        Err(BrainError::NextModel(message)) => {
            json!({"id": id, "ok": false, "error": message, "kind": "next_model"})
        }
        Err(BrainError::Io(message)) => json!({"id": id, "ok": false, "error": message, "kind": "io"}),
    }
}

fn print_json(value: &Value) {
    println!("{value}");
}
