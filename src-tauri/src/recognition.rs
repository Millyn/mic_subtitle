use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tauri::{AppHandle, Emitter, Manager};

use crate::audio;
use crate::models::model_path;
use crate::state::{AppState, SubtitleEvent};
use crate::translation::publish_with_translation;

#[derive(Debug, Deserialize)]
struct WorkerEvent {
    #[serde(rename = "type")]
    event_type: Option<String>,
    message: Option<String>,
    kind: Option<String>,
    text: Option<String>,
    chinese: Option<String>,
    error: Option<String>,
    level: Option<f32>,
    #[serde(rename = "asrTokens")]
    asr_tokens: Option<u64>,
    #[serde(rename = "asrTokensEstimated")]
    asr_tokens_estimated: Option<bool>,
}

pub async fn start(
    app: AppHandle,
    state: &AppState,
    device_name: Option<String>,
) -> Result<(), String> {
    {
        let child = state
            .recognition_child
            .lock()
            .map_err(|_| "识别进程状态锁已损坏".to_string())?;
        if child.is_some() {
            return Err("识别已经在运行中".into());
        }
    }
    let config = state.config.read().await.clone();
    let model_id = config.active_model.clone();
    let installed = config
        .models
        .get(&model_id)
        .map(|model| model.installed)
        .unwrap_or(false);
    let path = model_path(state, &model_id);
    if !installed || !path.exists() {
        return Err(format!(
            "模型「{model_id}」尚未下载或校验，请先在语音模型页面完成安装"
        ));
    }
    let session_token_usage = state.reset_token_session().await;

    let mut command = worker_command(&app, &path, device_name.as_deref())?;
    let diagnostics_log = open_diagnostics_log();
    write_diagnostic(
        &diagnostics_log,
        "host",
        &format!(
            "准备启动 worker：model={}，device={:?}，command={command:?}",
            path.display(),
            device_name
        ),
    );
    let mut child = command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| format!("无法启动本地语音识别 sidecar：{error}"))?;
    write_diagnostic(&diagnostics_log, "host", "worker 进程已启动");
    let stdout = match child.stdout.take() {
        Some(stdout) => stdout,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("识别进程没有输出通道".into());
        }
    };
    let stdin = match child.stdin.take() {
        Some(stdin) => stdin,
        None => {
            let _ = child.kill();
            let _ = child.wait();
            return Err("识别进程没有输入通道".into());
        }
    };
    let startup_stderr = Arc::new(Mutex::new(Vec::<String>::new()));
    if let Some(stderr) = child.stderr.take() {
        let startup_stderr = startup_stderr.clone();
        let diagnostics_log = diagnostics_log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines().flatten() {
                eprintln!("[whisper-worker] {line}");
                write_diagnostic(&diagnostics_log, "stderr", &line);
                if let Ok(mut lines) = startup_stderr.lock() {
                    if lines.len() < 20 {
                        lines.push(line);
                    }
                }
            }
        });
    }
    let recognition_capture =
        match audio::start_pipe_capture(app.clone(), state, device_name.clone(), stdin) {
            Ok(capture) => capture,
            Err(error) => {
                write_diagnostic(
                    &diagnostics_log,
                    "host",
                    &format!("无法启动 Rust 音频管道：{error}"),
                );
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
    let (startup_tx, startup_rx) = mpsc::sync_channel(1);
    let mut child_slot = match state.recognition_child.lock() {
        Ok(slot) => slot,
        Err(_) => {
            let _ = child.kill();
            let _ = child.wait();
            drop(recognition_capture);
            return Err("识别进程状态锁已损坏".into());
        }
    };
    *child_slot = Some(child);
    drop(child_slot);
    let mut capture_slot = match state.recognition_capture.lock() {
        Ok(slot) => slot,
        Err(_) => {
            stop(state);
            drop(recognition_capture);
            return Err("识别音频状态锁已损坏".into());
        }
    };
    if capture_slot.is_some() {
        drop(capture_slot);
        stop(state);
        drop(recognition_capture);
        return Err("上一次识别音频流尚未释放，请稍后重试".into());
    }
    *capture_slot = Some(recognition_capture);
    drop(capture_slot);

    let worker_state = state.clone();
    let worker_app = app.clone();
    let startup_stderr_for_reader = startup_stderr.clone();
    let diagnostics_log_for_reader = diagnostics_log.clone();
    std::thread::spawn(move || {
        let mut startup_tx = Some(startup_tx);
        for line in BufReader::new(stdout).lines().flatten() {
            if !line.contains("\"type\":\"audio-level\"")
                && !line.contains("\"type\": \"audio-level\"")
            {
                write_diagnostic(&diagnostics_log_for_reader, "stdout", &line);
            }
            match serde_json::from_str::<WorkerEvent>(&line) {
                Ok(event) => {
                    if event.event_type.as_deref() == Some("ready") {
                        if let Some(tx) = startup_tx.take() {
                            let _ = tx.send(Ok(()));
                        }
                        continue;
                    }
                    if event.event_type.as_deref() == Some("status") {
                        if let Some(message) = event.message {
                            let _ = worker_app.emit("recognition-status", message);
                        }
                        continue;
                    }
                    if event.event_type.as_deref() == Some("audio-level") {
                        if let Some(level) = event.level {
                            let _ = worker_app.emit("audio-level", level.clamp(0.0, 1.0));
                        }
                        continue;
                    }
                    if event.event_type.as_deref() == Some("token-usage") {
                        if let Some(tokens) = event.asr_tokens {
                            let usage_state = worker_state.clone();
                            let usage_app = worker_app.clone();
                            let estimated = event.asr_tokens_estimated.unwrap_or(true);
                            tauri::async_runtime::spawn(async move {
                                let usage = usage_state.record_asr_tokens(tokens, estimated).await;
                                let _ = usage_app.emit("token-usage", &usage);
                            });
                        }
                        continue;
                    }
                    if let Some(error) = event.error {
                        eprintln!("[whisper-worker] {error}");
                        if let Some(tx) = startup_tx.take() {
                            let _ = tx.send(Err(error.clone()));
                        }
                        let _ = worker_app.emit("recognition-error", error);
                        continue;
                    }
                    let kind = event.kind.unwrap_or_else(|| "partial".into());
                    let text = event
                        .text
                        .or(event.chinese)
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    if text.is_empty() || (kind != "partial" && kind != "final") {
                        continue;
                    }
                    // The overlay consumes the WebSocket broadcast below, but the
                    // desktop preview listens to Tauri's `subtitle` event. Keep
                    // both consumers on the same event stream so a successful
                    // worker result is visible in the app window as well.
                    let subtitle = SubtitleEvent::new(kind, text);
                    let _ = worker_app.emit("subtitle", &subtitle);
                    let app_state = worker_state.clone();
                    let subtitle_app = worker_app.clone();
                    tauri::async_runtime::spawn(async move {
                        let (translated, translation_result) =
                            publish_with_translation(&app_state, subtitle).await;
                        if let Some(result) = translation_result {
                            let usage = app_state
                                .record_translation_tokens(
                                    result.prompt_tokens,
                                    result.completion_tokens,
                                    result.total_tokens,
                                    result.usage_estimated,
                                )
                                .await;
                            let _ = subtitle_app.emit("token-usage", &usage);
                        }
                        // A final subtitle is first delivered immediately so the
                        // Chinese text is never held up by a network translation.
                        // Send one more UI event only when translation added a
                        // result or an actionable error to the same subtitle.
                        if translated.kind == "final"
                            && (translated.english.is_some()
                                || translated.translation_error.is_some())
                        {
                            let _ = subtitle_app.emit("subtitle", &translated);
                        }
                    });
                }
                Err(error) => eprintln!("无法解析识别事件：{error} / {line}"),
            }
        }
        if let Some(tx) = startup_tx.take() {
            let detail = startup_stderr_for_reader
                .lock()
                .ok()
                .map(|lines| lines.join("；"))
                .filter(|text| !text.trim().is_empty());
            let message = detail.map_or_else(
                || "识别 worker 在完成初始化前退出，请检查 Python、模型文件和麦克风权限".into(),
                |detail| format!("识别 worker 启动失败：{detail}"),
            );
            let _ = tx.send(Err(message));
        }
        if let Ok(mut child) = worker_state.recognition_child.lock() {
            if let Some(mut process) = child.take() {
                let _ = process.wait();
            }
        }
        let capture = worker_state
            .recognition_capture
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        drop(capture);
    });
    match startup_rx.recv_timeout(Duration::from_secs(180)) {
        Ok(Ok(())) => {
            let _ = app.emit("token-usage", &session_token_usage);
            write_diagnostic(&diagnostics_log, "host", "worker 已完成初始化");
            Ok(())
        }
        Ok(Err(error)) => {
            write_diagnostic(
                &diagnostics_log,
                "host",
                &format!("worker 初始化失败：{error}"),
            );
            stop(state);
            Err(error)
        }
        Err(mpsc::RecvTimeoutError::Timeout) => {
            write_diagnostic(&diagnostics_log, "host", "worker 初始化超时");
            stop(state);
            Err("识别模型加载超过 3 分钟，已停止启动。请确认内存充足，或改用更小的模型".into())
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            write_diagnostic(&diagnostics_log, "host", "worker 初始化通道断开");
            stop(state);
            Err("识别 worker 初始化通道已断开，请检查 Python 运行环境".into())
        }
    }
}

fn open_diagnostics_log() -> Option<Arc<Mutex<File>>> {
    let executable = std::env::current_exe().ok()?;
    let parent = executable.parent()?;
    let file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(parent.join("recognition.log"))
        .ok()?;
    Some(Arc::new(Mutex::new(file)))
}

fn write_diagnostic(log: &Option<Arc<Mutex<File>>>, source: &str, message: &str) {
    let Some(log) = log else {
        return;
    };
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .unwrap_or_default();
    if let Ok(mut file) = log.lock() {
        let _ = writeln!(file, "[{timestamp}] [{source}] {message}");
        let _ = file.flush();
    }
}

pub fn stop(state: &AppState) {
    if let Ok(mut child) = state.recognition_child.lock() {
        if let Some(mut process) = child.take() {
            let _ = process.kill();
            let _ = process.wait();
        }
    }
    let capture = state
        .recognition_capture
        .lock()
        .ok()
        .and_then(|mut slot| slot.take());
    drop(capture);
}

fn worker_command(
    app: &AppHandle,
    model_path: &PathBuf,
    device_name: Option<&str>,
) -> Result<Command, String> {
    let mut command = if let Ok(path) = std::env::var("WHISPER_WORKER_PATH") {
        Command::new(path)
    } else if let Some(path) = bundled_executable(app) {
        Command::new(path)
    } else if let Some(script) = bundled_script(app) {
        let mut command = python_worker_command()?;
        command.arg(script);
        command
    } else {
        return Err(
            "未找到语音识别 worker。请确认程序目录中包含 resources/sidecars/whisper_worker.py"
                .into(),
        );
    };
    command
        .arg("--model-path")
        .arg(model_path)
        .arg("--device")
        .arg("auto")
        .arg("--compute-type")
        .arg("auto")
        .arg("--language")
        .arg("auto")
        .arg("--stdin-audio");
    if let Some(device) = device_name.filter(|name| !name.trim().is_empty()) {
        command.arg("--device-name").arg(device);
    }
    Ok(command)
}

fn python_worker_command() -> Result<Command, String> {
    if let Ok(path) = std::env::var("WHISPER_PYTHON_PATH") {
        if path.trim().is_empty() {
            return Err(
                "WHISPER_PYTHON_PATH 为空，请取消该环境变量或填写 Python 可执行文件路径".into(),
            );
        }
        return Ok(Command::new(path));
    }

    #[cfg(windows)]
    {
        // The trial instructions install dependencies through `py -3 -m pip`.
        // Prefer the same Windows Python launcher when starting the worker;
        // on many machines `python.exe` is only a Microsoft Store alias.
        for (program, launcher_args) in [
            ("py.exe", vec!["-3"]),
            ("python.exe", Vec::new()),
            ("python3.exe", Vec::new()),
        ] {
            if command_available(program) {
                let mut command = Command::new(program);
                command.args(launcher_args);
                return Ok(command);
            }
        }
        return Err(
            "未找到 Python 3。请安装 Python 3.10–3.13，并在 PowerShell 执行 requirements.txt 中的安装命令".into(),
        );
    }

    #[cfg(not(windows))]
    {
        for program in ["python3", "python"] {
            if command_available(program) {
                return Ok(Command::new(program));
            }
        }
        Err("未找到 Python 3，请安装 Python 后再启动识别".into())
    }
}

fn command_available(program: &str) -> bool {
    let locator = if cfg!(windows) { "where.exe" } else { "which" };
    std::process::Command::new(locator)
        .arg(program)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|status| status.success())
        .unwrap_or(false)
}

