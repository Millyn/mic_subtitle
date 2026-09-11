import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  AppConfig,
  AudioDevice,
  GlossaryEntry,
  ModelDownloadProgress,
  ModelView,
  HardwareInfo,
  ServerStatus,
  SubtitleEvent,
  SubtitleStyle,
  TokenUsage,
} from "./types";

const isTauri = () => typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);

export async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  if (!isTauri()) {
    throw new Error("当前正在浏览器预览模式，请使用 Tauri 桌面端运行此功能。");
  }
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    // Tauri commands that return Result<T, String> reject with the raw
    // string payload, not always an Error instance. Normalize it here so the
    // desktop UI never hides the real Rust/Python startup diagnosis.
    if (error instanceof Error) throw error;
    if (typeof error === "string") throw new Error(error);
    if (error && typeof error === "object" && "message" in error) {
      throw new Error(String((error as { message: unknown }).message));
    }
    throw new Error(String(error ?? "桌面命令执行失败"));
  }
}

export async function getConfig(): Promise<AppConfig> {
  return call<AppConfig>("get_config");
}

export async function saveConfig(config: AppConfig): Promise<AppConfig> {
  return call<AppConfig>("save_config", { config });
}

export async function exportGlossary(glossary: GlossaryEntry[]): Promise<string> {
  return call<string>("export_glossary", { glossary });
}

export async function getAudioDevices(): Promise<AudioDevice[]> {
  return call<AudioDevice[]>("list_audio_devices");
}

export async function startAudioMonitor(deviceName: string | null): Promise<void> {
  return call("start_audio_monitor", { deviceName });
}

export async function stopAudioMonitor(): Promise<void> {
  return call("stop_audio_monitor");
}

export async function startRecognition(deviceName: string | null): Promise<void> {
  return call("start_recognition", { deviceName });
}

export async function stopRecognition(): Promise<void> {
  return call("stop_recognition");
}

export async function pauseRecognition(): Promise<void> {
  return call("pause_recognition");
}

export async function getServerStatus(): Promise<ServerStatus> {
  return call<ServerStatus>("get_server_status");
}

export async function getTokenUsage(): Promise<TokenUsage> {
  return call<TokenUsage>("get_token_usage");
}

export async function getHardwareInfo(): Promise<HardwareInfo> {
  return call<HardwareInfo>("get_hardware_info");
}

export async function getModels(): Promise<ModelView[]> {
  return call<ModelView[]>("get_models");
}

export async function downloadModel(modelId: string): Promise<void> {
  return call("download_model", { modelId });
}

export async function pauseModelDownload(modelId: string): Promise<void> {
  return call("pause_model_download", { modelId });
}

export async function verifyModel(modelId: string): Promise<ModelView> {
  return call<ModelView>("verify_model", { modelId });
}

export async function deleteModel(modelId: string): Promise<void> {
  return call("delete_model", { modelId });
}

export async function selectModel(modelId: string): Promise<AppConfig> {
  return call<AppConfig>("select_model", { modelId });
}

export async function saveStyle(style: SubtitleStyle): Promise<SubtitleStyle> {
  return call<SubtitleStyle>("save_style", { style });
}

export async function resetStyle(): Promise<SubtitleStyle> {
  return call<SubtitleStyle>("reset_style");
}

export async function translateText(text: string): Promise<string> {
  return call<string>("translate_text", { text });
}

export async function publishSubtitle(event: SubtitleEvent): Promise<void> {
  return call("publish_subtitle", { event });
}

export function onAudioLevel(handler: (level: number) => void): Promise<UnlistenFn> {
  return listen<number>("audio-level", (event) => handler(event.payload));
}

export function onSubtitle(handler: (event: SubtitleEvent) => void): Promise<UnlistenFn> {
  return listen<SubtitleEvent>("subtitle", (event) => handler(event.payload));
}

export function onRecognitionError(handler: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>("recognition-error", (event) => handler(event.payload));
}

export function onRecognitionStatus(handler: (message: string) => void): Promise<UnlistenFn> {
  return listen<string>("recognition-status", (event) => handler(event.payload));
}

export function onModelProgress(handler: (event: ModelDownloadProgress) => void): Promise<UnlistenFn> {
  return listen<ModelDownloadProgress>("model-download", (event) => handler(event.payload));
}

export function onTokenUsage(handler: (usage: TokenUsage) => void): Promise<UnlistenFn> {
  return listen<TokenUsage>("token-usage", (event) => handler(event.payload));
}
