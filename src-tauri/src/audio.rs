use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::SampleFormat;
use serde::Serialize;
use std::io::Write;
use std::process::ChildStdin;
use std::sync::mpsc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter};

use crate::state::AppState;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioDevice {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

/// Keeps the cpal stream that feeds the recognition worker alive.
///
/// The normal level monitor and the ASR worker must not open the virtual
/// NVIDIA Broadcast device independently: Windows may expose those opens
/// through different backends and one of them can receive silence. The
/// recognition path therefore owns one cpal stream and writes that exact PCM
/// stream to the worker's stdin.
pub struct RecognitionCapture {
    stop: Option<mpsc::Sender<()>>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl Drop for RecognitionCapture {
    fn drop(&mut self) {
        if let Some(sender) = self.stop.take() {
            let _ = sender.send(());
        }
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

pub fn list_audio_devices() -> Result<Vec<AudioDevice>, String> {
    let host = cpal::default_host();
    let default_name = host
        .default_input_device()
        .and_then(|device| device.name().ok());
    let devices = host
        .input_devices()
        .map_err(|error| format!("无法枚举麦克风：{error}"))?;
    let mut output = Vec::new();
    for device in devices {
        let name = device.name().map_err(|error| error.to_string())?;
        output.push(AudioDevice {
            id: name.clone(),
            is_default: default_name.as_deref() == Some(name.as_str()),
            name,
        });
    }
    Ok(output)
}

pub fn start_monitor(
    app: AppHandle,
    state: &AppState,
    requested_name: Option<String>,
) -> Result<(), String> {
    stop_monitor(state);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (stop_tx, stop_rx) = mpsc::channel();
    *state
        .audio_stop
        .lock()
        .map_err(|_| "音频状态锁已损坏".to_string())? = Some(stop_tx);
    let thread_app = app.clone();
    std::thread::spawn(move || {
        let result = (|| {
            let host = cpal::default_host();
            let device = choose_device(&host, requested_name.as_deref())?;
            let device_name = device.name().unwrap_or_else(|_| "麦克风".into());
            let supported = device
                .default_input_config()
                .map_err(|error| format!("无法读取「{device_name}」输入配置：{error}"))?;
            let stream_config: cpal::StreamConfig = supported.clone().into();
            let channels = stream_config.channels as usize;
            let _ = thread_app.emit(
                "recognition-status",
                format!(
                    "电平监视选择输入设备：{}（{} Hz，{} 声道）",
                    device_name, stream_config.sample_rate.0, channels
                ),
            );
            let stream = match supported.sample_format() {
                SampleFormat::F32 => {
                    build_f32_stream(&device, &stream_config, channels, thread_app.clone())
                }
                SampleFormat::I16 => {
                    build_i16_stream(&device, &stream_config, channels, thread_app.clone())
                }
                SampleFormat::U16 => {
                    build_u16_stream(&device, &stream_config, channels, thread_app.clone())
                }
                format => return Err(format!("暂不支持麦克风采样格式：{format:?}")),
            }?;
            stream
                .play()
                .map_err(|error| format!("无法启动麦克风：{error}"))?;
            let _ = thread_app.emit("recognition-status", "电平监视已启动，正在接收音频…");
            ready_tx
                .send(Ok(()))
                .map_err(|_| "音频线程已退出".to_string())?;
            let _ = stop_rx.recv();
            drop(stream);
            let _ = thread_app.emit("recognition-status", "电平监视已停止");
            Ok(())
        })();
        if let Err(error) = result {
            let _ = thread_app.emit("recognition-status", format!("电平监视启动失败：{error}"));
            let _ = ready_tx.send(Err(error));
        }
    });
    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(()),
        Ok(Err(error)) => {
            stop_monitor(state);
            Err(error)
        }
        Err(error) => {
            stop_monitor(state);
            Err(format!("启动音频线程超时：{error}"))
        }
    }
}

pub fn stop_monitor(state: &AppState) {
    if let Ok(mut stop) = state.audio_stop.lock() {
        if let Some(sender) = stop.take() {
            let _ = sender.send(());
        }
    }
}

/// Capture the selected device through the same Rust/cpal path used by the
/// level meter, and stream little-endian float32 PCM to the Python worker.
///
/// The pipe has a tiny private header so the worker can receive the actual
/// device sample rate and channel count instead of guessing them again.
pub fn start_pipe_capture(
    app: AppHandle,
    state: &AppState,
    requested_name: Option<String>,
    stdin: ChildStdin,
) -> Result<RecognitionCapture, String> {
    // The idle monitor is intentionally replaced by this stream. Opening the
    // same virtual input twice is unreliable with NVIDIA Broadcast/WASAPI.
    stop_monitor(state);
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let (stop_tx, stop_rx) = mpsc::channel();
    let thread_app = app.clone();
    let thread = std::thread::spawn(move || {
        let result = (|| {
            let host = cpal::default_host();
            let device = choose_device(&host, requested_name.as_deref())?;
            let device_name = device.name().unwrap_or_else(|_| "麦克风".into());
            let supported = device
                .default_input_config()
                .map_err(|error| format!("无法读取「{device_name}」输入配置：{error}"))?;
            let stream_config: cpal::StreamConfig = supported.clone().into();
            let channels = stream_config.channels as usize;
            let sample_rate = stream_config.sample_rate.0;
            let _ = thread_app.emit(
                "recognition-status",
                format!(
                    "识别音频管道选择输入设备：{}（{} Hz，{} 声道）",
                    device_name, sample_rate, channels
                ),
            );

            let (audio_tx, audio_rx) = mpsc::sync_channel::<Vec<f32>>(32);
            let stream_tx = audio_tx.clone();
            let stream = match supported.sample_format() {
                SampleFormat::F32 => build_pipe_f32_stream(
                    &device,
                    &stream_config,
                    channels,
                    thread_app.clone(),
                    stream_tx,
                ),
                SampleFormat::I16 => build_pipe_i16_stream(
                    &device,
                    &stream_config,
                    channels,
                    thread_app.clone(),
                    stream_tx,
                ),
                SampleFormat::U16 => build_pipe_u16_stream(
                    &device,
                    &stream_config,
                    channels,
                    thread_app.clone(),
                    stream_tx,
                ),
                format => Err(format!("暂不支持麦克风采样格式：{format:?}")),
            }?;

            let writer_app = thread_app.clone();
            let writer = std::thread::spawn(move || {
                if let Err(error) = write_audio_pipe(stdin, audio_rx, sample_rate, channels) {
                    let _ = writer_app.emit(
                        "recognition-status",
                        format!("识别音频管道写入失败：{error}"),
                    );
                }
            });

            if let Err(error) = stream.play() {
                drop(stream);
                drop(audio_tx);
                let _ = writer.join();
                return Err(format!("无法启动识别麦克风：{error}"));
            }
            let _ = thread_app.emit(
                "recognition-status",
                "识别音频管道已启动，ASR 将使用电平监视的同一份音频…",
            );
            if ready_tx.send(Ok(())).is_err() {
                drop(stream);
                drop(audio_tx);
                let _ = writer.join();
                return Err("识别音频线程已退出".into());
            }

            let _ = stop_rx.recv();
            drop(stream);
            drop(audio_tx);
            let _ = writer.join();
            let _ = thread_app.emit("recognition-status", "识别音频管道已停止");
            Ok(())
        })();
        if let Err(error) = result {
            let _ = thread_app.emit(
                "recognition-status",
                format!("识别音频管道启动失败：{error}"),
            );
            let _ = ready_tx.send(Err(error));
        }
    });

    match ready_rx.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok(())) => Ok(RecognitionCapture {
            stop: Some(stop_tx),
            thread: Some(thread),
        }),
        Ok(Err(error)) => {
            let _ = stop_tx.send(());
            // The writer may be blocked while the worker is still loading the
            // model and has not started reading stdin. Return immediately so
            // the caller can terminate the child; closing its stdin then
            // releases the detached writer thread.
            drop(thread);
            Err(error)
        }
        Err(error) => {
            let _ = stop_tx.send(());
            // See the error arm above: joining here could wait forever on a
            // full OS pipe while the worker is still initializing.
            drop(thread);
            Err(format!("启动识别音频线程超时：{error}"))
        }
    }
}

fn choose_device(host: &cpal::Host, requested_name: Option<&str>) -> Result<cpal::Device, String> {
    if let Some(name) = requested_name.filter(|name| !name.trim().is_empty()) {
        let devices = host
            .input_devices()
            .map_err(|error| format!("无法枚举麦克风：{error}"))?;
        return devices
            .filter_map(|device| device.name().ok().map(|device_name| (device, device_name)))
            .find(|(_, device_name)| device_name == name)
            .map(|(device, _)| device)
            .ok_or_else(|| format!("找不到麦克风「{name}」，可能已断开"));
    }
    host.default_input_device()
        .ok_or_else(|| "没有可用的麦克风输入设备，请检查系统权限".into())
}

fn build_f32_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    app: AppHandle,
) -> Result<cpal::Stream, String> {
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let error_app = app.clone();
    device
        .build_input_stream(
            config,
            move |data: &[f32], _| {
                emit_level(&app, data.iter().copied(), channels, &mut last_emit);
            },
            move |error| input_error(&error_app, error),
            None,
        )
        .map_err(|error| error.to_string())
}

