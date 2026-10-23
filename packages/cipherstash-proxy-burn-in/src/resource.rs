//! Cross-platform resident-memory sampling for the spawned Proxy process.
//!
//! Linux reads `/proc` for CI while other platforms use `ps`, keeping report
//! semantics identical for local and automated soak runs.

use std::time::Instant;

#[cfg(target_os = "linux")]
use std::fs;
#[cfg(not(target_os = "linux"))]
use std::process::Command;

use anyhow::{Context, Result};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct MemorySample {
    pub elapsed_millis: u128,
    pub rss_bytes: u64,
}

pub fn sample(pid: u32, started_at: Instant) -> Result<MemorySample> {
    Ok(MemorySample {
        elapsed_millis: started_at.elapsed().as_millis(),
        rss_bytes: resident_bytes(pid)?,
    })
}

#[cfg(target_os = "linux")]
fn resident_bytes(pid: u32) -> Result<u64> {
    let status = fs::read_to_string(format!("/proc/{pid}/status"))
        .with_context(|| format!("reading memory for proxy PID {pid}"))?;
    let value = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .and_then(|line| line.split_whitespace().next())
        .context("VmRSS was absent from proc status")?;
    Ok(value.parse::<u64>()? * 1024)
}

#[cfg(not(target_os = "linux"))]
fn resident_bytes(pid: u32) -> Result<u64> {
    let output = Command::new("ps")
        .args(["-o", "rss=", "-p", &pid.to_string()])
        .output()
        .with_context(|| format!("running ps for proxy PID {pid}"))?;
    anyhow::ensure!(
        output.status.success(),
        "ps could not inspect proxy PID {pid}"
    );
    let rss_kib = String::from_utf8(output.stdout)?.trim().parse::<u64>()?;
    Ok(rss_kib * 1024)
}

pub fn growth_bytes(samples: &[MemorySample]) -> u64 {
    let Some(first) = samples.first() else {
        return 0;
    };
    samples
        .last()
        .map_or(0, |last| last.rss_bytes.saturating_sub(first.rss_bytes))
}

pub fn peak_bytes(samples: &[MemorySample]) -> u64 {
    samples
        .iter()
        .map(|sample| sample.rss_bytes)
        .max()
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calculates_growth_and_peak() {
        let samples = [
            MemorySample {
                elapsed_millis: 0,
                rss_bytes: 10,
            },
            MemorySample {
                elapsed_millis: 1,
                rss_bytes: 25,
            },
            MemorySample {
                elapsed_millis: 2,
                rss_bytes: 20,
            },
        ];
        assert_eq!(growth_bytes(&samples), 10);
        assert_eq!(peak_bytes(&samples), 25);
    }
}