fn bundled_executable(app: &AppHandle) -> Option<PathBuf> {
    let relative_paths = if cfg!(windows) {
        vec![
            PathBuf::from("sidecars/whisper-worker.exe"),
            PathBuf::from("whisper-worker.exe"),
            PathBuf::from("resources/sidecars/whisper-worker.exe"),
            PathBuf::from("resources/whisper-worker.exe"),
        ]
    } else {
        vec![
            PathBuf::from("sidecars/whisper-worker"),
            PathBuf::from("whisper-worker"),
            PathBuf::from("resources/sidecars/whisper-worker"),
            PathBuf::from("resources/whisper-worker"),
        ]
    };
    resource_roots(app)
        .into_iter()
        .flat_map(|root| {
            relative_paths
                .iter()
                .map(move |relative| root.join(relative))
        })
        .find(|path| path.is_file())
}

fn bundled_script(app: &AppHandle) -> Option<PathBuf> {
    let relative_paths = [
        PathBuf::from("sidecars/whisper_worker.py"),
        PathBuf::from("whisper_worker.py"),
        PathBuf::from("resources/sidecars/whisper_worker.py"),
        PathBuf::from("resources/whisper_worker.py"),
    ];
    resource_roots(app)
        .into_iter()
        .flat_map(|root| {
            relative_paths
                .iter()
                .map(move |relative| root.join(relative))
        })
        .find(|path| path.is_file())
}

fn resource_roots(app: &AppHandle) -> Vec<PathBuf> {
    let mut roots = Vec::new();
    if let Ok(resource_dir) = app.path().resource_dir() {
        roots.push(resource_dir);
    }
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            roots.push(parent.to_path_buf());
        }
    }
    roots.dedup();
    roots
}