fn build_i16_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    app: AppHandle,
) -> Result<cpal::Stream, String> {
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let error_app = app.clone();
    device
        .build_input_stream(
            config,
            move |data: &[i16], _| {
                emit_level(
                    &app,
                    data.iter().map(|sample| *sample as f32 / i16::MAX as f32),
                    channels,
                    &mut last_emit,
                );
            },
            move |error| input_error(&error_app, error),
            None,
        )
        .map_err(|error| error.to_string())
}

fn build_u16_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    app: AppHandle,
) -> Result<cpal::Stream, String> {
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let error_app = app.clone();
    device
        .build_input_stream(
            config,
            move |data: &[u16], _| {
                emit_level(
                    &app,
                    data.iter()
                        .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0),
                    channels,
                    &mut last_emit,
                );
            },
            move |error| input_error(&error_app, error),
            None,
        )
        .map_err(|error| error.to_string())
}

fn build_pipe_f32_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    app: AppHandle,
    audio_tx: mpsc::SyncSender<Vec<f32>>,
) -> Result<cpal::Stream, String> {
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let error_app = app.clone();
    device
        .build_input_stream(
            config,
            move |data: &[f32], _| {
                emit_level(&app, data.iter().copied(), channels, &mut last_emit);
                let _ = audio_tx.try_send(data.to_vec());
            },
            move |error| input_error(&error_app, error),
            None,
        )
        .map_err(|error| error.to_string())
}

