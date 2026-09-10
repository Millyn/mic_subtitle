use futures_util::StreamExt;
use reqwest::header::{ACCEPT, CONTENT_RANGE, RANGE};
use reqwest::StatusCode;
use serde::Serialize;
use sha2::Digest;
use std::path::PathBuf;
#[cfg(target_os = "windows")]
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tauri::{AppHandle, Emitter};
use tokio::io::AsyncWriteExt;
#[cfg(target_os = "windows")]
use tokio::process::Command;
use tokio::time::{sleep, Duration};

use crate::state::{sha256_file, AppState, InstalledModel, ModelFileRecord};

#[derive(Debug, Clone)]
struct RemoteFile {
    name: &'static str,
    url: &'static str,
}

#[derive(Debug, Clone)]
struct CatalogEntry {
    id: &'static str,
    repo: &'static str,
    name: &'static str,
    size: &'static str,
    description: &'static str,
    accuracy: &'static str,
    speed: &'static str,
    recommended: bool,
    files: &'static [RemoteFile],
}

const BASE_FILES: &[RemoteFile] = &[
    RemoteFile {
        name: "config.json",
        url: "config.json?download=true",
    },
    RemoteFile {
        name: "model.bin",
        url: "model.bin?download=true",
    },
    RemoteFile {
        name: "tokenizer.json",
        url: "tokenizer.json?download=true",
    },
    RemoteFile {
        name: "vocabulary.txt",
        url: "vocabulary.txt?download=true",
    },
];

const TURBO_FILES: &[RemoteFile] = &[
    RemoteFile {
        name: "config.json",
        url: "config.json?download=true",
    },
    RemoteFile {
        name: "model.bin",
        url: "model.bin?download=true",
    },
    RemoteFile {
        name: "tokenizer.json",
        url: "tokenizer.json?download=true",
    },
    RemoteFile {
        name: "vocabulary.json",
        url: "vocabulary.json?download=true",
    },
    RemoteFile {
        name: "preprocessor_config.json",
        url: "preprocessor_config.json?download=true",
    },
];

const QWEN_0_6B_FILES: &[RemoteFile] = &[
    RemoteFile {
        name: "chat_template.json",
        url: "chat_template.json?download=true",
    },
    RemoteFile {
        name: "config.json",
        url: "config.json?download=true",
    },
    RemoteFile {
        name: "generation_config.json",
        url: "generation_config.json?download=true",
    },
    RemoteFile {
        name: "merges.txt",
        url: "merges.txt?download=true",
    },
    RemoteFile {
        name: "model.safetensors",
        url: "model.safetensors?download=true",
    },
    RemoteFile {
        name: "preprocessor_config.json",
        url: "preprocessor_config.json?download=true",
    },
    RemoteFile {
        name: "tokenizer_config.json",
        url: "tokenizer_config.json?download=true",
    },
    RemoteFile {
        name: "vocab.json",
        url: "vocab.json?download=true",
    },
];

const QWEN_1_7B_FILES: &[RemoteFile] = &[
    RemoteFile {
        name: "chat_template.json",
        url: "chat_template.json?download=true",
    },
    RemoteFile {
        name: "config.json",
        url: "config.json?download=true",
    },
    RemoteFile {
        name: "generation_config.json",
        url: "generation_config.json?download=true",
    },
    RemoteFile {
        name: "merges.txt",
        url: "merges.txt?download=true",
    },
    RemoteFile {
        name: "model-00001-of-00002.safetensors",
        url: "model-00001-of-00002.safetensors?download=true",
    },
    RemoteFile {
        name: "model-00002-of-00002.safetensors",
        url: "model-00002-of-00002.safetensors?download=true",
    },
    RemoteFile {
        name: "model.safetensors.index.json",
        url: "model.safetensors.index.json?download=true",
    },
    RemoteFile {
        name: "preprocessor_config.json",
        url: "preprocessor_config.json?download=true",
    },
    RemoteFile {
        name: "tokenizer_config.json",
        url: "tokenizer_config.json?download=true",
    },
    RemoteFile {
        name: "vocab.json",
        url: "vocab.json?download=true",
    },
];

