#!/usr/bin/env python3
"""Local ASR sidecar for the subtitle pipeline.

The Rust host owns configuration, WebSocket broadcasting and translation. This
process receives PCM from the selected microphone, runs either faster-whisper
or Qwen3-ASR, and emits JSON subtitle events on stdout.

The desktop host normally supplies ``--stdin-audio``. In that mode Rust owns
the cpal/WASAPI capture stream and this process consumes the exact same PCM
that drives the input meter. Direct sounddevice capture remains available for
standalone diagnostics and development.
"""

from __future__ import annotations

import argparse
from contextlib import nullcontext
from collections import deque
import json
import os
from pathlib import Path
import queue
import sys
import threading
import time
from typing import Any

_emit_lock = threading.Lock()


def emit(payload: dict[str, Any]) -> None:
    with _emit_lock:
        # The worker stdout is a Windows pipe, not a console. ASCII-only JSON
        # avoids locale/code-page failures before Rust can decode Chinese
        # strings; serde_json restores the original Unicode text on receipt.
        sys.stdout.write(json.dumps(payload, ensure_ascii=True) + "\n")
        sys.stdout.flush()


def emit_status(stage: str, message: str) -> None:
    emit({"type": "status", "stage": stage, "message": message})


def normalize_device_name(name: str) -> str:
    return "".join(character for character in name.casefold() if character.isalnum())


def host_api_name(sounddevice: Any, info: dict[str, Any]) -> str:
    try:
        hostapis = sounddevice.query_hostapis()
        hostapi_index = int(info.get("hostapi", -1))
        if 0 <= hostapi_index < len(hostapis):
            return str(hostapis[hostapi_index].get("name", "未知音频接口"))
    except Exception:
        pass
    return "未知音频接口"


def select_input_device(sounddevice: Any, requested_name: str | None) -> tuple[int | None, dict[str, Any], str]:
    """Resolve the UI device name to a PortAudio index.

    Rust/cpal and PortAudio can expose the same Windows virtual microphone with
    slightly different host-api suffixes. Matching only the raw name can pick
    no device or the wrong duplicate. Prefer an exact/fuzzy input match and
    prefer WASAPI, which uses Windows shared mode for virtual microphones.
    """
    entries: list[dict[str, Any]] = []
    for index, raw_info in enumerate(sounddevice.query_devices()):
        info = dict(raw_info)
        try:
            max_input_channels = int(info.get("max_input_channels", 0) or 0)
        except (TypeError, ValueError):
            max_input_channels = 0
        if max_input_channels <= 0:
            continue
        name = str(info.get("name", "")).strip()
        entries.append({
            "index": index,
            "info": info,
            "name": name,
            "normalized": normalize_device_name(name),
            "host_api": host_api_name(sounddevice, info),
        })

    if not entries:
        raise RuntimeError("PortAudio 没有检测到可用的输入设备，请检查 Windows 麦克风权限。")

    def preference(entry: dict[str, Any]) -> tuple[int, int]:
        return (0 if "WASAPI" in entry["host_api"].upper() else 1, int(entry["index"]))

    requested = (requested_name or "").strip()
    if requested:
        normalized_requested = normalize_device_name(requested)
        exact = [entry for entry in entries if entry["normalized"] == normalized_requested]
        matches = exact or [
            entry
            for entry in entries
            if normalized_requested in entry["normalized"]
            or entry["normalized"] in normalized_requested
        ]
        if not matches:
            available = "；".join(
                f"{entry['name']} [{entry['host_api']}]" for entry in entries[:12]
            )
            raise RuntimeError(
                f"PortAudio 找不到输入设备「{requested}」。当前可用输入设备：{available}"
            )
        selected = sorted(matches, key=preference)[0]
        return int(selected["index"]), selected["info"], selected["host_api"]

    default_index: int | None = None
    try:
        default_device = sounddevice.default.device
        if isinstance(default_device, (tuple, list)):
            candidate = int(default_device[0])
        else:
            candidate = int(default_device)
        if candidate >= 0:
            default_index = candidate
    except (AttributeError, TypeError, ValueError, IndexError):
        pass
    if default_index is not None:
        for entry in entries:
            if entry["index"] == default_index:
                return default_index, entry["info"], entry["host_api"]

    try:
        default_info = dict(sounddevice.query_devices(None, "input"))
        default_normalized = normalize_device_name(str(default_info.get("name", "")))
        matches = [entry for entry in entries if entry["normalized"] == default_normalized]
        if matches:
            selected = sorted(matches, key=preference)[0]
            return int(selected["index"]), selected["info"], selected["host_api"]
        return None, default_info, host_api_name(sounddevice, default_info)
    except Exception:
        selected = sorted(entries, key=preference)[0]
        return int(selected["index"]), selected["info"], selected["host_api"]


