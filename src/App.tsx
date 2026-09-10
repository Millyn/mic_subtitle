import { useEffect, useMemo, useState, type ReactNode } from "react";
import {
  deleteModel,
  downloadModel,
  getAudioDevices,
  getConfig,
  getHardwareInfo,
  getModels,
  getServerStatus,
  getTokenUsage,
  onAudioLevel,
  onRecognitionError,
  onRecognitionStatus,
  onModelProgress,
  onSubtitle,
  onTokenUsage,
  pauseModelDownload,
  pauseRecognition,
  resetStyle,
  saveConfig,
  saveStyle,
  selectModel,
  startAudioMonitor,
  startRecognition,
  stopAudioMonitor,
  stopRecognition,
  translateText,
  verifyModel,
} from "./api";
import {
  defaultStyle,
  emptyConfig,
  type AppConfig,
  type AudioDevice,
  type GlossaryEntry,
  type LogEntry,
  type HardwareInfo,
  type ModelDownloadProgress,
  type ModelView,
  type Page,
  type ServerStatus,
  type SubtitleEvent,
  type SubtitleStyle,
  emptyTokenUsage,
  type TokenUsage,
} from "./types";

const desktopRuntime = () => typeof window !== "undefined" && Boolean(window.__TAURI_INTERNALS__);

const cloneStyle = (style: SubtitleStyle): SubtitleStyle => ({
  ...style,
  chinese: { ...style.chinese },
  english: { ...style.english },
});

const cloneConfig = (config: AppConfig): AppConfig => ({
  ...config,
  deepseek: { ...config.deepseek },
  recognitionLanguage: config.recognitionLanguage || "Chinese",
  glossary: (config.glossary || []).map((entry) => ({ ...entry })),
  style: cloneStyle(config.style),
});

const nowLabel = () =>
  new Date().toLocaleTimeString("zh-CN", { hour: "2-digit", minute: "2-digit", second: "2-digit" });

const errorMessage = (error: unknown, fallback: string) => {
  if (error instanceof Error) return error.message;
  if (typeof error === "string" && error.trim()) return error;
  if (error && typeof error === "object") {
    try {
      return JSON.stringify(error);
    } catch {
      return fallback;
    }
  }
  return fallback;
};