const CATALOG: &[CatalogEntry] = &[
    CatalogEntry {
        id: "small",
        repo: "Systran/faster-whisper-small",
        name: "Small",
        size: "~486 MB",
        description: "低配置优先，响应速度快",
        accuracy: "良好",
        speed: "很快",
        recommended: false,
        files: BASE_FILES,
    },
    CatalogEntry {
        id: "medium",
        repo: "Systran/faster-whisper-medium",
        name: "Medium",
        size: "~1.53 GB",
        description: "中英文平衡，首选模型",
        accuracy: "优秀",
        speed: "平衡",
        recommended: true,
        files: BASE_FILES,
    },
    CatalogEntry {
        id: "large-v3-turbo",
        repo: "h2oai/faster-whisper-large-v3-turbo",
        name: "Large V3 Turbo（旧版兼容）",
        size: "~1.62 GB",
        description: "兼容旧安装；新安装建议使用 2026 Qwen3-ASR",
        accuracy: "顶级",
        speed: "较快",
        recommended: false,
        files: TURBO_FILES,
    },
    CatalogEntry {
        id: "qwen3-asr-0.6b",
        repo: "Qwen/Qwen3-ASR-0.6B",
        name: "Qwen3-ASR 0.6B（2026）",
        size: "~1.88 GB",
        description: "2026 新模型，中文/方言覆盖广，内存占用较低",
        accuracy: "优秀",
        speed: "较快",
        recommended: true,
        files: QWEN_0_6B_FILES,
    },
    CatalogEntry {
        id: "qwen3-asr-1.7b",
        repo: "Qwen/Qwen3-ASR-1.7B",
        name: "Qwen3-ASR 1.7B（2026）",
        size: "~4.6 GB",
        description: "2026 高精度模型，适合显存/内存充足的设备",
        accuracy: "顶级",
        speed: "平衡",
        recommended: false,
        files: QWEN_1_7B_FILES,
    },
];

const MAX_DOWNLOAD_ATTEMPTS: usize = 6;
const DOWNLOAD_CONNECT_TIMEOUT_SECS: u64 = 15;
const DOWNLOAD_READ_TIMEOUT_SECS: u64 = 60;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelView {
    pub id: String,
    pub name: String,
    pub size: String,
    pub description: String,
    pub accuracy: String,
    pub speed: String,
    pub recommended: bool,
    pub status: String,
    pub progress: f32,
    pub installed_size: u64,
    pub sha256: Option<String>,
    pub active: bool,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadProgress {
    pub model_id: String,
    pub file: String,
    pub downloaded: u64,
    pub total: u64,
    pub progress: f32,
    pub status: String,
    pub message: Option<String>,
}

pub fn get_models(state: &AppState) -> impl std::future::Future<Output = Vec<ModelView>> + '_ {
    async move {
        let config = state.config.read().await.clone();
        let downloading = state
            .download_cancellations
            .lock()
            .map(|guard| guard.keys().cloned().collect::<Vec<_>>())
            .unwrap_or_default();
        CATALOG
            .iter()
            .map(|entry| {
                let installed = config.models.get(entry.id).filter(|model| model.installed);
                ModelView {
                    id: entry.id.into(),
                    name: entry.name.into(),
                    size: entry.size.into(),
                    description: entry.description.into(),
                    accuracy: entry.accuracy.into(),
                    speed: entry.speed.into(),
                    recommended: entry.recommended,
                    status: if downloading.iter().any(|id| id == entry.id) {
                        "downloading"
                    } else if installed.is_some() {
                        "installed"
                    } else {
                        "notInstalled"
                    }
                    .into(),
                    progress: if installed.is_some() { 1.0 } else { 0.0 },
                    installed_size: installed.map(|model| model.total_size).unwrap_or_default(),
                    sha256: installed.and_then(|model| model.sha256.clone()),
                    active: config.active_model == entry.id,
                    error: None,
                }
            })
            .collect()
    }
}