def resample_audio(audio: np.ndarray, source_rate: int, target_rate: int) -> np.ndarray:
    """Convert one microphone block to the 16 kHz rate required by the ASR models."""
    import numpy as np

    if source_rate == target_rate or audio.size == 0:
        return audio.astype(np.float32, copy=False)
    target_length = max(1, int(round(audio.size * target_rate / source_rate)))
    source_positions = np.linspace(0, audio.size - 1, num=target_length)
    return np.interp(source_positions, np.arange(audio.size), audio).astype(np.float32)


def configure_qwen_generation(model: Any) -> None:
    """Avoid a harmless Transformers pad-token warning on every decode."""
    for candidate in (model, getattr(model, "model", None)):
        generation_config = getattr(candidate, "generation_config", None)
        if generation_config is None:
            continue
        if getattr(generation_config, "pad_token_id", None) is not None:
            return
        eos_token_id = getattr(generation_config, "eos_token_id", None)
        if isinstance(eos_token_id, (list, tuple)):
            eos_token_id = eos_token_id[-1] if eos_token_id else None
        if eos_token_id is not None:
            generation_config.pad_token_id = int(eos_token_id)
            return


def estimate_text_tokens(model: Any, text: str) -> tuple[int, bool]:
    """Count transcript tokens when a local tokenizer is available.

    Local ASR libraries do not expose a billing-style token counter. The value
    is therefore deliberately marked estimated, even when it comes from the
    model tokenizer, so the UI never presents it as an API billable count.
    """
    tokenizer_candidates = [
        getattr(model, "tokenizer", None),
        getattr(getattr(model, "processor", None), "tokenizer", None),
        getattr(getattr(model, "model", None), "tokenizer", None),
    ]
    for tokenizer in tokenizer_candidates:
        if tokenizer is None:
            continue
        try:
            encoded = tokenizer(text, add_special_tokens=False)
            input_ids = encoded.get("input_ids") if isinstance(encoded, dict) else None
            if input_ids is None:
                continue
            if hasattr(input_ids, "numel"):
                return max(1, int(input_ids.numel())), True
            if isinstance(input_ids, (list, tuple)):
                if input_ids and isinstance(input_ids[0], (list, tuple)):
                    input_ids = input_ids[0]
                return max(1, len(input_ids)), True
        except Exception:
            continue
    return max(1, len(text)), True


def inference_context(torch_module: Any) -> Any:
    """Use the lightest available no-grad context across torch versions."""
    inference_mode = getattr(torch_module, "inference_mode", None)
    if callable(inference_mode):
        return inference_mode()
    no_grad = getattr(torch_module, "no_grad", None)
    if callable(no_grad):
        return no_grad()
    return nullcontext()


def create_voice_detector() -> Any | None:
    """Create an optional WebRTC VAD used before the ASR decoder.

    The dependency is deliberately optional at runtime: an older manually
    installed worker still falls back to the adaptive energy gate below.
    """
    try:
        import webrtcvad

        return webrtcvad.Vad(2)
    except Exception:
        return None