const formatBytes = (bytes: number) => {
  if (!bytes) return "—";
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / 1024 / 1024).toFixed(1)} GB`;
  return `${(bytes / 1024 / 1024 / 1024).toFixed(1)} GB`;
};

const formatCount = (value: number) => new Intl.NumberFormat("zh-CN").format(value || 0);

const glossaryToText = (entries: GlossaryEntry[] | undefined) =>
  (entries || []).map((entry) => `${entry.source}=${entry.target}`).join("\n");

const parseGlossary = (value: string): GlossaryEntry[] =>
  value
    .split(/\r?\n/)
    .map((line) => line.trim())
    .map((line) => {
      if (!line) return null;
      const separator = line.indexOf("=");
      if (separator < 0) return { source: line, target: "" };
      return {
        source: line.slice(0, separator).trim(),
        target: line.slice(separator + 1).trim(),
      };
    })
    .filter((entry): entry is GlossaryEntry => Boolean(entry?.source))
    .slice(0, 200);

function Icon({ name, size = 18 }: { name: string; size?: number }) {
  const common = { width: size, height: size, viewBox: "0 0 24 24", fill: "none", stroke: "currentColor", strokeWidth: 1.8, strokeLinecap: "round" as const, strokeLinejoin: "round" as const };
  const paths: Record<string, ReactNode> = {
    grid: <><rect x="3" y="3" width="7" height="7" rx="1" /><rect x="14" y="3" width="7" height="7" rx="1" /><rect x="3" y="14" width="7" height="7" rx="1" /><rect x="14" y="14" width="7" height="7" rx="1" /></>,
    mic: <><path d="M12 2a3 3 0 0 0-3 3v7a3 3 0 0 0 6 0V5a3 3 0 0 0-3-3Z" /><path d="M19 10v2a7 7 0 0 1-14 0v-2M12 19v3M8 22h8" /></>,
    sliders: <><path d="M4 21v-7M4 10V3M12 21v-9M12 8V3M20 21v-5M20 12V3" /><path d="M1 14h6M9 8h6M17 16h6" /></>,
    palette: <><path d="M12 3a9 9 0 0 0 0 18h1.2a1.8 1.8 0 0 0 1.2-3.1 1.8 1.8 0 0 1 1.2-3.1h1.9A4.5 4.5 0 0 0 22 10.3 8.9 8.9 0 0 0 12 3Z" /><circle cx="7.5" cy="10" r=".8" fill="currentColor" /><circle cx="9" cy="6.5" r=".8" fill="currentColor" /><circle cx="14" cy="6" r=".8" fill="currentColor" /><circle cx="17" cy="9" r=".8" fill="currentColor" /></>,
    settings: <><path d="M12 15.5a3.5 3.5 0 1 0 0-7 3.5 3.5 0 0 0 0 7Z" /><path d="m19.4 15 .1.1a2 2 0 1 1-2.8 2.8l-.1-.1a1.7 1.7 0 0 0-2.9 1.2v.3a2 2 0 1 1-4 0V19a1.7 1.7 0 0 0-2.9-1.2l-.1.1a2 2 0 1 1-2.8-2.8l.1-.1A1.7 1.7 0 0 0 4.8 12H4.5a2 2 0 1 1 0-4h.3a1.7 1.7 0 0 0 1.2-2.9l-.1-.1a2 2 0 1 1 2.8-2.8l.1.1A1.7 1.7 0 0 0 11.7 1h.3a2 2 0 1 1 4 0v.3a1.7 1.7 0 0 0 2.9 1.2l.1-.1a2 2 0 1 1 2.8 2.8l-.1.1A1.7 1.7 0 0 0 22.9 8h.3a2 2 0 1 1 0 4h-.3a1.7 1.7 0 0 0-1.2 3Z" /></>,
    download: <><path d="M12 3v12M7 10l5 5 5-5" /><path d="M5 21h14" /></>,
    copy: <><rect x="9" y="9" width="11" height="11" rx="2" /><path d="M15 9V6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v7a2 2 0 0 0 2 2h3" /></>,
    play: <path d="m8 5 11 7-11 7V5Z" fill="currentColor" stroke="none" />,
    pause: <><rect x="7" y="5" width="3" height="14" rx="1" fill="currentColor" stroke="none" /><rect x="14" y="5" width="3" height="14" rx="1" fill="currentColor" stroke="none" /></>,
    stop: <rect x="6" y="6" width="12" height="12" rx="2" fill="currentColor" stroke="none" />,
    check: <path d="m5 12 4 4L19 6" />,
    external: <><path d="M14 4h6v6M20 4l-9 9" /><path d="M18 13v5a2 2 0 0 1-2 2H6a2 2 0 0 1-2-2V8a2 2 0 0 1 2-2h5" /></>,
    refresh: <><path d="M20 11a8.1 8.1 0 0 0-14.7-3L3 11" /><path d="M3 5v6h6M4 13a8.1 8.1 0 0 0 14.7 3L21 13" /><path d="M21 19v-6h-6" /></>,
    trash: <><path d="M4 7h16M10 11v6M14 11v6M6 7l1 13h10l1-13M9 7V4h6v3" /></>,
    link: <><path d="M10 13a5 5 0 0 0 7.1.1l2-2a5 5 0 0 0-7.1-7.1l-1.1 1.1" /><path d="M14 11a5 5 0 0 0-7.1-.1l-2 2a5 5 0 0 0 7.1 7.1l1.1-1.1" /></>,
    shield: <path d="M12 3 20 6v5c0 5-3.4 8.5-8 10-4.6-1.5-8-5-8-10V6l8-3Z" />,
    chevron: <path d="m9 18 6-6-6-6" />,
  };
  return <svg {...common}>{paths[name] ?? paths.grid}</svg>;
}

const mockModels: ModelView[] = [
  { id: "small", name: "Small", size: "~486 MB", description: "低配置优先，响应速度快", accuracy: "良好", speed: "很快", recommended: false, status: "notInstalled", progress: 0, installedSize: 0, sha256: null, active: false },
  { id: "medium", name: "Medium", size: "~1.53 GB", description: "中英文平衡，兼容旧配置", accuracy: "优秀", speed: "平衡", recommended: false, status: "notInstalled", progress: 0, installedSize: 0, sha256: null, active: false },
  { id: "large-v3-turbo", name: "Large V3 Turbo（旧版兼容）", size: "~1.62 GB", description: "旧版兼容模型，新安装建议使用 2026 Qwen3-ASR", accuracy: "顶级", speed: "较快", recommended: false, status: "notInstalled", progress: 0, installedSize: 0, sha256: null, active: false },
  { id: "qwen3-asr-0.6b", name: "Qwen3-ASR 0.6B（2026）", size: "~1.88 GB", description: "2026 新模型，中文/方言覆盖广，内存占用较低", accuracy: "优秀", speed: "较快", recommended: true, status: "notInstalled", progress: 0, installedSize: 0, sha256: null, active: true },
  { id: "qwen3-asr-1.7b", name: "Qwen3-ASR 1.7B（2026）", size: "~4.6 GB", description: "2026 高精度模型，适合显存/内存充足的设备", accuracy: "顶级", speed: "平衡", recommended: false, status: "notInstalled", progress: 0, installedSize: 0, sha256: null, active: false },
];

function App() {
  const [page, setPage] = useState<Page>("dashboard");
  const [config, setConfig] = useState<AppConfig>(() => cloneConfig(emptyConfig));
  const [devices, setDevices] = useState<AudioDevice[]>([]);
  const [server, setServer] = useState<ServerStatus>({ running: false, port: 0, host: "127.0.0.1", bindAddress: "0.0.0.0", overlayUrl: "", editorUrl: "" });
  const [models, setModels] = useState<ModelView[]>(mockModels);
  const [hardware, setHardware] = useState<HardwareInfo | null>(null);
  const [audioLevel, setAudioLevel] = useState(0.08);
  const [recognitionDiagnostic, setRecognitionDiagnostic] = useState("尚未启动识别");
  const [recognitionStatus, setRecognitionStatus] = useState<"idle" | "running" | "paused" | "error">("idle");
  const [currentSubtitle, setCurrentSubtitle] = useState<SubtitleEvent | null>(null);
  const [logs, setLogs] = useState<LogEntry[]>([]);
  const [toast, setToast] = useState<string | null>(null);
  const [modelProgress, setModelProgress] = useState<Record<string, ModelDownloadProgress>>({});
  const [isSaving, setIsSaving] = useState(false);
  const [tokenUsage, setTokenUsage] = useState<TokenUsage>(() => ({
    session: { ...emptyTokenUsage.session },
    allTime: { ...emptyTokenUsage.allTime },
  }));

  const addLog = (message: string, tone: LogEntry["tone"] = "neutral") => {
    setLogs((previous) => [{ id: `${Date.now()}-${Math.random()}`, time: nowLabel(), tone, message }, ...previous].slice(0, 8));
  };

  useEffect(() => {
    let cancelled = false;
    const bootstrap = async () => {
      if (!desktopRuntime()) {
        setDevices([{ id: "preview", name: "浏览器预览麦克风", isDefault: true }]);
        setServer({ running: true, port: 39071, host: "127.0.0.1", bindAddress: "0.0.0.0", overlayUrl: "http://127.0.0.1:39071/overlay", editorUrl: "http://127.0.0.1:39071/editor" });
        setHardware({ cudaAvailable: false, gpuName: null, vramMb: 0, memoryMb: 0, recommendedModel: "medium", recommendationReason: "浏览器预览模式不会读取本机硬件" });
        addLog("浏览器预览模式已启动", "warning");
        return;
      }
      try {
        const [nextConfig, nextDevices, nextServer, nextModels, nextHardware, nextTokenUsage] = await Promise.all([getConfig(), getAudioDevices(), getServerStatus(), getModels(), getHardwareInfo(), getTokenUsage()]);
        if (cancelled) return;
        setConfig(cloneConfig(nextConfig));
        setDevices(nextDevices);
        setServer(nextServer);
        setModels(nextModels);
        setHardware(nextHardware);
        setTokenUsage(nextTokenUsage);
        addLog("本机字幕服务已连接", "success");
        addLog(`${nextDevices.length} 个麦克风设备可用`, "neutral");
        addLog(`硬件建议使用 ${nextHardware.recommendedModel} 模型`, "neutral");
        try {
          await startAudioMonitor(nextConfig.selectedDevice);
          setRecognitionDiagnostic("电平监视已启动，等待麦克风输入…");
        } catch (error) {
          const message = error instanceof Error ? error.message : "电平监视启动失败";
          setRecognitionDiagnostic(message);
          addLog(`电平监视：${message}`, "warning");
        }
      } catch (error) {
        if (!cancelled) addLog(error instanceof Error ? error.message : "初始化失败", "error");
      }
    };
    void bootstrap();
    return () => {
      cancelled = true;
    };
  }, []);

  useEffect(() => {
    if (!desktopRuntime()) return;
    let disposed = false;
    const poll = async () => {
      try {
        const next = await getServerStatus();
        if (!disposed) setServer(next);
      } catch {
        // The service can still be binding while the desktop window is ready.
      }
    };
    void poll();
    const timer = window.setInterval(() => void poll(), 1500);
    return () => {
      disposed = true;
      window.clearInterval(timer);
    };
  }, []);

  useEffect(() => {
    if (!desktopRuntime()) {
      if (recognitionStatus === "running") {
        const timer = window.setInterval(() => setAudioLevel(0.22 + Math.random() * 0.5), 100);
        return () => window.clearInterval(timer);
      }
      setAudioLevel(0.06);
      return;
    }
    let disposed = false;
    let cleanups: UnlistenLike[] = [];
    const register = async () => {
      const registered: UnlistenLike[] = [];
      const subscriptions: Array<[string, () => Promise<UnlistenLike>]> = [
        ["audio-level", () => onAudioLevel(setAudioLevel)],
        ["subtitle", () => onSubtitle((event) => {
          setCurrentSubtitle(event);
          if (event.kind === "final") addLog(event.english ? "中文句子已翻译并同步" : "已确认一条中文句子", event.english ? "success" : "neutral");
        })],
        ["recognition-error", () => onRecognitionError((message) => {
          setRecognitionDiagnostic(message);
          setRecognitionStatus("error");
          addLog(message, "error");
          setToast(message);
        })],
        ["recognition-status", () => onRecognitionStatus((message) => {
          setRecognitionDiagnostic(message);
          addLog(message, "neutral");
        })],
        ["token-usage", () => onTokenUsage(setTokenUsage)],
      ];
      for (const [name, create] of subscriptions) {
        if (disposed) break;
        try {
          registered.push(await create());
        } catch (error) {
          const message = errorMessage(error, "未知错误");
          setRecognitionDiagnostic(`事件监听 ${name} 失败：${message}`);
          addLog(`事件监听 ${name} 失败：${message}`, "error");
        }
      }
      if (disposed) {
        registered.forEach((cleanup) => cleanup());
      } else {
        cleanups = registered;
      }
    };
    void register();
    return () => {
      disposed = true;
      cleanups.forEach((cleanup) => cleanup());
      cleanups = [];
    };
  }, []);

  useEffect(() => {
    if (!desktopRuntime()) return;
    let cleanup: UnlistenLike | undefined;
    void onModelProgress((event) => {
      setModelProgress((previous) => ({ ...previous, [event.modelId]: event }));
      if (event.status === "complete") {
        addLog(`${event.modelId} 模型下载完成`, "success");
        void refreshModels();
      } else if (event.status === "error") {
        addLog(event.message || `${event.modelId} 模型下载失败`, "error");
        setToast(event.message || "模型下载失败，可重试");
      } else if (event.status === "paused") {
        addLog(`${event.modelId} 模型下载已暂停，可继续下载`, "warning");
      }
    }).then((unlisten) => { cleanup = unlisten; });
    return () => cleanup?.();
  }, []);

  const refreshModels = async () => {
    if (!desktopRuntime()) return;
    try {
      setModels(await getModels());
    } catch (error) {
      addLog(error instanceof Error ? error.message : "模型状态读取失败", "error");
    }
  };

  const updateConfig = (patch: Partial<AppConfig>) => setConfig((previous) => ({ ...previous, ...patch }));
  const updateStyle = (patch: Partial<SubtitleStyle>) => setConfig((previous) => {
    const nextStyle = { ...previous.style, ...patch };
    if (patch.position) {
      nextStyle.subtitleX = Math.round(nextStyle.canvasWidth * 0.5);
      nextStyle.subtitleY = Math.round(nextStyle.canvasHeight * (patch.position === "top" ? 0.16 : patch.position === "center" ? 0.5 : 0.83));
    }
    return { ...previous, style: nextStyle };
  });

  const handleStart = async () => {
    if (recognitionStatus === "running") return;
    try {
      if (desktopRuntime()) {
        await startAudioMonitor(config.selectedDevice).catch((error) => {
          const message = error instanceof Error ? error.message : "电平监视启动失败";
          setRecognitionDiagnostic(message);
          addLog(`电平监视：${message}`, "warning");
        });
        await startRecognition(config.selectedDevice);
      }
      setRecognitionStatus("running");
      addLog("实时识别已启动", "success");
      setToast("识别已开始");
    } catch (error) {
      if (desktopRuntime()) {
        await stopAudioMonitor().catch(() => undefined);
      }
      setRecognitionStatus("error");
      const message = error instanceof Error ? error.message : "识别启动失败，请检查模型和麦克风";
      setRecognitionDiagnostic(message);
      addLog(message, "error");
      setToast(message);
    }
  };

  const handlePause = async () => {
    try {
      if (desktopRuntime()) await pauseRecognition();
      setRecognitionDiagnostic("识别已暂停，电平监视仍在运行");
      setRecognitionStatus("paused");
      addLog("识别已暂停", "warning");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "暂停失败", "error");
    }
  };

  const handleStop = async () => {
    try {
      if (desktopRuntime()) {
        await stopRecognition();
        await stopAudioMonitor();
      }
      setRecognitionStatus("idle");
      setRecognitionDiagnostic("识别已停止");
      setCurrentSubtitle(null);
      setAudioLevel(0.05);
      addLog("识别已停止", "neutral");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "停止失败", "error");
    }
  };

  const handleDeviceChange = async (deviceName: string) => {
    const next = { ...config, selectedDevice: deviceName || null };
    setConfig(next);
    if (desktopRuntime() && recognitionStatus === "running") {
      try {
        await stopAudioMonitor();
        await stopRecognition();
        await startAudioMonitor(next.selectedDevice);
        await startRecognition(next.selectedDevice);
        addLog(`已切换到 ${deviceName}`, "success");
      } catch (error) {
        addLog(error instanceof Error ? error.message : "设备切换失败", "error");
      }
    } else if (desktopRuntime()) {
      try {
        await startAudioMonitor(next.selectedDevice);
        setRecognitionDiagnostic("已切换输入设备，等待麦克风输入…");
      } catch (error) {
        const message = error instanceof Error ? error.message : "电平监视启动失败";
        setRecognitionDiagnostic(message);
        addLog(`电平监视：${message}`, "warning");
      }
    }
  };

  const copyOverlayUrl = async () => {
    if (!server.overlayUrl) return;
    await navigator.clipboard?.writeText(server.overlayUrl);
    setToast("OBS 地址已复制");
  };

  const saveSettings = async () => {
    setIsSaving(true);
    try {
      if (desktopRuntime()) await saveConfig(config);
      addLog("配置已保存到本机", "success");
      setToast("配置已保存");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "保存失败", "error");
    } finally {
      setIsSaving(false);
    }
  };

  const handleDownload = async (model: ModelView) => {
    if (!desktopRuntime()) {
      setToast("浏览器预览模式不执行模型下载");
      return;
    }
    try {
      setModelProgress((previous) => ({ ...previous, [model.id]: { modelId: model.id, file: "准备中", downloaded: 0, total: 0, progress: 0, status: "downloading" } }));
      addLog(`开始下载 ${model.name}`, "neutral");
      await downloadModel(model.id);
      await refreshModels();
    } catch (error) {
      const message = error instanceof Error ? error.message : "模型下载失败";
      setModelProgress((previous) => {
        const current = previous[model.id];
        return {
          ...previous,
          [model.id]: {
            modelId: model.id,
            file: "下载失败",
            downloaded: current?.downloaded ?? 0,
            total: current?.total ?? 0,
            progress: current?.progress ?? 0,
            status: "error",
            message,
          },
        };
      });
      addLog(message, "error");
      setToast(message);
    }
  };

  const handlePauseDownload = async (model: ModelView) => {
    try {
      if (desktopRuntime()) await pauseModelDownload(model.id);
      addLog(`${model.name} 下载已暂停，可继续下载`, "warning");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "暂停下载失败", "error");
    }
  };

  const handleVerify = async (model: ModelView) => {
    try {
      if (desktopRuntime()) await verifyModel(model.id);
      await refreshModels();
      addLog(`${model.name} 校验完成`, "success");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "模型校验失败", "error");
    }
  };

  const handleDelete = async (model: ModelView) => {
    if (!window.confirm(`确定删除 ${model.name} 模型吗？`)) return;
    try {
      if (desktopRuntime()) await deleteModel(model.id);
      await refreshModels();
      addLog(`${model.name} 已删除`, "neutral");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "删除模型失败", "error");
    }
  };

  const handleSelectModel = async (model: ModelView) => {
    try {
      if (desktopRuntime()) {
        const nextConfig = await selectModel(model.id);
        setConfig(cloneConfig(nextConfig));
      } else {
        setConfig((previous) => ({ ...previous, activeModel: model.id }));
      }
      setModels((previous) => previous.map((item) => ({ ...item, active: item.id === model.id })));
      addLog(`已切换到 ${model.name}`, "success");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "模型切换失败", "error");
    }
  };

  const handleSaveStyle = async () => {
    try {
      if (desktopRuntime()) await saveStyle(config.style);
      addLog("字幕样式已保存并同步到 OBS", "success");
      setToast("样式已保存");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "样式保存失败", "error");
    }
  };

  const handleResetStyle = async () => {
    try {
      const next = desktopRuntime() ? await resetStyle() : cloneStyle(defaultStyle);
      updateStyle(next);
      addLog("样式已恢复默认值", "neutral");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "恢复默认样式失败", "error");
    }
  };

  const handleTestTranslation = async () => {
    try {
      const result = desktopRuntime() ? await translateText("这是一个实时字幕翻译测试。") : "This is a live subtitle translation test.";
      setCurrentSubtitle({ id: "test", timestamp: Date.now(), kind: "final", chinese: "这是一个实时字幕翻译测试。", english: result });
      addLog("翻译测试成功", "success");
    } catch (error) {
      addLog(error instanceof Error ? error.message : "翻译测试失败", "error");
    }
  };

  const currentModel = useMemo(() => models.find((model) => model.id === config.activeModel), [models, config.activeModel]);

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="brand">
          <div className="brand-mark"><span>声</span><i /></div>
          <div><div className="brand-name">声译</div><div className="brand-subtitle">LIVE CAPTION STUDIO</div></div>
        </div>

        <div className="nav-group">
          <div className="nav-label">工作台</div>
          <NavItem icon="grid" label="实时控制台" active={page === "dashboard"} onClick={() => setPage("dashboard")} />
          <NavItem icon="download" label="语音模型" active={page === "models"} onClick={() => setPage("models")} badge={models.filter((model) => model.status === "installed").length || undefined} />
          <NavItem icon="palette" label="字幕样式" active={page === "style"} onClick={() => setPage("style")} />
        </div>
        <div className="nav-group settings-nav">
          <div className="nav-label">系统</div>
          <NavItem icon="settings" label="连接与设置" active={page === "settings"} onClick={() => setPage("settings")} />
        </div>

        <div className="sidebar-bottom">
          <div className="local-card"><div className="local-icon"><Icon name="shield" size={16} /></div><div><strong>本地优先</strong><span>识别在此设备运行</span></div><span className="green-dot" /></div>
          <div className="version">声译 v0.1.17 <span>·</span> Windows 版</div>
        </div>
      </aside>

      <main className="main-area">
        <header className="topbar">
          <div><div className="eyebrow">{pageLabel(page)}</div><h1>{pageTitle(page)}</h1></div>
          <div className="topbar-actions">
            <div className={`service-status ${server.running ? "online" : "offline"}`}><span className="status-dot" />{server.running ? "字幕服务运行中" : "字幕服务连接中"}</div>
            <button className="icon-button" title="刷新模型状态" onClick={() => void refreshModels()}><Icon name="refresh" size={17} /></button>
            <div className="avatar">Y</div>
          </div>
        </header>

        <div className="content-scroll">
          {page === "dashboard" && <DashboardPage {...{ config, devices, server, audioLevel, recognitionDiagnostic, recognitionStatus, currentSubtitle, logs, tokenUsage, currentModel, onStart: handleStart, onPause: handlePause, onStop: handleStop, onDeviceChange: handleDeviceChange, onCopyUrl: copyOverlayUrl, onOpenPage: setPage, onTestTranslation: handleTestTranslation }} />}
          {page === "models" && <><HardwareHint hardware={hardware} /><ModelsPage models={models} progress={modelProgress} onDownload={handleDownload} onPause={handlePauseDownload} onVerify={handleVerify} onDelete={handleDelete} onSelect={handleSelectModel} /></>}
          {page === "style" && <StylePage style={config.style} currentSubtitle={currentSubtitle} onChange={updateStyle} onSave={handleSaveStyle} onReset={handleResetStyle} />}
          {page === "settings" && <SettingsPageV2 config={config} server={server} isSaving={isSaving} onChange={updateConfig} onSave={saveSettings} onTest={handleTestTranslation} />}
        </div>
      </main>
      {toast && <Toast message={toast} onClose={() => setToast(null)} />}
    </div>
  );
}

type UnlistenLike = () => void;

function pageLabel(page: Page) {
  return { dashboard: "LIVE WORKSPACE", models: "LOCAL ENGINE", style: "VISUAL SYSTEM", settings: "PREFERENCES" }[page];
}

function pageTitle(page: Page) {
  return { dashboard: "实时控制台", models: "语音模型", style: "字幕样式", settings: "连接与设置" }[page];
}

function NavItem({ icon, label, active, onClick, badge }: { icon: string; label: string; active: boolean; onClick: () => void; badge?: number }) {
  return <button className={`nav-item ${active ? "active" : ""}`} onClick={onClick}><Icon name={icon} size={18} /><span>{label}</span>{badge ? <em>{badge}</em> : null}{active && <Icon name="chevron" size={15} />}</button>;
}

function DashboardPage(props: {
  config: AppConfig;
  devices: AudioDevice[];
  server: ServerStatus;
  audioLevel: number;
  recognitionDiagnostic: string;
  recognitionStatus: "idle" | "running" | "paused" | "error";
  currentSubtitle: SubtitleEvent | null;
  logs: LogEntry[];
  tokenUsage: TokenUsage;
  currentModel?: ModelView;
  onStart: () => void;
  onPause: () => void;
  onStop: () => void;
  onDeviceChange: (value: string) => void;
  onCopyUrl: () => void;
  onOpenPage: (page: Page) => void;
  onTestTranslation: () => void;
}) {
  const { config, devices, server, audioLevel, recognitionDiagnostic, recognitionStatus, currentSubtitle, logs, tokenUsage, currentModel } = props;
  const running = recognitionStatus === "running";
  const hasEnglish = Boolean(currentSubtitle?.english);
  const translationTokenLabel = tokenUsage.session.translationEstimated ? "翻译输入 / 输出（估算）" : "翻译输入 / 输出";
  return <div className="dashboard-page">
    <section className="hero-row">
      <div className="hero-copy"><div className="live-kicker"><span className="pulse-ring" />实时字幕工作区</div><h2>把每一句话，<span>清晰地传达。</span></h2><p>本地语音识别 · 可选云端翻译 · 为 OBS 而生</p></div>
      <div className="hero-metrics"><Metric value={running ? "LIVE" : "READY"} label="识别状态" accent={running} /><Metric value={currentModel?.name || "Medium"} label="当前模型" /><Metric value={server.port ? `:${server.port}` : "—"} label="本机服务" /></div>
    </section>

    <div className="dashboard-grid">
      <section className="panel control-panel">
        <PanelHeading eyebrow="CAPTURE" title="输入控制" action={<span className={`mini-state ${running ? "green" : ""}`}><i />{recognitionStatusText(recognitionStatus)}</span>} />
        <div className="control-body">
          <label className="field-label">麦克风设备</label>
          <div className="select-wrap"><Icon name="mic" size={17} /><select value={config.selectedDevice ?? devices.find((device) => device.isDefault)?.name ?? ""} onChange={(event) => props.onDeviceChange(event.target.value)}><option value="">选择输入设备</option>{devices.map((device) => <option key={device.id} value={device.name}>{device.name}{device.isDefault ? " · 默认" : ""}</option>)}</select><span className="select-chevron">⌄</span></div>
          <div className="level-card"><div className="level-head"><span>输入电平</span><span className={audioLevel > 0.02 ? "level-on" : ""}>{Math.round(Math.min(audioLevel, 1) * 100)}%</span></div><div className="level-bars">{Array.from({ length: 28 }).map((_, index) => <i key={index} className={index / 28 < audioLevel ? levelColor(index / 28) : ""} />)}</div><div className="level-foot"><span>静音</span><span>正常输入</span><span>峰值</span></div></div>
          <div className={`input-diagnostic ${recognitionStatus === "error" ? "error" : ""}`}><i />{recognitionDiagnostic}</div>
          <div className="button-row"><button className={`primary-button ${running ? "is-running" : ""}`} onClick={running ? props.onPause : props.onStart}><Icon name={running ? "pause" : "play"} size={15} />{running ? "暂停识别" : recognitionStatus === "paused" ? "继续识别" : "开始识别"}</button><button className="secondary-button" onClick={props.onStop} disabled={recognitionStatus === "idle"}><Icon name="stop" size={14} />停止</button></div>
          <div className="hint-line"><Icon name="shield" size={13} />音频只用于实时识别，不会保存到磁盘</div>
        </div>
      </section>

      <section className="panel caption-panel">
        <PanelHeading eyebrow="LIVE OUTPUT" title="字幕预览" action={<span className="output-chip"><span />OBS 同步</span>} />
        <div className="caption-stage" style={{ textAlign: config.style.alignment }}>
          <div className="stage-grid" />
          <div className={`caption-display ${config.style.position}`}>
            {currentSubtitle?.chinese ? <div className={`caption-cn ${currentSubtitle.kind === "partial" ? "temporary" : ""}`}>{currentSubtitle.chinese}</div> : <div className="caption-empty"><div className="wave-placeholder"><i /><i /><i /><i /><i /></div><span>开始说话后，字幕会出现在这里</span></div>}
            {hasEnglish && <div className="caption-en">{currentSubtitle?.english}</div>}
          </div>
          <div className="stage-badge"><span className="live-dot" />{currentSubtitle?.kind === "partial" ? "临时字幕" : "实时预览"}</div>
        </div>
        <div className="caption-footer"><div className="caption-stat"><span className="tiny-avatar">中</span><div><b>{currentSubtitle?.chinese ? "中文识别" : "等待输入"}</b><small>{currentSubtitle?.chinese ? (currentSubtitle.kind === "partial" ? "正在聆听…" : "已确认") : "本地 ASR 引擎"}</small></div></div><div className="caption-actions"><button onClick={props.onTestTranslation}>测试翻译</button><button onClick={() => props.onOpenPage("style")}>编辑样式 <Icon name="chevron" size={13} /></button></div></div>
      </section>
    </div>

    <div className="bottom-grid">
      <section className="panel obs-panel"><PanelHeading eyebrow="BROWSER SOURCE" title="OBS 字幕源" action={<button className="text-button" onClick={() => props.onOpenPage("settings")}>服务设置 <Icon name="chevron" size={13} /></button>} /><div className="obs-body"><div className="url-box"><Icon name="link" size={16} /><span>{server.overlayUrl || "http://127.0.0.1:39071/overlay"}</span><button onClick={props.onCopyUrl}><Icon name="copy" size={15} />复制</button></div><div className="obs-tips"><span><i>1</i>复制地址</span><span><i>2</i>添加浏览器源</span><span><i>3</i>设置为透明背景</span></div></div></section>
      <section className="panel token-panel"><PanelHeading eyebrow="TOKEN USAGE" title="本次用量" action={<span className="log-count">{formatCount(tokenUsage.session.totalTokens)} tokens</span>} /><div className="token-body"><div className="token-main"><strong>{formatCount(tokenUsage.session.totalTokens)}</strong><span>本次识别总 Token</span></div><div className="token-row"><span>本地识别（估算）</span><b>{formatCount(tokenUsage.session.asrTokens)}</b></div><div className="token-row"><span>{translationTokenLabel}</span><b>{formatCount(tokenUsage.session.translationPromptTokens)} / {formatCount(tokenUsage.session.translationCompletionTokens)}</b></div><div className="token-foot">累计 {formatCount(tokenUsage.allTime.totalTokens)} · {tokenUsage.session.translationRequests} 次翻译请求</div></div></section>
      <section className="panel activity-panel"><PanelHeading eyebrow="ACTIVITY" title="运行日志" action={<span className="log-count">{logs.length} 条</span>} /><div className="activity-list">{logs.length ? logs.slice(0, 4).map((log) => <div className="activity-item" key={log.id}><span className={`activity-icon ${log.tone}`}><Icon name={log.tone === "error" ? "" : log.tone === "success" ? "check" : "grid"} size={12} /></span><span>{log.message}</span><time>{log.time}</time></div>) : <div className="empty-log">暂无运行记录</div>}</div></section>
    </div>
  </div>;
}

function Metric({ value, label, accent = false }: { value: string; label: string; accent?: boolean }) { return <div className="metric"><strong className={accent ? "accent" : ""}>{value}</strong><span>{label}</span></div>; }
function PanelHeading({ eyebrow, title, action }: { eyebrow: string; title: string; action?: ReactNode }) { return <div className="panel-heading"><div><span>{eyebrow}</span><h3>{title}</h3></div>{action}</div>; }
function recognitionStatusText(status: string) { return { idle: "待机", running: "识别中", paused: "已暂停", error: "异常" }[status] ?? status; }
function levelColor(value: number) { return value > 0.82 ? "peak" : value > 0.58 ? "warm" : "active"; }

function HardwareHint({ hardware }: { hardware: HardwareInfo | null }) {
  if (!hardware) return null;
  const memory = hardware.memoryMb ? `${Math.round(hardware.memoryMb / 1024)} GB RAM` : "内存未知";
  const gpu = hardware.cudaAvailable ? `${hardware.gpuName || "NVIDIA GPU"}${hardware.vramMb ? ` · ${Math.round(hardware.vramMb / 1024)} GB VRAM` : ""}` : "CPU 推理";
  return <div className="hardware-banner"><div className="hardware-icon"><Icon name={hardware.cudaAvailable ? "grid" : "settings"} size={16} /></div><div><b>硬件检测</b><span>{gpu} · {memory}</span></div><div className="hardware-recommend"><small>推荐模型</small><strong>{hardware.recommendedModel}</strong></div><p>{hardware.recommendationReason}</p></div>;
}

function ModelsPage({ models, progress, onDownload, onPause, onVerify, onDelete, onSelect }: { models: ModelView[]; progress: Record<string, ModelDownloadProgress>; onDownload: (model: ModelView) => void; onPause: (model: ModelView) => void; onVerify: (model: ModelView) => void; onDelete: (model: ModelView) => void; onSelect: (model: ModelView) => void }) {
  return <div className="subpage"><div className="subpage-intro"><div><p className="section-kicker">LOCAL ASR ENGINE</p><h2>选择你的识别引擎</h2><p>模型保存到程序目录的 models 文件夹，下载后无需联网即可完成语音识别。新安装建议从 2026 Qwen3-ASR 0.6B 开始。</p></div><div className="privacy-note"><Icon name="shield" size={18} /><span><b>本地推理</b><small>音频不会离开你的设备</small></span></div></div><div className="model-list">{models.map((model) => { const event = progress[model.id]; const isDownloading = model.status === "downloading" || event?.status === "downloading"; const progressLabel = event?.total ? `${Math.round((event.progress ?? model.progress) * 100)}%` : event?.downloaded ? `${formatBytes(event.downloaded)} · 下载中` : "连接中…"; return <div className={`model-card ${model.active ? "selected" : ""}`} key={model.id}><div className="model-icon">{model.id.startsWith("qwen") ? "Q" : model.id === "large-v3-turbo" ? "L" : model.id === "medium" ? "M" : "S"}</div><div className="model-main"><div className="model-title-row"><h3>{model.name}</h3>{model.recommended && <span className="recommend-tag">推荐</span>}{model.active && <span className="active-tag"><Icon name="check" size={11} />当前使用</span>}</div><p>{model.description}</p><div className="model-specs"><span><b>{model.size}</b> 下载大小</span><span><b>{model.accuracy}</b>准确率</span><span><b>{model.speed}</b>速度</span></div>{isDownloading && <div className="download-progress"><div className="progress-label"><span>{event?.message || event?.file || "正在下载…"}</span><b>{progressLabel}</b></div><div className={`progress-track ${event?.total ? "" : "indeterminate"}`}><i style={event?.total ? { width: `${(event.progress ?? model.progress) * 100}%` } : undefined} /></div></div>}{event?.status === "error" && <div className="download-error">{event.message || "下载失败，可重试"}</div>}</div><div className="model-actions">{model.status === "installed" ? <><button className="outline-button" onClick={() => onVerify(model)}><Icon name="check" size={14} />校验</button><button className="ghost-danger" onClick={() => onDelete(model)} title="删除模型"><Icon name="trash" size={15} /></button>{!model.active && <button className="dark-button" onClick={() => onSelect(model)}>使用此模型</button>}</> : isDownloading ? <button className="outline-button" onClick={() => onPause(model)}><Icon name="pause" size={13} />暂停</button> : <button className="dark-button" onClick={() => onDownload(model)}><Icon name="download" size={14} />下载模型</button>}</div></div>; })}</div><div className="model-footer"><Icon name="link" size={14} />2026 Qwen3-ASR 模型来自 Hugging Face / ModelScope；旧 Whisper 模型仅为兼容保留。下载后会记录 SHA-256 校验值。<button onClick={() => window.open("https://huggingface.co/Qwen", "_blank")}>查看来源 <Icon name="external" size={13} /></button></div></div>;
}

function StylePage({ style, currentSubtitle, onChange, onSave, onReset }: { style: SubtitleStyle; currentSubtitle: SubtitleEvent | null; onChange: (patch: Partial<SubtitleStyle>) => void; onSave: () => void; onReset: () => void }) {
  const updateText = (language: "chinese" | "english", patch: Partial<SubtitleStyle["chinese"]>) => onChange({ [language]: { ...style[language], ...patch } });
  return <div className="subpage style-page"><div className="subpage-intro"><div><p className="section-kicker">VISUAL SYSTEM</p><h2>让字幕成为画面的一部分</h2><p>调整会实时同步到本机 OBS 字幕源，保存后重启软件也会保留。</p></div><div className="style-actions"><button className="outline-button" onClick={onReset}>恢复默认</button><button className="dark-button" onClick={onSave}><Icon name="check" size={14} />保存样式</button></div></div><div className="style-layout"><section className="panel style-controls"><PanelHeading eyebrow="TYPOGRAPHY" title="文字样式" /><div className="style-section"><div className="style-section-title"><span className="language-dot cn" />中文</div><div className="two-fields"><label>字体<select value={style.chinese.fontFamily} onChange={(e) => updateText("chinese", { fontFamily: e.target.value })}><option value="Microsoft YaHei, PingFang SC, sans-serif">微软雅黑</option><option value="SimSun, serif">宋体</option><option value="Arial, sans-serif">Arial</option></select></label><label>字号 <Range value={style.chinese.fontSize} min={18} max={64} suffix="px" onChange={(value) => updateText("chinese", { fontSize: value })} /></label></div><div className="two-fields"><label>颜色<div className="color-input"><input type="color" value={style.chinese.color} onChange={(e) => updateText("chinese", { color: e.target.value })} /><span>{style.chinese.color}</span></div></label><label>不透明度<Range value={style.chinese.opacity * 100} min={20} max={100} suffix="%" onChange={(value) => updateText("chinese", { opacity: value / 100 })} /></label></div></div><div className="style-divider" /><div className="style-section"><div className="style-section-title"><span className="language-dot en" />English</div><div className="two-fields"><label>字体<select value={style.english.fontFamily} onChange={(e) => updateText("english", { fontFamily: e.target.value })}><option value="Segoe UI, Arial, sans-serif">Segoe UI</option><option value="Arial, sans-serif">Arial</option><option value="Georgia, serif">Georgia</option></select></label><label>字号<Range value={style.english.fontSize} min={14} max={48} suffix="px" onChange={(value) => updateText("english", { fontSize: value })} /></label></div><div className="two-fields"><label>颜色<div className="color-input"><input type="color" value={style.english.color} onChange={(e) => updateText("english", { color: e.target.value })} /><span>{style.english.color}</span></div></label><label>不透明度<Range value={style.english.opacity * 100} min={20} max={100} suffix="%" onChange={(value) => updateText("english", { opacity: value / 100 })} /></label></div></div><div className="style-divider" /><PanelHeading eyebrow="LAYOUT" title="布局与效果" /><div className="segmented"><button className={style.layout === "stacked" ? "active" : ""} onClick={() => onChange({ layout: "stacked" })}>上下布局</button><button className={style.layout === "sideBySide" ? "active" : ""} onClick={() => onChange({ layout: "sideBySide" })}>左右布局</button></div><div className="two-fields"><label>对齐<select value={style.alignment} onChange={(e) => onChange({ alignment: e.target.value as SubtitleStyle["alignment"] })}><option value="left">左对齐</option><option value="center">居中</option><option value="right">右对齐</option></select></label><label>位置<select value={style.position} onChange={(e) => onChange({ position: e.target.value as SubtitleStyle["position"] })}><option value="top">顶部</option><option value="center">居中</option><option value="bottom">底部</option></select></label></div><div className="two-fields"><label>背景颜色<div className="color-input"><input type="color" value={style.backgroundColor} onChange={(e) => onChange({ backgroundColor: e.target.value })} /><span>{style.backgroundColor}</span></div></label><label>背景透明度<Range value={style.backgroundOpacity * 100} min={0} max={100} suffix="%" onChange={(value) => onChange({ backgroundOpacity: value / 100 })} /></label></div><label className="toggle-row"><span><b>显示临时字幕</b><small>说话过程中显示未确认文本</small></span><button className={`toggle ${style.showTemporary ? "on" : ""}`} onClick={() => onChange({ showTemporary: !style.showTemporary })}><i /></button></label><label className="toggle-row"><span><b>显示最终字幕</b><small>停顿后保留已确认文本</small></span><button className={`toggle ${style.showFinal ? "on" : ""}`} onClick={() => onChange({ showFinal: !style.showFinal })}><i /></button></label></section><section className="style-preview-wrap"><div className="preview-label"><span>LIVE PREVIEW</span><small>OBS 画布比例 16:9</small></div><div className="style-preview" style={{ background: "linear-gradient(135deg, #182536, #0d1725 55%, #1a2933)" }}><div className="preview-watermark">PREVIEW</div><div className={`preview-caption ${style.position}`} style={{ textAlign: style.alignment, maxWidth: style.maxWidth, backgroundColor: hexToRgba(style.backgroundColor, style.backgroundOpacity), lineHeight: style.lineSpacing }}><div className="preview-cn" style={{ fontFamily: style.chinese.fontFamily, fontSize: style.chinese.fontSize, color: style.chinese.color, opacity: style.chinese.opacity, textShadow: `${style.outlineWidth}px ${style.outlineWidth}px 0 ${style.outlineColor}, 0 3px 12px ${style.shadow ? "#000b" : "transparent"}` }}>{currentSubtitle?.chinese || "欢迎使用声译实时字幕"}</div><div className="preview-en" style={{ fontFamily: style.english.fontFamily, fontSize: style.english.fontSize, color: style.english.color, opacity: style.english.opacity }}>{currentSubtitle?.english || "Welcome to live caption studio"}</div></div><div className="preview-controls"><span>中英双语</span><span><i />实时</span></div></div><div className="preview-note"><Icon name="check" size={15} /><span>样式会通过 WebSocket 自动推送到 OBS，无需刷新浏览器源。</span></div></section></div></div>;
}

function Range({ value, min, max, suffix, onChange }: { value: number; min: number; max: number; suffix: string; onChange: (value: number) => void }) { return <div className="range-wrap"><input type="range" min={min} max={max} value={value} onChange={(e) => onChange(Number(e.target.value))} /><output>{Math.round(value)}{suffix}</output></div>; }
function hexToRgba(hex: string, alpha: number) { const clean = hex.replace("#", ""); const value = Number.parseInt(clean.length === 3 ? clean.split("").map((c) => c + c).join("") : clean, 16); return `rgba(${(value >> 16) & 255}, ${(value >> 8) & 255}, ${value & 255}, ${alpha})`; }

function SettingsPageV2({ config, server, isSaving, onChange, onSave, onTest }: { config: AppConfig; server: ServerStatus; isSaving: boolean; onChange: (patch: Partial<AppConfig>) => void; onSave: () => void; onTest: () => void }) {
  const overlayUrl = server.overlayUrl || `http://${server.host || "127.0.0.1"}:${config.serverPort}/overlay`;
  const editorUrl = server.editorUrl || `http://${server.host || "127.0.0.1"}:${config.serverPort}/editor`;
  const style = config.style;
  const glossaryText = glossaryToText(config.glossary);
  const updateStyle = (patch: Partial<SubtitleStyle>) => onChange({ style: { ...style, ...patch } });
  const updateCanvas = (field: "canvasWidth" | "canvasHeight", raw: number) => {
    const oldValue = field === "canvasWidth" ? style.canvasWidth : style.canvasHeight;
    const nextValue = Math.max(field === "canvasWidth" ? 320 : 180, Math.min(field === "canvasWidth" ? 16384 : 8640, Math.round(raw || oldValue)));
    const ratio = nextValue / Math.max(1, oldValue);
    updateStyle(field === "canvasWidth" ? { canvasWidth: nextValue, subtitleX: Math.round(style.subtitleX * ratio) } : { canvasHeight: nextValue, subtitleY: Math.round(style.subtitleY * ratio) });
  };
  const updateAnchor = (field: "subtitleX" | "subtitleY", raw: number) => {
    const limit = field === "subtitleX" ? style.canvasWidth : style.canvasHeight;
    updateStyle({ [field]: Math.max(0, Math.min(limit, Math.round(Number.isFinite(raw) ? raw : 0))) });
  };
  const applyPreset = (position: SubtitleStyle["position"]) => {
    const y = position === "top" ? Math.round(style.canvasHeight * 0.16) : position === "center" ? Math.round(style.canvasHeight * 0.5) : Math.round(style.canvasHeight * 0.83);
    updateStyle({ position, subtitleX: Math.round(style.canvasWidth * 0.5), subtitleY: y });
  };
  return <div className="subpage settings-page"><div className="subpage-intro"><div><p className="section-kicker">PRIVATE CONNECTIONS</p><h2>控制你的字幕管线</h2><p>API Key 只保存在本机配置中，不会发送到 OBS 字幕页面。</p></div><button className="dark-button save-settings" onClick={onSave} disabled={isSaving}>{isSaving ? "保存中…" : "保存全部设置"}</button></div><div className="settings-grid"><section className="panel settings-card"><PanelHeading eyebrow="TRANSLATION" title="DeepSeek 翻译" action={<button className={`toggle ${config.deepseek.enabled ? "on" : ""}`} onClick={() => onChange({ deepseek: { ...config.deepseek, enabled: !config.deepseek.enabled } })}><i /></button>} /><p className="card-description">确认中文句子后发送到 DeepSeek，翻译失败时仍保留本地中文字幕。</p><label className="form-label">识别语言<select value={config.recognitionLanguage || "Chinese"} onChange={(e) => onChange({ recognitionLanguage: e.target.value as AppConfig["recognitionLanguage"] })}><option value="Chinese">中文优先（推荐）</option><option value="Chinese,English">中英混合</option><option value="auto">自动识别</option></select></label><label className="form-label">API Key<input type="password" value={config.deepseek.apiKey} placeholder="sk-…" onChange={(e) => onChange({ deepseek: { ...config.deepseek, apiKey: e.target.value } })} autoComplete="off" /></label><label className="form-label">API 地址<input value={config.deepseek.baseUrl} onChange={(e) => onChange({ deepseek: { ...config.deepseek, baseUrl: e.target.value } })} /></label><label className="form-label">模型名<input value={config.deepseek.model} onChange={(e) => onChange({ deepseek: { ...config.deepseek, model: e.target.value } })} /></label><label className="form-label">本地术语表<textarea rows={5} value={glossaryText} placeholder={"英伟达=NVIDIA\n广播=NVIDIA Broadcast\n实时字幕=live captions"} onChange={(e) => onChange({ glossary: parseGlossary(e.target.value) })} /><small className="form-hint">每行一条“中文术语=固定英文”。只替换命中的词，不发送整张词典；单独命中一条术语时可本地直出，不消耗 API Token。</small></label><button className="outline-button full-button" onClick={onTest}><Icon name="link" size={14} />发送翻译测试</button><div className="security-callout"><Icon name="shield" size={16} /><span><b>密钥隔离</b><small>翻译请求由桌面端发出，OBS 只能看到翻译结果。</small></span></div></section><section className="panel settings-card"><PanelHeading eyebrow="SERVICE" title="局域网字幕服务" /><p className="card-description">服务监听所有网卡，OBS 可以从本机或同一局域网的其他设备访问。</p><label className="form-label">监听端口<input type="number" min={1024} max={65535} value={config.serverPort} onChange={(e) => onChange({ serverPort: Number(e.target.value) || 39071 })} /></label><div className="service-preview"><div><span>OBS 字幕页面</span><b>{overlayUrl}</b></div><span className={`service-live ${server.running ? "" : "offline"}`}><i />{server.running ? "监听中" : "启动中"}</span></div><div className="service-preview"><div><span>样式编辑器</span><b>{editorUrl}</b></div><span className="service-live"><i />可访问</span></div><div className="settings-note"><Icon name="check" size={15} />监听地址：0.0.0.0:{server.port || config.serverPort} · Windows 防火墙首次提示请选择“专用网络”</div></section><section className="panel settings-card canvas-card"><PanelHeading eyebrow="OBS CANVAS" title="画布与字幕位置" /><p className="card-description">Overlay 以画布左上角为原点，X/Y 表示字幕框中心点。OBS 浏览器源建议设置为相同的画布比例。</p><div className="two-fields canvas-fields"><label>画布宽度<input type="number" min={320} max={16384} value={style.canvasWidth} onChange={(e) => updateCanvas("canvasWidth", Number(e.target.value))} /></label><label>画布高度<input type="number" min={180} max={8640} value={style.canvasHeight} onChange={(e) => updateCanvas("canvasHeight", Number(e.target.value))} /></label></div><div className="two-fields canvas-fields"><label>字幕 X<input type="number" min={0} max={style.canvasWidth} value={style.subtitleX} onChange={(e) => updateAnchor("subtitleX", Number(e.target.value))} /></label><label>字幕 Y<input type="number" min={0} max={style.canvasHeight} value={style.subtitleY} onChange={(e) => updateAnchor("subtitleY", Number(e.target.value))} /></label></div><div className="position-presets"><span>快速定位</span><button className={style.position === "top" ? "active" : ""} onClick={() => applyPreset("top")}>顶部</button><button className={style.position === "center" ? "active" : ""} onClick={() => applyPreset("center")}>居中</button><button className={style.position === "bottom" ? "active" : ""} onClick={() => applyPreset("bottom")}>底部</button></div><div className="settings-note"><Icon name="check" size={15} />当前锚点：{style.subtitleX} × {style.subtitleY} · 保存后实时同步到 OBS</div></section></div></div>;
}