pub async fn download_model(
    app: AppHandle,
    state: AppState,
    model_id: String,
) -> Result<(), String> {
    let entry = CATALOG
        .iter()
        .find(|entry| entry.id == model_id)
        .ok_or_else(|| format!("未知模型：{model_id}"))?
        .clone();
    let cancellation = Arc::new(AtomicBool::new(false));
    {
        let mut downloads = state
            .download_cancellations
            .lock()
            .map_err(|_| "模型下载状态锁已损坏".to_string())?;
        if downloads.contains_key(&model_id) {
            return Err("该模型已经在下载中".into());
        }
        downloads.insert(model_id.clone(), cancellation.clone());
    }
    let result = download_model_inner(&app, &state, &entry, &model_id, cancellation).await;
    if let Err(error) = &result {
        // A command error alone leaves the frontend's optimistic "downloading"
        // state in place. Emit a terminal event so the retry button is restored.
        if !error.contains("已暂停") {
            emit_progress(
                &app,
                DownloadProgress {
                    model_id: model_id.clone(),
                    file: "下载失败".into(),
                    downloaded: 0,
                    total: 0,
                    progress: 0.0,
                    status: "error".into(),
                    message: Some(error.clone()),
                },
            );
        }
    }
    if let Ok(mut downloads) = state.download_cancellations.lock() {
        downloads.remove(&model_id);
    }
    result
}

async fn download_model_inner(
    app: &AppHandle,
    state: &AppState,
    entry: &CatalogEntry,
    model_id: &str,
    cancellation: Arc<AtomicBool>,
) -> Result<(), String> {
    let model_dir = state.model_dir(model_id);
    tokio::fs::create_dir_all(&model_dir)
        .await
        .map_err(|error| error.to_string())?;
    let client = reqwest::Client::builder()
        .user_agent("voice-caption-studio/0.1.15")
        .connect_timeout(Duration::from_secs(DOWNLOAD_CONNECT_TIMEOUT_SECS))
        .read_timeout(Duration::from_secs(DOWNLOAD_READ_TIMEOUT_SECS))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .map_err(|error| error.to_string())?;
    let endpoints = download_endpoints(entry.repo);
    let mut records = Vec::new();

    for remote in entry.files {
        let target = model_dir.join(remote.name);
        records.push(
            download_file(
                app,
                &client,
                &endpoints,
                model_id,
                entry.repo,
                remote,
                target,
                cancellation.clone(),
            )
            .await?,
        );
    }

    let total_size = records.iter().map(|record| record.size).sum();
    let aggregate = aggregate_hash(&records);
    let manifest = serde_json::json!({ "modelId": model_id, "version": "main", "files": records, "totalSize": total_size, "sha256": aggregate });
    tokio::fs::write(
        model_dir.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).map_err(|error| error.to_string())?,
    )
    .await
    .map_err(|error| error.to_string())?;
    {
        let mut config = state.config.write().await;
        config.models.insert(
            model_id.into(),
            InstalledModel {
                id: model_id.into(),
                version: Some("main".into()),
                installed: true,
                total_size,
                sha256: Some(aggregate),
                verified_at: Some(crate::state::now_millis().to_string()),
                files: records,
            },
        );
    }
    state.save_config().await?;
    emit_progress(
        app,
        DownloadProgress {
            model_id: model_id.into(),
            file: "全部文件".into(),
            downloaded: total_size,
            total: total_size,
            progress: 1.0,
            status: "complete".into(),
            message: None,
        },
    );
    Ok(())
}