def voice_activity_ratio(detector: Any | None, audio: Any, sample_rate: int) -> float | None:
    """Return the voiced-frame ratio for a 16 kHz float32 audio block."""
    if detector is None or sample_rate not in (8000, 16000, 32000, 48000):
        return None
    frame_samples = int(sample_rate * 0.03)
    usable_samples = (len(audio) // frame_samples) * frame_samples
    if usable_samples <= 0:
        return None
    try:
        import numpy as np

        pcm = np.clip(audio[:usable_samples], -1.0, 1.0)
        pcm = (pcm * 32767.0).astype(np.int16).tobytes()
        frame_bytes = frame_samples * 2
        total_frames = usable_samples // frame_samples
        voiced_frames = 0
        for offset in range(0, len(pcm), frame_bytes):
            frame = pcm[offset : offset + frame_bytes]
            if len(frame) != frame_bytes:
                continue
            voiced_frames += int(detector.is_speech(frame, sample_rate))
        return voiced_frames / max(1, total_frames)
    except Exception:
        return None


def faster_whisper_language(language: str) -> str | None:
    """Map the UI's language choices to faster-whisper's language codes."""
    if language in ("auto", "Chinese,English"):
        return None
    if language == "Chinese":
        return "zh"
    return language


def read_exact(stream: Any, size: int) -> bytes:
    chunks: list[bytes] = []
    remaining = size
    while remaining > 0:
        chunk = stream.read(remaining)
        if not chunk:
            break
        chunks.append(chunk)
        remaining -= len(chunk)
    return b"".join(chunks)


def read_audio_pipe_header() -> tuple[int, int]:
    """Read the Rust host's 16-byte PCM stream header."""
    header = read_exact(sys.stdin.buffer, 16)
    if len(header) != 16 or header[:8] != b"VCAUDIO1":
        raise RuntimeError("Rust 音频管道头无效或未收到")
    sample_rate = int.from_bytes(header[8:12], "little")
    channels = int.from_bytes(header[12:14], "little")
    sample_format = int.from_bytes(header[14:16], "little")
    if sample_rate <= 0 or channels <= 0:
        raise RuntimeError(f"Rust 音频管道参数无效：{sample_rate} Hz，{channels} 声道")
    if sample_format != 1:
        raise RuntimeError(f"Rust 音频管道采样格式不支持：{sample_format}")
    return sample_rate, channels


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--model-path", required=True)
    parser.add_argument("--device", default="auto")
    parser.add_argument("--compute-type", default="auto")
    parser.add_argument("--language", default="auto")
    parser.add_argument("--device-name", default=None)
    parser.add_argument("--noise-floor", type=float, default=0.0)
    parser.add_argument("--speech-threshold", type=float, default=0.0)
    parser.add_argument("--silence-threshold", type=float, default=0.0)
    parser.add_argument("--silence-ms", type=int, default=700)
    parser.add_argument("--no-auto-calibrate", action="store_true")
    parser.add_argument("--disable-vad", action="store_true")
    parser.add_argument(
        "--stdin-audio",
        action="store_true",
        help="从 Rust/cpal 音频管道读取 PCM，而不是重新打开 sounddevice",
    )
    args = parser.parse_args()

    emit_status("dependencies", "正在检查 Python 依赖…")
    try:
        import numpy as np
        if args.stdin_audio:
            sd = None
        else:
            import sounddevice as sd
    except ImportError as error:
        emit({
            "type": "error",
            "error": f"sidecar 缺少 Python 依赖：{error}（当前 Python：{sys.executable}）。请用该 Python 安装 resources/sidecars/requirements.txt。",
        })
        return 2

    model_path = Path(args.model_path)
    is_qwen = (model_path / "config.json").is_file() and (
        (model_path / "model.safetensors").is_file()
        or (model_path / "model.safetensors.index.json").is_file()
    )
    if is_qwen:
        emit_status("model", "正在加载 Qwen3-ASR 模型，首次启动可能需要一段时间…")
        try:
            import torch
            try:
                from transformers.utils import logging as transformers_logging

                transformers_logging.set_verbosity_error()
            except Exception:
                pass
            from qwen_asr import Qwen3ASRModel

            use_cuda = args.device != "cpu" and torch.cuda.is_available()
            device = "cuda:0" if use_cuda else "cpu"
            dtype = torch.float16 if use_cuda else torch.float32
            if not use_cuda:
                cpu_threads = max(1, min(4, (os.cpu_count() or 4) // 2))
                try:
                    torch.set_num_threads(cpu_threads)
                except Exception:
                    pass
                try:
                    torch.set_num_interop_threads(1)
                except Exception:
                    pass
            try:
                torch.set_grad_enabled(False)
            except Exception:
                pass
            model = Qwen3ASRModel.from_pretrained(
                str(model_path),
                dtype=dtype,
                device_map=device,
                max_inference_batch_size=1,
                max_new_tokens=256,
                local_files_only=True,
            )
            configure_qwen_generation(model)
            for candidate in (model, getattr(model, "model", None)):
                eval_method = getattr(candidate, "eval", None)
                if callable(eval_method):
                    eval_method()
        except Exception as error:  # pragma: no cover - depends on local torch/model
            emit({
                "type": "error",
                "error": f"无法加载 Qwen3-ASR 模型：{type(error).__name__}: {error}。当前 Python：{sys.executable}。请确认已安装 requirements.txt 中的 qwen-asr、torch，且模型文件完整。",
            })
            return 3

        def transcribe_audio(audio: np.ndarray, sample_rate: int) -> tuple[str, str]:
            language = None if args.language == "auto" else args.language
            with inference_context(torch):
                results = model.transcribe(
                    audio=(audio.astype(np.float32, copy=False), sample_rate),
                    language=language,
                )
            result = results[0] if results else None
            text = str(getattr(result, "text", "") or "").strip() if result else ""
            detected_language = str(getattr(result, "language", "") or "").strip() if result else ""

            # Qwen's automatic language detector can occasionally return an
            # empty result for a short Chinese utterance even though the audio
            # gate has already detected speech. Retry only that case with the
            # documented Chinese language hint; English and other languages
            # still use automatic detection on the first pass.
            if not text and language is None:
                with inference_context(torch):
                    fallback_results = model.transcribe(
                        audio=(audio.astype(np.float32, copy=False), sample_rate),
                        language="Chinese",
                    )
                fallback = fallback_results[0] if fallback_results else None
                text = str(getattr(fallback, "text", "") or "").strip() if fallback else ""
                detected_language = str(getattr(fallback, "language", "") or "Chinese").strip() if fallback else ""
            return text, detected_language

    else:
        emit_status("model", "正在加载 faster-whisper 模型…")
        try:
            from faster_whisper import WhisperModel

            compute_type = "default" if args.compute_type == "auto" else args.compute_type
            model = WhisperModel(
                str(model_path),
                device=args.device,
                compute_type=compute_type,
                cpu_threads=max(1, min(4, (os.cpu_count() or 4) // 2)),
                num_workers=1,
            )
        except Exception as error:  # pragma: no cover - depends on local CUDA/model
            emit({"type": "error", "error": f"无法加载 faster-whisper 模型：{error}"})
            return 3

        def transcribe_audio(audio: np.ndarray, _sample_rate: int) -> tuple[str, str]:
            segments, _info = model.transcribe(
                audio,
                language=faster_whisper_language(args.language),
                beam_size=3,
                vad_filter=True,
                condition_on_previous_text=False,
            )
            return " ".join(segment.text.strip() for segment in segments).strip(), ""

    # Inference can be slower than realtime on a CPU. A bounded queue keeps
    # recognition focused on recent audio instead of building an unbounded
    # backlog while the model is running.
    audio_queue: queue.Queue[np.ndarray] = queue.Queue(maxsize=32)
    latest_rms = 0.0
    last_callback_time = 0.0
    received_samples = 0

    def callback(indata: np.ndarray, _frames: int, _time: Any, status: Any) -> None:
        nonlocal latest_rms, last_callback_time, received_samples
        if status:
            message = f"音频输入状态：{status}"
            print(message, file=sys.stderr, flush=True)
            emit_status("microphone", message)
        if getattr(indata, "ndim", 1) > 1:
            chunk = np.mean(indata, axis=1).astype(np.float32, copy=False)
        else:
            chunk = indata.astype(np.float32, copy=False)
        chunk = chunk.copy()
        latest_rms = float(np.sqrt(np.mean(np.square(chunk)))) if len(chunk) else 0.0
        last_callback_time = time.monotonic()
        received_samples += len(chunk)
        try:
            audio_queue.put_nowait(chunk)
        except queue.Full:
            # Drop the oldest block so a slow inference never makes the UI
            # transcribe stale speech several seconds after it was spoken.
            try:
                audio_queue.get_nowait()
            except queue.Empty:
                pass
            try:
                audio_queue.put_nowait(chunk)
            except queue.Full:
                pass

    emit_status("microphone", "正在打开麦克风…")
    stream: Any | None = None
    pipe_thread: threading.Thread | None = None
    pipe_stop = threading.Event()
    capture_rate = 16_000
    try:
        if args.stdin_audio:
            capture_rate, pipe_channels = read_audio_pipe_header()
            device_name = args.device_name or "Rust/cpal 输入设备"
            emit_status(
                "microphone",
                f"已连接 Rust/cpal 音频管道：{device_name}（{capture_rate} Hz，{pipe_channels} 声道）",
            )

            def read_pipe() -> None:
                pending = bytearray()
                bytes_per_frame = pipe_channels * 4
                try:
                    while not pipe_stop.is_set():
                        raw_bytes = sys.stdin.buffer.read(16_384)
                        if not raw_bytes:
                            break
                        pending.extend(raw_bytes)
                        usable = len(pending) - (len(pending) % bytes_per_frame)
                        if usable <= 0:
                            continue
                        frame_bytes = bytes(pending[:usable])
                        del pending[:usable]
                        raw = np.frombuffer(frame_bytes, dtype="<f4")
                        frames = usable // bytes_per_frame
                        indata = (
                            raw.reshape(frames, pipe_channels)
                            if pipe_channels > 1
                            else raw
                        )
                        callback(indata, frames, None, None)
                    if not pipe_stop.is_set():
                        emit_status("microphone", "Rust/cpal 音频管道已关闭")
                except Exception as error:
                    emit(
                        {
                            "type": "error",
                            "error": f"读取 Rust/cpal 音频管道失败：{type(error).__name__}: {error}",
                        }
                    )

            pipe_thread = threading.Thread(
                target=read_pipe,
                name="rust-audio-pipe",
                daemon=True,
            )
            pipe_thread.start()
            emit_status("microphone", "已启动同源音频输入，正在监听语音…")
        else:
            device_index, device_info, host_api = select_input_device(sd, args.device_name)
            device_name = str(device_info.get("name", "默认输入设备"))
            max_input_channels = max(1, int(device_info.get("max_input_channels", 1) or 1))
            capture_rate = int(round(float(device_info["default_samplerate"])))
            if capture_rate <= 0:
                capture_rate = 16_000
            capture_blocksize = max(1, int(round(capture_rate * 0.1)))
            extra_settings = None
            if "WASAPI" in host_api.upper() and hasattr(sd, "WasapiSettings"):
                try:
                    extra_settings = sd.WasapiSettings(exclusive=False)
                except Exception:
                    extra_settings = None
            emit_status(
                "microphone",
                f"已选择输入设备：{device_name}（{host_api}，{capture_rate} Hz，最多 {max_input_channels} 声道）",
            )

            def open_stream(channels: int) -> Any:
                options: dict[str, Any] = {
                    # Many Windows microphones expose 44.1/48 kHz but reject a
                    # direct 16 kHz stream. Capture at the device default and
                    # resample each block before sending it to the ASR model.
                    "samplerate": capture_rate,
                    "blocksize": capture_blocksize,
                    "channels": channels,
                    "dtype": "float32",
                    "device": device_index,
                    "callback": callback,
                }
                if extra_settings is not None:
                    options["extra_settings"] = extra_settings
                return sd.InputStream(**options)

            try:
                stream = open_stream(1)
            except Exception as first_error:
                if max_input_channels < 2:
                    raise first_error
                emit_status("microphone", f"单声道打开失败，尝试双声道：{first_error}")
                stream = open_stream(2)
            stream.start()
    except Exception as error:
        emit({"type": "error", "error": f"无法打开麦克风：{type(error).__name__}: {error}。请检查 Windows 麦克风权限、NVIDIA Broadcast 输入设备和共享模式。"})
        return 4

    emit({"type": "ready"})
    emit_status("listening", "麦克风已打开，正在监听语音…")
    level_stop = threading.Event()

    def report_audio_level() -> None:
        last_status_report = time.monotonic() - 2.0
        while not level_stop.wait(0.1):
            now = time.monotonic()
            level = min(1.0, latest_rms * 8.0)
            callback_age = now - last_callback_time
            if callback_age > 0.25:
                level = 0.0
            emit({"type": "audio-level", "level": level})
            if now - last_status_report >= 2.0:
                if received_samples == 0:
                    message = "设备流已启动，但尚未收到音频回调；请检查 Windows 麦克风权限和 NVIDIA Broadcast 输出"
                elif callback_age > 0.25:
                    message = f"音频回调曾收到 {received_samples} 个采样，但最近 {callback_age:.1f} 秒没有新数据"
                else:
                    message = f"音频回调正常：累计 {received_samples} 个采样，当前 RMS {latest_rms:.4f}"
                emit_status("microphone", message)
                last_status_report = now

    level_thread = threading.Thread(target=report_audio_level, name="audio-level", daemon=True)
    level_thread.start()
    # Qwen3-ASR's Transformers backend is a complete-utterance decoder, not
    # a low-latency partial decoder. Keep a short pre-roll, wait for a natural
    # pause, and submit one complete utterance instead of repeatedly decoding
    # the same growing buffer every 0.9 seconds.
    pre_roll: deque[np.ndarray] = deque(maxlen=3)
    chunks: list[np.ndarray] = []
    buffered_samples = 0
    silence_samples = 0
    had_speech = False
    peak_rms = 0.0
    last_no_audio_status = 0.0
    sample_rate = 16_000
    # WebRTC VAD is combined with an RMS gate. The RMS gate is important for
    # physical microphones such as Maono Fairy whose steady analogue noise can
    # otherwise be classified as voiced by VAD. A per-device profile can turn
    # VAD off completely and provide explicit thresholds.
    voice_detector = None if args.disable_vad else create_voice_detector()
    if voice_detector is None:
        emit_status("microphone", "未启用 WebRTC VAD，将使用设备噪声阈值断句")
    else:
        emit_status("microphone", "已启用 WebRTC VAD，并叠加设备噪声阈值断句")
    configured_noise_floor = max(0.0, min(0.9, args.noise_floor))
    configured_speech_threshold = max(0.0, min(0.9, args.speech_threshold))
    configured_silence_threshold = max(0.0, min(0.9, args.silence_threshold))
    noise_floor = configured_noise_floor or 0.003
    auto_calibrate = not args.no_auto_calibrate
    calibration_target_samples = int(sample_rate * 1.2)
    calibration_samples = 0
    calibration_levels: list[float] = []
    calibration_complete = not auto_calibrate
    min_utterance_samples = int(sample_rate * 0.5)
    silence_ms = max(250, min(3_000, int(args.silence_ms)))
    silence_to_finalize_samples = int(sample_rate * silence_ms / 1000)
    max_utterance_samples = int(sample_rate * 10.0)

    try:
        while True:
            try:
                input_chunk = audio_queue.get(timeout=1.0)
            except queue.Empty:
                now = time.monotonic()
                if received_samples == 0 and now - last_no_audio_status >= 3.0:
                    emit_status("microphone", "尚未收到麦克风音频，请检查系统权限和输入设备…")
                    last_no_audio_status = now
                continue
            chunk = resample_audio(input_chunk, capture_rate, sample_rate)
            rms = float(np.sqrt(np.mean(np.square(chunk)))) if len(chunk) else 0.0
            vad_ratio = voice_activity_ratio(voice_detector, chunk, sample_rate)
            if not calibration_complete:
                # Use VAD only to decide whether an initial block is clearly
                # voiced; the actual floor is always measured from RMS. This
                # makes auto calibration useful for both quiet and noisy mics.
                calibration_is_quiet = rms <= 0.02
                if vad_ratio is not None:
                    calibration_is_quiet = vad_ratio < 0.20
                if calibration_is_quiet:
                    calibration_levels.append(rms)
                    calibration_samples += len(chunk)
                    pre_roll.append(chunk)
                    if calibration_samples < calibration_target_samples:
                        continue
                    ordered_levels = sorted(calibration_levels)
                    measured_floor = ordered_levels[max(0, len(ordered_levels) // 3)]
                    noise_floor = max(configured_noise_floor, measured_floor, 0.0005)
                    calibration_complete = True
                    pre_roll.clear()
                    emit_status(
                        "microphone",
                        f"噪声底校准完成（RMS {noise_floor:.4f}），将按设备阈值断句",
                    )
                    continue
                noise_floor = max(
                    configured_noise_floor,
                    min(calibration_levels, default=0.003),
                )
                calibration_complete = True
                emit_status("microphone", "检测到语音，跳过启动噪声校准")

            on_threshold = configured_speech_threshold or max(0.006, noise_floor * 1.6)
            off_threshold = configured_silence_threshold or max(0.003, noise_floor * 1.18)
            energy_active = rms > (off_threshold if had_speech else on_threshold)
            if vad_ratio is not None:
                speech_active = energy_active and vad_ratio >= 0.10
            else:
                speech_active = energy_active
            if not had_speech:
                if not speech_active:
                    pre_roll.append(chunk)
                    continue
                chunks = list(pre_roll) + [chunk]
                buffered_samples = sum(len(part) for part in chunks)
                peak_rms = rms
                silence_samples = 0
                had_speech = True
                pre_roll.clear()
                continue

            chunks.append(chunk)
            buffered_samples += len(chunk)
            peak_rms = max(peak_rms, rms)
            if speech_active:
                silence_samples = 0
            else:
                silence_samples += len(chunk)

            should_finalize = (
                buffered_samples >= min_utterance_samples
                and silence_samples >= silence_to_finalize_samples
            ) or buffered_samples >= max_utterance_samples
            if not should_finalize:
                continue

            audio = np.concatenate(chunks)
            duration = len(audio) / sample_rate
            emit_status("transcribing", f"正在识别 {duration:.1f} 秒语音…")
            inference_started = time.monotonic()
            try:
                text, detected_language = transcribe_audio(audio, sample_rate)
            except Exception as error:  # pragma: no cover - depends on local model runtime
                emit({
                    "type": "error",
                    "error": f"语音推理失败：{type(error).__name__}: {error}",
                })
                text = ""
                detected_language = ""
            inference_duration = time.monotonic() - inference_started
            if text:
                asr_tokens, asr_tokens_estimated = estimate_text_tokens(model, text)
                emit(
                    {
                        "type": "token-usage",
                        "asrTokens": asr_tokens,
                        "asrTokensEstimated": asr_tokens_estimated,
                    }
                )
                subtitle_payload = {"kind": "partial", "text": text}
                if detected_language:
                    subtitle_payload["language"] = detected_language
                emit({"type": "subtitle", **subtitle_payload})
                subtitle_payload["kind"] = "final"
                emit({"type": "subtitle", **subtitle_payload})
                language_note = f"（{detected_language}）" if detected_language else ""
                emit_status("listening", f"已识别到{language_note}语音：{text}（推理 {inference_duration:.1f} 秒），继续监听…")
            else:
                emit_status(
                    "microphone",
                    f"模型返回空文本：已提交 {duration:.1f} 秒音频（峰值 RMS {peak_rms:.4f}）",
                )

            del audio
            chunks.clear()
            pre_roll.clear()
            buffered_samples = 0
            silence_samples = 0
            peak_rms = 0.0
            had_speech = False
    except KeyboardInterrupt:
        pass
    finally:
        level_stop.set()
        level_thread.join(timeout=1.0)
        pipe_stop.set()
        if pipe_thread is not None:
            pipe_thread.join(timeout=1.0)
        if stream is not None:
            stream.stop()
            stream.close()
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