function SettingsPage({ config, server, isSaving, onChange, onSave, onTest }: { config: AppConfig; server: ServerStatus; isSaving: boolean; onChange: (patch: Partial<AppConfig>) => void; onSave: () => void; onTest: () => void }) {
  const overlayUrl = server.overlayUrl || `http://${server.host || "127.0.0.1"}:${config.serverPort}/overlay`;
  const editorUrl = server.editorUrl || `http://${server.host || "127.0.0.1"}:${config.serverPort}/editor`;
  return <div className="subpage settings-page"><div className="subpage-intro"><div><p className="section-kicker">PRIVATE CONNECTIONS</p><h2>控制你的字幕管线</h2><p>API Key 只保存在本机配置中，不会发送到 OBS 字幕页面。</p></div><button className="dark-button save-settings" onClick={onSave} disabled={isSaving}>{isSaving ? "保存中…" : "保存全部设置"}</button></div><div className="settings-grid"><section className="panel settings-card"><PanelHeading eyebrow="TRANSLATION" title="DeepSeek 翻译" action={<button className={`toggle ${config.deepseek.enabled ? "on" : ""}`} onClick={() => onChange({ deepseek: { ...config.deepseek, enabled: !config.deepseek.enabled } })}><i /></button>} /><p className="card-description">确认中文句子后发送到 DeepSeek，翻译失败时仍保留本地中文字幕。</p><label className="form-label">API Key<input type="password" value={config.deepseek.apiKey} placeholder="sk-…" onChange={(e) => onChange({ deepseek: { ...config.deepseek, apiKey: e.target.value } })} autoComplete="off" /></label><label className="form-label">API 地址<input value={config.deepseek.baseUrl} onChange={(e) => onChange({ deepseek: { ...config.deepseek, baseUrl: e.target.value } })} /></label><label className="form-label">模型名<input value={config.deepseek.model} onChange={(e) => onChange({ deepseek: { ...config.deepseek, model: e.target.value } })} /></label><button className="outline-button full-button" onClick={onTest}><Icon name="link" size={14} />发送翻译测试</button><div className="security-callout"><Icon name="shield" size={16} /><span><b>密钥隔离</b><small>翻译请求由桌面端发出，OBS 只能看到翻译结果。</small></span></div></section><section className="panel settings-card"><PanelHeading eyebrow="SERVICE" title="局域网字幕服务" /><p className="card-description">服务监听所有网卡，OBS 可以从本机或同一局域网的其他设备访问。</p><label className="form-label">监听端口<input type="number" min={1024} max={65535} value={config.serverPort} onChange={(e) => onChange({ serverPort: Number(e.target.value) || 39071 })} /></label><div className="service-preview"><div><span>OBS 字幕页面</span><b>{overlayUrl}</b></div><span className={`service-live ${server.running ? "" : "offline"}`}><i />{server.running ? "监听中" : "启动中"}</span></div><div className="service-preview"><div><span>样式编辑器</span><b>{editorUrl}</b></div><span className="service-live"><i />可访问</span></div><div className="settings-note"><Icon name="check" size={15} />监听地址：0.0.0.0:{server.port || config.serverPort} · Windows 防火墙首次提示请选择“专用网络”</div></section></div></div>;
}

function Toast({ message, onClose }: { message: string; onClose: () => void }) { useEffect(() => { const timer = window.setTimeout(onClose, 2600); return () => window.clearTimeout(timer); }, [message, onClose]); return <div className="toast"><Icon name="check" size={15} />{message}</div>; }

export default App;