async fn download_file(
    app: &AppHandle,
    client: &reqwest::Client,
    endpoints: &[DownloadSource],
    model_id: &str,
    repo: &str,
    remote: &RemoteFile,
    target: PathBuf,
    cancellation: Arc<AtomicBool>,
) -> Result<ModelFileRecord, String> {
    let mut errors = Vec::new();

    #[cfg(target_os = "windows")]
    {
        // Windows 10/11 ships curl.exe. It uses the Windows TLS and proxy
        // stack more consistently than an embedded Rust client on machines
        // with enterprise certificates or a system proxy configured. Keep the
        // Rust implementation below as a fallback for stripped-down systems.
        match download_file_with_curl(
            app,
            endpoints,
            model_id,
            repo,
            remote,
            &target,
            cancellation.clone(),
        )
        .await
        {
            Ok(record) => return Ok(record),
            Err(error) if error.contains("已暂停") => return Err(error),
            Err(error) if !error.contains("找不到 curl.exe") => {
                errors.push(format!("Windows 下载器：{error}"));
            }
            Err(_) => {}
        }
    }

    for attempt in 0..MAX_DOWNLOAD_ATTEMPTS {
        if cancellation.load(Ordering::Relaxed) {
            emit_paused(app, model_id, remote.name, 0, 0);
            return Err("模型下载已暂停，点击继续可从断点恢复".into());
        }
        if attempt > 0 {
            sleep(Duration::from_secs(2)).await;
        }

        let existing = tokio::fs::metadata(&target)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        let endpoint = &endpoints[attempt % endpoints.len()];
        let url = endpoint_url(endpoint, repo, remote);
        emit_progress(
            app,
            DownloadProgress {
                model_id: model_id.into(),
                file: remote.name.into(),
                downloaded: existing,
                total: 0,
                progress: 0.0,
                status: "downloading".into(),
                message: Some(format!("连接下载源 {}…", endpoint.label)),
            },
        );

        let mut request = client
            .get(&url)
            .header(ACCEPT, "application/octet-stream")
            .header("Connection", "keep-alive");
        if existing > 0 {
            request = request.header(RANGE, format!("bytes={existing}-"));
        }
        let response = match tokio::time::timeout(
            Duration::from_secs(DOWNLOAD_CONNECT_TIMEOUT_SECS + 15),
            request.send(),
        )
        .await
        {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                errors.push(format!("{}: {error}", endpoint.label));
                continue;
            }
            Err(_) => {
                errors.push(format!("{}: 连接超时", endpoint.label));
                continue;
            }
        };

        if response.status() == StatusCode::RANGE_NOT_SATISFIABLE && existing > 0 {
            // The server may have a stale/incompatible partial file. Restart
            // that one file instead of leaving the download permanently stuck.
            let _ = tokio::fs::remove_file(&target).await;
            errors.push(format!("{}: 断点范围无效，已重新开始", endpoint.label));
            continue;
        }
        if !response.status().is_success() {
            errors.push(format!("{}: HTTP {}", endpoint.label, response.status()));
            continue;
        }

        let append = existing > 0 && response.status() == StatusCode::PARTIAL_CONTENT;
        let starting = if append { existing } else { 0 };
        let total = response_total(&response, starting);
        emit_progress(
            app,
            DownloadProgress {
                model_id: model_id.into(),
                file: remote.name.into(),
                downloaded: starting,
                total,
                progress: ratio(starting, total),
                status: "downloading".into(),
                message: Some(format!("已连接 {}，开始传输…", endpoint.label)),
            },
        );
        let mut options = tokio::fs::OpenOptions::new();
        options.create(true).write(true);
        if append {
            options.append(true);
        } else {
            options.truncate(true);
        }
        let mut file = options
            .open(&target)
            .await
            .map_err(|error| format!("无法写入 {}：{error}", remote.name))?;
        let mut downloaded = starting;
        let mut stream = response.bytes_stream();
        let mut stream_error = None;
        loop {
            let next = tokio::time::timeout(
                Duration::from_secs(DOWNLOAD_READ_TIMEOUT_SECS),
                stream.next(),
            )
            .await;
            let chunk = match next {
                Ok(Some(chunk)) => chunk,
                Ok(None) => break,
                Err(_) => {
                    stream_error = Some(format!("读取 {} 超时", remote.name));
                    break;
                }
            };
            if cancellation.load(Ordering::Relaxed) {
                emit_paused(app, model_id, remote.name, downloaded, total);
                return Err("模型下载已暂停，点击继续可从断点恢复".into());
            }
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(error) => {
                    stream_error = Some(format!("读取 {} 失败：{error}", remote.name));
                    break;
                }
            };
            file.write_all(&bytes)
                .await
                .map_err(|error| format!("写入 {} 失败：{error}", remote.name))?;
            downloaded = downloaded.saturating_add(bytes.len() as u64);
            emit_progress(
                app,
                DownloadProgress {
                    model_id: model_id.into(),
                    file: remote.name.into(),
                    downloaded,
                    total,
                    progress: ratio(downloaded, total),
                    status: "downloading".into(),
                    message: None,
                },
            );
        }
        if let Some(error) = stream_error {
            let _ = file.flush().await;
            errors.push(format!("{}: {error}", endpoint.label));
            continue;
        }
        file.flush()
            .await
            .map_err(|error| format!("刷新 {} 失败：{error}", remote.name))?;

        if downloaded == 0 {
            errors.push(format!("{}: 下载源返回空文件", endpoint.label));
            continue;
        }
        if total > 0 && downloaded < total {
            errors.push(format!(
                "{}: 文件未传输完整（{downloaded}/{total} 字节）",
                endpoint.label
            ));
            continue;
        }

        let path = target.clone();
        let hash = tokio::task::spawn_blocking(move || sha256_file(&path))
            .await
            .map_err(|error| error.to_string())??;
        return Ok(ModelFileRecord {
            name: remote.name.into(),
            size: hash.0,
            sha256: hash.1,
        });
    }

    Err(format!(
        "下载 {} 失败：{}。可检查网络，或设置 HF_ENDPOINT/HUGGINGFACE_ENDPOINT 指向可访问的 Hugging Face 镜像",
        remote.name,
        errors.join("；")
    ))
}

