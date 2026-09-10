use serde::Serialize;
use std::process::Command;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HardwareInfo {
    pub cuda_available: bool,
    pub gpu_name: Option<String>,
    pub vram_mb: u64,
    pub memory_mb: u64,
    pub recommended_model: String,
    pub recommendation_reason: String,
}

pub fn inspect() -> HardwareInfo {
    let (cuda_available, gpu_name, vram_mb) = detect_nvidia();
    let memory_mb = detect_memory_mb();
    let (recommended_model, recommendation_reason) = if cuda_available && vram_mb >= 6_000 {
        (
            "qwen3-asr-1.7b".into(),
            "检测到 NVIDIA CUDA 和充足显存，优先使用 2026 高精度模型".into(),
        )
    } else if memory_mb >= 10_000 {
        (
            "qwen3-asr-0.6b".into(),
            "内存条件适合 Qwen3-ASR 0.6B，兼顾中文准确率和速度".into(),
        )
    } else {
        (
            "qwen3-asr-0.6b".into(),
            "当前硬件优先使用 Qwen3-ASR 0.6B，降低内存占用".into(),
        )
    };
    HardwareInfo {
        cuda_available,
        gpu_name,
        vram_mb,
        memory_mb,
        recommended_model,
        recommendation_reason,
    }
}

fn detect_nvidia() -> (bool, Option<String>, u64) {
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output();
    let Ok(output) = output else {
        return (false, None, 0);
    };
    if !output.status.success() {
        return (false, None, 0);
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    let line = stdout.lines().next().unwrap_or_default();
    let mut values = line.split(',').map(str::trim);
    let name = values
        .next()
        .filter(|value| !value.is_empty())
        .map(str::to_string);
    let vram_mb = values
        .next()
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or_default();
    (name.is_some(), name, vram_mb)
}

#[cfg(target_os = "macos")]
fn detect_memory_mb() -> u64 {
    Command::new("sysctl")
        .args(["-n", "hw.memsize"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| value.trim().parse::<u64>().ok())
        .map(|bytes| bytes / 1024 / 1024)
        .unwrap_or_default()
}

#[cfg(target_os = "windows")]
fn detect_memory_mb() -> u64 {
    Command::new("wmic")
        .args(["computersystem", "get", "TotalPhysicalMemory", "/value"])
        .output()
        .ok()
        .and_then(|output| String::from_utf8(output.stdout).ok())
        .and_then(|value| {
            value.lines().find_map(|line| {
                line.strip_prefix("TotalPhysicalMemory=")?
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
        })
        .map(|bytes| bytes / 1024 / 1024)
        .unwrap_or_default()
}

#[cfg(target_os = "linux")]
fn detect_memory_mb() -> u64 {
    std::fs::read_to_string("/proc/meminfo")
        .ok()
        .and_then(|value| {
            value.lines().find_map(|line| {
                line.strip_prefix("MemTotal:")?
                    .split_whitespace()
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
        })
        .map(|kb| kb / 1024)
        .unwrap_or_default()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn detect_memory_mb() -> u64 {
    0
}
