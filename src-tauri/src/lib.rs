#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod hardware;
mod models;
mod recognition;
mod server;
mod state;
mod translation;

use models::ModelView;
use state::{normalize_server_port, AppConfig, AppState, SubtitleEvent, SubtitleStyle};
use tauri::{AppHandle, Manager, State};

#[tauri::command]
async fn get_config(state: State<'_, AppState>) -> Result<AppConfig, String> {
    Ok(state.config.read().await.clone())
}

#[tauri::command]
async fn save_config(state: State<'_, AppState>, config: AppConfig) -> Result<AppConfig, String> {
    let mut current = state.config.write().await;
    let next_server_port = normalize_server_port(config.server_port);
    let server_port_changed = current.server_port != next_server_port;
    current.selected_device = config.selected_device;
    current.active_model = config.active_model;
    current.deepseek = config.deepseek;
    current.style = config.style;
    current.server_port = next_server_port;
    if !config.models.is_empty() {
        current.models = config.models;
    }
    let output = current.clone();
    drop(current);
    state.save_config().await?;
    if server_port_changed {
        server::restart(&state).await;
    }
    Ok(output)
}

#[tauri::command]
fn list_audio_devices() -> Result<Vec<audio::AudioDevice>, String> {
    audio::list_audio_devices()
}

#[tauri::command]
fn start_audio_monitor(
    app: AppHandle,
    state: State<'_, AppState>,
    device_name: Option<String>,
) -> Result<(), String> {
    audio::start_monitor(app, &state, device_name)
}

#[tauri::command]
fn stop_audio_monitor(state: State<'_, AppState>) -> Result<(), String> {
    audio::stop_monitor(&state);
    Ok(())
}

#[tauri::command]
async fn start_recognition(
    app: AppHandle,
    state: State<'_, AppState>,
    device_name: Option<String>,
) -> Result<(), String> {
    recognition::start(app, &state, device_name).await
}

#[tauri::command]
fn pause_recognition(state: State<'_, AppState>) -> Result<(), String> {
    recognition::stop(&state);
    Ok(())
}

#[tauri::command]
fn stop_recognition(state: State<'_, AppState>) -> Result<(), String> {
    recognition::stop(&state);
    Ok(())
}

#[tauri::command]
fn get_server_status(state: State<'_, AppState>) -> ServerStatus {
    let port = state.server_port.load(std::sync::atomic::Ordering::Relaxed);
    let running = state
        .server_started
        .load(std::sync::atomic::Ordering::Relaxed);
    ServerStatus::new(port, running)
}

#[tauri::command]
fn get_hardware_info() -> hardware::HardwareInfo {
    hardware::inspect()
}

#[tauri::command]
async fn get_models(state: State<'_, AppState>) -> Result<Vec<ModelView>, String> {
    Ok(models::get_models(&state).await)
}

#[tauri::command]
async fn download_model(
    app: AppHandle,
    state: State<'_, AppState>,
    model_id: String,
) -> Result<(), String> {
    models::download_model(app, state.inner().clone(), model_id).await
}

#[tauri::command]
async fn pause_model_download(state: State<'_, AppState>, model_id: String) -> Result<(), String> {
    models::pause_model_download(&state, model_id).await
}

#[tauri::command]
async fn verify_model(state: State<'_, AppState>, model_id: String) -> Result<ModelView, String> {
    models::verify_model(&state, model_id).await
}

#[tauri::command]
async fn delete_model(state: State<'_, AppState>, model_id: String) -> Result<(), String> {
    models::delete_model(&state, model_id).await
}

#[tauri::command]
async fn select_model(state: State<'_, AppState>, model_id: String) -> Result<AppConfig, String> {
    models::select_model(&state, model_id).await
}

#[tauri::command]
async fn save_style(
    state: State<'_, AppState>,
    style: SubtitleStyle,
) -> Result<SubtitleStyle, String> {
    {
        let mut config = state.config.write().await;
        config.style = style.clone();
    }
    state.save_config().await?;
    state.publish_style(style.clone()).await;
    Ok(style)
}

#[tauri::command]
async fn reset_style(state: State<'_, AppState>) -> Result<SubtitleStyle, String> {
    let style = SubtitleStyle::default();
    {
        let mut config = state.config.write().await;
        config.style = style.clone();
    }
    state.save_config().await?;
    state.publish_style(style.clone()).await;
    Ok(style)
}

#[tauri::command]
async fn translate_text(state: State<'_, AppState>, text: String) -> Result<String, String> {
    let config = state.config.read().await.clone();
    translation::translate(&config, &text).await
}

#[tauri::command]
async fn publish_subtitle(state: State<'_, AppState>, event: SubtitleEvent) -> Result<(), String> {
    let _ = translation::publish_with_translation(&state, event).await;
    Ok(())
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerStatus {
    pub running: bool,
    pub port: u16,
    pub host: String,
    pub bind_address: String,
    pub overlay_url: String,
    pub editor_url: String,
}

impl ServerStatus {
    pub fn new(port: u16, running: bool) -> Self {
        let host = server::local_ipv4();
        Self {
            running,
            port,
            host: host.clone(),
            bind_address: "0.0.0.0".into(),
            overlay_url: if port == 0 {
                String::new()
            } else {
                format!("http://{host}:{port}/overlay")
            },
            editor_url: if port == 0 {
                String::new()
            } else {
                format!("http://{host}:{port}/editor")
            },
        }
    }
}

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let executable_dir = std::env::current_exe()
                .map_err(|error| format!("无法定位程序目录：{error}"))?
                .parent()
                .map(|path| path.to_path_buf())
                .ok_or_else(|| "无法定位程序目录".to_string())?;
            let models_dir = executable_dir.join("models");
            std::fs::create_dir_all(&models_dir).map_err(|error| {
                format!("程序目录不可写，请将软件解压到有写入权限的文件夹：{error}")
            })?;
            let state = AppState::new(executable_dir.join("config.json"), models_dir)?;
            app.manage(state.clone());
            server::spawn(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            list_audio_devices,
            start_audio_monitor,
            stop_audio_monitor,
            start_recognition,
            pause_recognition,
            stop_recognition,
            get_server_status,
            get_hardware_info,
            get_models,
            download_model,
            pause_model_download,
            verify_model,
            delete_model,
            select_model,
            save_style,
            reset_style,
            translate_text,
            publish_subtitle
        ])
        .run(tauri::generate_context!())
        .expect("error while running voice caption studio");
}