#[derive(Debug, Clone)]
enum DownloadSourceKind {
    HuggingFace,
    ModelScope,
}

#[derive(Debug, Clone)]
struct DownloadSource {
    base_url: String,
    kind: DownloadSourceKind,
    label: String,
}

fn download_endpoints(repo: &str) -> Vec<DownloadSource> {
    let mut endpoints = Vec::new();
    for variable in ["HF_ENDPOINT", "HUGGINGFACE_ENDPOINT"] {
        if let Ok(endpoint) = std::env::var(variable) {
            push_endpoint(&mut endpoints, &endpoint, DownloadSourceKind::HuggingFace);
        }
    }
    if repo.starts_with("Qwen/") {
        // Qwen's official model card recommends ModelScope for mainland
        // networks. Hugging Face remains available as a fallback.
        let endpoint = std::env::var("MODELSCOPE_ENDPOINT")
            .unwrap_or_else(|_| "https://www.modelscope.cn".into());
        push_endpoint(&mut endpoints, &endpoint, DownloadSourceKind::ModelScope);
    }
    // The mirror is tried first for networks where huggingface.co is slow or
    // blocked; the official endpoint remains a fallback for other regions.
    for endpoint in ["https://hf-mirror.com", "https://huggingface.co"] {
        push_endpoint(&mut endpoints, endpoint, DownloadSourceKind::HuggingFace);
    }
    endpoints
}

fn push_endpoint(endpoints: &mut Vec<DownloadSource>, endpoint: &str, kind: DownloadSourceKind) {
    let normalized = endpoint.trim().trim_end_matches('/');
    if !normalized.is_empty()
        && !endpoints.iter().any(|existing| {
            existing.base_url.eq_ignore_ascii_case(normalized)
                && std::mem::discriminant(&existing.kind) == std::mem::discriminant(&kind)
        })
    {
        endpoints.push(DownloadSource {
            base_url: normalized.into(),
            label: normalized.into(),
            kind,
        });
    }
}

fn endpoint_url(endpoint: &DownloadSource, repo: &str, remote: &RemoteFile) -> String {
    match endpoint.kind {
        DownloadSourceKind::HuggingFace => {
            format!("{}/{}/resolve/main/{}", endpoint.base_url, repo, remote.url)
        }
        DownloadSourceKind::ModelScope => format!(
            "{}/models/{}/resolve/master/{}",
            endpoint.base_url, repo, remote.url
        ),
    }
}