fn build_pipe_i16_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    app: AppHandle,
    audio_tx: mpsc::SyncSender<Vec<f32>>,
) -> Result<cpal::Stream, String> {
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let error_app = app.clone();
    device
        .build_input_stream(
            config,
            move |data: &[i16], _| {
                let samples: Vec<f32> = data
                    .iter()
                    .map(|sample| *sample as f32 / i16::MAX as f32)
                    .collect();
                emit_level(&app, samples.iter().copied(), channels, &mut last_emit);
                let _ = audio_tx.try_send(samples);
            },
            move |error| input_error(&error_app, error),
            None,
        )
        .map_err(|error| error.to_string())
}

fn build_pipe_u16_stream(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    channels: usize,
    app: AppHandle,
    audio_tx: mpsc::SyncSender<Vec<f32>>,
) -> Result<cpal::Stream, String> {
    let mut last_emit = Instant::now() - Duration::from_secs(1);
    let error_app = app.clone();
    device
        .build_input_stream(
            config,
            move |data: &[u16], _| {
                let samples: Vec<f32> = data
                    .iter()
                    .map(|sample| (*sample as f32 / u16::MAX as f32) * 2.0 - 1.0)
                    .collect();
                emit_level(&app, samples.iter().copied(), channels, &mut last_emit);
                let _ = audio_tx.try_send(samples);
            },
            move |error| input_error(&error_app, error),
            None,
        )
        .map_err(|error| error.to_string())
}

fn write_audio_pipe(
    mut stdin: ChildStdin,
    audio_rx: mpsc::Receiver<Vec<f32>>,
    sample_rate: u32,
    channels: usize,
) -> Result<(), String> {
    let channels = channels.max(1).min(u16::MAX as usize);
    let mut header = Vec::with_capacity(16);
    header.extend_from_slice(b"VCAUDIO1");
    header.extend_from_slice(&sample_rate.to_le_bytes());
    header.extend_from_slice(&(channels as u16).to_le_bytes());
    header.extend_from_slice(&1_u16.to_le_bytes());
    stdin
        .write_all(&header)
        .map_err(|error| format!("无法写入音频管道头：{error}"))?;

    for samples in audio_rx {
        let mut bytes = Vec::with_capacity(samples.len() * std::mem::size_of::<f32>());
        for sample in samples {
            bytes.extend_from_slice(&sample.to_le_bytes());
        }
        stdin
            .write_all(&bytes)
            .map_err(|error| format!("无法写入音频采样：{error}"))?;
    }
    Ok(())
}

fn emit_level<I>(app: &AppHandle, samples: I, channels: usize, last_emit: &mut Instant)
where
    I: Iterator<Item = f32>,
{
    if last_emit.elapsed() < Duration::from_millis(55) {
        return;
    }
    let mut sum = 0.0_f32;
    let mut count = 0_u32;
    for sample in samples {
        sum += sample * sample;
        count += 1;
    }
    if count == 0 {
        return;
    }
    let rms = (sum / count as f32).sqrt();
    let normalized = (rms * (channels.max(1) as f32).sqrt() * 2.8).clamp(0.0, 1.0);
    let _ = app.emit("audio-level", normalized);
    *last_emit = Instant::now();
}

fn input_error(app: &AppHandle, error: cpal::StreamError) {
    eprintln!("audio input stream error: {error}");
    let _ = app.emit("recognition-status", format!("麦克风音频流错误：{error}"));
}
