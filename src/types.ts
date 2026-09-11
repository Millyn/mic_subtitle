export type Page = "dashboard" | "models" | "style" | "glossary" | "settings";

export type RecognitionStatus = "idle" | "running" | "paused" | "error";

export interface AudioDevice {
  id: string;
  name: string;
  isDefault: boolean;
}

export interface TextStyle {
  fontFamily: string;
  fontSize: number;
  color: string;
  opacity: number;
}

export interface SubtitleStyle {
  chinese: TextStyle;
  english: TextStyle;
  layout: "stacked" | "sideBySide";
  alignment: "left" | "center" | "right";
  position: "top" | "center" | "bottom";
  canvasWidth: number;
  canvasHeight: number;
  subtitleX: number;
  subtitleY: number;
  maxWidth: number;
  lineSpacing: number;
  showTemporary: boolean;
  showFinal: boolean;
  hideAfterSeconds: number;
  backgroundColor: string;
  backgroundOpacity: number;
  outlineWidth: number;
  outlineColor: string;
  shadow: boolean;
}

export interface DeepSeekConfig {
  enabled: boolean;
  apiKey: string;
  baseUrl: string;
  model: string;
}

export interface GlossaryEntry {
  source: string;
  target: string;
}

export interface NoiseProfile {
  enabled: boolean;
  autoCalibrate: boolean;
  useVad: boolean;
  noiseFloor: number;
  speechThreshold: number;
  silenceThreshold: number;
  silenceMs: number;
}

export interface AppConfig {
  configVersion: number;
  selectedDevice: string | null;
  activeModel: string;
  deepseek: DeepSeekConfig;
  recognitionLanguage: "auto" | "Chinese" | "Chinese,English";
  glossary: GlossaryEntry[];
  noiseProfiles: Record<string, NoiseProfile>;
  style: SubtitleStyle;
  serverPort: number;
}

export interface ServerStatus {
  running: boolean;
  port: number;
  host: string;
  bindAddress: string;
  overlayUrl: string;
  editorUrl: string;
}

export interface HardwareInfo {
  cudaAvailable: boolean;
  gpuName: string | null;
  vramMb: number;
  memoryMb: number;
  recommendedModel: string;
  recommendationReason: string;
}

export interface SubtitleEvent {
  id: string;
  timestamp: number;
  kind: "partial" | "final";
  chinese: string;
  language?: string;
  english?: string;
  translationError?: string;
}

export interface TokenCounter {
  asrUtterances: number;
  asrTokens: number;
  asrEstimated: boolean;
  translationRequests: number;
  translationPromptTokens: number;
  translationCompletionTokens: number;
  translationTotalTokens: number;
  translationEstimated: boolean;
  totalTokens: number;
}

export interface TokenUsage {
  session: TokenCounter;
  allTime: TokenCounter;
}

export interface ModelCatalogEntry {
  id: string;
  name: string;
  size: string;
  description: string;
  accuracy: string;
  speed: string;
  recommended: boolean;
}

export type ModelStatus = "notInstalled" | "downloading" | "installed" | "verifying" | "error";

export interface ModelView extends ModelCatalogEntry {
  status: ModelStatus;
  progress: number;
  installedSize: number;
  sha256: string | null;
  active: boolean;
  error?: string;
}

export interface ModelDownloadProgress {
  modelId: string;
  file: string;
  downloaded: number;
  total: number;
  progress: number;
  status: "downloading" | "paused" | "complete" | "error";
  message?: string;
}

export interface LogEntry {
  id: string;
  time: string;
  tone: "neutral" | "success" | "warning" | "error";
  message: string;
}

export const defaultStyle: SubtitleStyle = {
  chinese: {
    fontFamily: "Microsoft YaHei, PingFang SC, sans-serif",
    fontSize: 34,
    color: "#ffffff",
    opacity: 1,
  },
  english: {
    fontFamily: "Segoe UI, Arial, sans-serif",
    fontSize: 23,
    color: "#a7f3d0",
    opacity: 0.94,
  },
  layout: "stacked",
  alignment: "center",
  position: "bottom",
  canvasWidth: 1920,
  canvasHeight: 1080,
  subtitleX: 960,
  subtitleY: 900,
  maxWidth: 1100,
  lineSpacing: 1.3,
  showTemporary: true,
  showFinal: true,
  hideAfterSeconds: 10,
  backgroundColor: "#07111f",
  backgroundOpacity: 0.68,
  outlineWidth: 2,
  outlineColor: "#020617",
  shadow: true,
};

export const emptyConfig: AppConfig = {
  configVersion: 1,
  selectedDevice: null,
  activeModel: "qwen3-asr-0.6b",
  deepseek: {
    enabled: true,
    apiKey: "",
    baseUrl: "https://api.deepseek.com",
    model: "deepseek-chat",
  },
  recognitionLanguage: "auto",
  glossary: [],
  noiseProfiles: {},
  style: defaultStyle,
  serverPort: 39071,
};

export const emptyTokenCounter: TokenCounter = {
  asrUtterances: 0,
  asrTokens: 0,
  asrEstimated: false,
  translationRequests: 0,
  translationPromptTokens: 0,
  translationCompletionTokens: 0,
  translationTotalTokens: 0,
  translationEstimated: false,
  totalTokens: 0,
};

export const emptyTokenUsage: TokenUsage = {
  session: emptyTokenCounter,
  allTime: emptyTokenCounter,
};