#[cfg(target_os = "windows")]
async fn download_file_with_curl(
    app: &AppHandle,
    endpoints: &[DownloadSource],
    model_id: &str,
    repo: &str,
    remote: &RemoteFile,
    target: &PathBuf,
    cancellation: Arc<AtomicBool>,
) -> Result<ModelFileRecord, String> {
    let mut errors = Vec::new();
    for attempt in 0..MAX_DOWNLOAD_ATTEMPTS {
        if cancellation.load(Ordering::Relaxed) {
            let downloaded = tokio::fs::metadata(target)
                .await
                .map(|metadata| metadata.len())
                .unwrap_or_default();
            emit_paused(app, model_id, remote.name, downloaded, 0);
            return Err("模型下载已暂停，点击继续可从断点恢复".into());
        }
        if attempt > 0 {
            sleep(Duration::from_secs(2)).await;
        }

        let endpoint = &endpoints[attempt % endpoints.len()];
        let url = endpoint_url(endpoint, repo, remote);
        let existing = tokio::fs::metadata(target)
            .await
            .map(|metadata| metadata.len())
            .unwrap_or_default();
        emit_progress(
            app,
            DownloadProgress {
                model_id: model_id.into(),
                file: remote.name.into(),
                downloaded: existing,
                total: 0,
                progress: 0.0,
                status: "downloading".into(),
                message: Some(format!("Windows 下载器连接 {}…", endpoint.label)),
            },
        );

        let mut curl = Command::new("curl.exe");
        // Do not open a visible console window for the GUI application's
        // background downloader.
        curl.creation_flags(0x08000000);
        let mut child = match curl
            .args([
                "--location",
                "--fail",
                "--silent",
                "--show-error",
                "--retry",
                "3",
                "--retry-delay",
                "2",
                "--connect-timeout",
                "20",
                "--speed-time",
                "30",
                "--speed-limit",
                "1",
                "--continue-at",
                "-",
                "--output",
            ])
            .arg(target)
            .args(["--user-agent", "voice-caption-studio/0.1.15"])
            .arg(&url)
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
        {
            Ok(child) => child,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err("找不到 curl.exe".into());
            }
            Err(error) => {
                errors.push(format!("{}: 无法启动 curl.exe：{error}", endpoint.label));
                continue;
            }
        };

        let status = loop {
            if cancellation.load(Ordering::Relaxed) {
                let _ = child.kill().await;
                let _ = child.wait().await;
                let downloaded = tokio::fs::metadata(target)
                    .await
                    .map(|metadata| metadata.len())
                    .unwrap_or_default();
                emit_paused(app, model_id, remote.name, downloaded, 0);
                return Err("模型下载已暂停，点击继续可从断点恢复".into());
            }
            match child.try_wait() {
                Ok(Some(status)) => break status,
                Ok(None) => {
                    let downloaded = tokio::fs::metadata(target)
                        .await
                        .map(|metadata| metadata.len())
                        .unwrap_or_default();
                    emit_progress(
                        app,
                        DownloadProgress {
                            model_id: model_id.into(),
                            file: remote.name.into(),
                            downloaded,
                            total: 0,
                            progress: 0.0,
                            status: "downloading".into(),
                            message: Some("正在传输，已启用断点续传…".into()),
                        },
                    );
                    sleep(Duration::from_millis(350)).await;
                }
                Err(error) => {
                    return Err(format!("{}: 读取 curl 状态失败：{error}", endpoint.label));
                }
            }
        };
        let output = child
            .wait_with_output()
            .await
            .map_err(|error| format!("读取 curl 输出失败：{error}"))?;
        if !status.success() {
            let message = String::from_utf8_lossy(&output.stderr).trim().to_string();
            if existing > 0 && (message.contains("416") || message.to_lowercase().contains("range"))
            {
                let _ = tokio::fs::remove_file(target).await;
            }
            errors.push(format!(
                "{}: {}",
                endpoint.label,
                if message.is_empty() {
                    format!("curl 返回 {}", status)
                } else {
                    message
                }
            ));
            continue;
        }

        let path = target.clone();
        let hash = tokio::task::spawn_blocking(move || sha256_file(&path))
            .await
            .map_err(|error| error.to_string())??;
        if hash.0 == 0 {
            errors.push(format!("{}: 下载源返回空文件", endpoint.label));
            continue;
        }
        emit_progress(
            app,
            DownloadProgress {
                model_id: model_id.into(),
                file: remote.name.into(),
                downloaded: hash.0,
                total: hash.0,
                progress: 1.0,
                status: "downloading".into(),
                message: Some("文件传输完成，正在校验…".into()),
            },
        );
        return Ok(ModelFileRecord {
            name: remote.name.into(),
            size: hash.0,
            sha256: hash.1,
        });
    }
    Err(format!("curl 下载失败：{}", errors.join("；")))
}

fn response_total(response: &reqwest::Response, starting: u64) -> u64 {
    if let Some(total) = response
        .headers()
        .get(CONTENT_RANGE)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.rsplit('/').next())
        .and_then(|value| value.parse::<u64>().ok())
    {
        return total;
    }
    response
        .content_length()
        .unwrap_or_default()
        .saturating_add(starting)
}

