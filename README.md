# 声译 · 实时字幕工作台

Windows 10/11 的本地优先实时中英字幕工具。桌面端使用 Tauri 2 + React + TypeScript，音频输入电平和模型/配置管理由 Rust 负责；识别由 faster-whisper 或 2026 Qwen3-ASR sidecar 负责；DeepSeek 只接收已确认的中文句子。

## 已实现

- 麦克风设备枚举、切换和实时 RMS 输入电平。
- small、medium、large-v3-turbo 兼容模型，以及 2026 Qwen3-ASR 0.6B/1.7B 模型目录；支持断点续传下载、暂停/继续、SHA-256 记录、校验、切换和删除。
- faster-whisper/CTranslate2 与 Qwen3-ASR worker：临时字幕、停顿后的最终字幕，以及中英文/混合语言自动检测。
- DeepSeek Chat Completions 翻译接口，API Key 只由桌面端使用；禁用或失败时保留中文识别。
- 字幕服务默认监听 `0.0.0.0:39071`，可在“连接与设置”修改端口（占用时回退到随机局域网端口）：
  - /overlay：OBS 浏览器源透明字幕页。
  - /editor：浏览器字幕样式编辑页。
  - /ws：字幕和样式 WebSocket。
  - /api/style、/api/health：本机样式/健康接口。
- React 控制台、模型管理、样式预览、DeepSeek 设置和运行日志。

## 开发

先安装 Node.js、Rust、Tauri 所需的系统依赖，然后运行：

~~~bash
npm install
npm run dev
~~~

纯前端预览可用：

~~~bash
npm run build
npm run preview
~~~

Tauri 开发/打包：

~~~bash
npm run tauri dev
npm run tauri build
~~~

## faster-whisper worker

开发环境默认从资源目录启动 sidecars/whisper_worker.py。桌面端启动识别时由 Rust/cpal（Windows 使用 WASAPI）打开所选设备，并把与电平监视完全相同的 PCM 通过标准输入送给 worker，避免 NVIDIA Broadcast 被不同音频后端重复打开后出现“电平正常但识别为空”。需要在 Python 环境安装：

~~~bash
pip install -r src-tauri/sidecars/requirements.txt
~~~

生产 Windows 包应将该脚本替换为带 Python 运行时和依赖的 whisper-worker.exe，或设置 WHISPER_WORKER_PATH 指向可执行 sidecar。应用会优先使用该环境变量，其次查找打包的 whisper-worker.exe，最后尝试 Python worker。

模型文件来自 Hugging Face / ModelScope 上的 Systran faster-whisper 兼容仓库和 Qwen/Qwen3-ASR-0.6B、Qwen/Qwen3-ASR-1.7B。Qwen3-ASR 于 2026 年发布，支持中文及多种中文方言。下载完成后，应用为每个文件和整个模型目录记录 SHA-256；识别启动前要求模型已安装且路径存在。

模型下载默认对 Qwen 先尝试 ModelScope，再尝试 `hf-mirror.com` 和官方 Hugging Face；Windows 10/11 优先使用系统 `curl.exe`，支持系统代理、TLS、断点续传、连接/低速超时和自动重试，Rust 下载器作为后备。也可以在启动应用前设置 `MODELSCOPE_ENDPOINT`、`HF_ENDPOINT` 或 `HUGGINGFACE_ENDPOINT` 指定自己的镜像地址。

模型和 `config.json` 保存在可执行文件所在目录：`models/` 和 `config.json`，不会写入用户目录。

## OBS

1. 在桌面端复制“OBS 字幕源”中的 `/overlay` 地址；该地址使用本机局域网 IPv4，其他局域网设备可直接访问。
2. OBS 添加“浏览器”源，粘贴该地址，勾选透明背景。
3. 需要在浏览器中修改样式时打开同一端口的 /editor，保存会通过 WebSocket 推送给已连接的 OBS 页面。

字幕服务监听 `0.0.0.0`，没有任何 DeepSeek API Key 路由。端口可在设置中修改；修改后保存即可重启服务。Windows 防火墙首次提示时请允许“专用网络”。配置保存在程序目录的 `config.json`，模型放在程序目录下的 `models/`。

## 验证

当前工程已验证：

- npm run build：TypeScript 检查和 Vite 生产构建通过。
- cargo fmt --all
- cargo check --no-default-features：Tauri Rust 后端通过。
- cargo test --no-default-features：测试目标编译并通过。
- npm run tauri -- build --debug --bundles app：Tauri `.app` 打包通过，sidecar 资源已进入应用包。

第一次实际识别前，需要在目标 Windows 机器安装 Python 3.10–3.13 和 worker 运行时、下载至少一个模型，并授予麦克风权限。若使用 2026 Qwen3-ASR，`requirements.txt` 已固定 `qwen-asr==0.0.6`；0.6B 更适合普通 CPU，1.7B 需要更多内存/显存。Windows 启动 worker 时优先使用 `py.exe -3`，与依赖安装使用的 Python 保持一致；麦克风会按设备默认采样率采集，再转换为 ASR 所需的 16 kHz。

## Windows 试用包

当前工作区会生成带版本号的 Windows 试用包，例如 `dist-windows/voice-caption-studio-v0.1.15-windows-x64.zip`。解压后双击 `声译-实时字幕工作台.exe` 即可启动桌面端；压缩包内已带 Windows loader、Rust 运行库和 worker 脚本。Windows 10/11 还需要系统安装 WebView2 Runtime。

若要使用真实语音识别，在解压目录打开 PowerShell，先安装 64 位 Python 运行时和依赖：

~~~powershell
py -3 -m pip install -r .\\resources\\sidecars\\requirements.txt
~~~

然后在应用的“语音模型”页下载并校验至少一个模型。DeepSeek 翻译为可选项，不填 API Key 时仍可使用中文识别和 OBS 字幕输出。