fn emit_paused(app: &AppHandle, model_id: &str, file: &str, downloaded: u64, total: u64) {
    emit_progress(
        app,
        DownloadProgress {
            model_id: model_id.into(),
            file: file.into(),
            downloaded,
            total,
            progress: ratio(downloaded, total),
            status: "paused".into(),
            message: Some("已暂停，临时文件已保留".into()),
        },
    );
}

pub async fn pause_model_download(state: &AppState, model_id: String) -> Result<(), String> {
    let downloads = state
        .download_cancellations
        .lock()
        .map_err(|_| "模型下载状态锁已损坏".to_string())?;
    downloads
        .get(&model_id)
        .map(|flag| flag.store(true, Ordering::Relaxed))
        .ok_or_else(|| "该模型当前没有进行中的下载".into())
}

pub async fn verify_model(state: &AppState, model_id: String) -> Result<ModelView, String> {
    let entry = CATALOG
        .iter()
        .find(|entry| entry.id == model_id)
        .ok_or_else(|| format!("未知模型：{model_id}"))?;
    let model_dir = state.model_dir(&model_id);
    let mut records = Vec::new();
    for remote in entry.files {
        let path = model_dir.join(remote.name);
        if !path.exists() {
            return Err(format!("缺少模型文件：{}", remote.name));
        }
        let (size, sha256) = tokio::task::spawn_blocking(move || sha256_file(&path))
            .await
            .map_err(|error| error.to_string())??;
        records.push(ModelFileRecord {
            name: remote.name.into(),
            size,
            sha256,
        });
    }
    let total_size = records.iter().map(|record| record.size).sum();
    let aggregate = aggregate_hash(&records);
    tokio::fs::write(model_dir.join("manifest.json"), serde_json::to_vec_pretty(&serde_json::json!({ "modelId": model_id, "version": "main", "files": records, "totalSize": total_size, "sha256": aggregate })).map_err(|error| error.to_string())?).await.map_err(|error| error.to_string())?;
    {
        let mut config = state.config.write().await;
        config.models.insert(
            model_id.clone(),
            InstalledModel {
                id: model_id.clone(),
                version: Some("main".into()),
                installed: true,
                total_size,
                sha256: Some(aggregate),
                verified_at: Some(crate::state::now_millis().to_string()),
                files: records,
            },
        );
    }
    state.save_config().await?;
    get_models(state)
        .await
        .into_iter()
        .find(|model| model.id == model_id)
        .ok_or_else(|| "模型状态刷新失败".into())
}

pub async fn delete_model(state: &AppState, model_id: String) -> Result<(), String> {
    if CATALOG.iter().all(|entry| entry.id != model_id) {
        return Err(format!("未知模型：{model_id}"));
    }
    let model_dir = state.model_dir(&model_id);
    if model_dir.exists() {
        tokio::fs::remove_dir_all(&model_dir)
            .await
            .map_err(|error| format!("删除模型失败：{error}"))?;
    }
    {
        let mut config = state.config.write().await;
        config.models.remove(&model_id);
        if config.active_model == model_id {
            config.active_model = "medium".into();
        }
    }
    state.save_config().await
}

pub async fn select_model(
    state: &AppState,
    model_id: String,
) -> Result<crate::state::AppConfig, String> {
    let installed = state
        .config
        .read()
        .await
        .models
        .get(&model_id)
        .map(|model| model.installed)
        .unwrap_or(false);
    if !installed {
        return Err("请先下载并校验模型，再切换使用".into());
    }
    let mut config = state.config.write().await;
    config.active_model = model_id;
    let output = config.clone();
    drop(config);
    state.save_config().await?;
    Ok(output)
}

pub fn model_path(state: &AppState, model_id: &str) -> PathBuf {
    state.model_dir(model_id)
}

fn emit_progress(app: &AppHandle, progress: DownloadProgress) {
    let _ = app.emit("model-download", progress);
}

fn ratio(done: u64, total: u64) -> f32 {
    if total == 0 {
        0.0
    } else {
        (done as f32 / total as f32).clamp(0.0, 1.0)
    }
}

fn aggregate_hash(records: &[ModelFileRecord]) -> String {
    let mut text = String::new();
    for record in records {
        text.push_str(&record.name);
        text.push(':');
        text.push_str(&record.sha256);
        text.push('\n');
    }
    let mut hasher = sha2::Sha256::new();
    hasher.update(text.as_bytes());
    hex::encode(hasher.finalize())
}
