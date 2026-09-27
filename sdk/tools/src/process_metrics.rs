//! Cumulative CPU observations for isolated preview measurements.
//! ps is a diagnostic child command; no OS process state is changed.
use anyhow::Result;
#[cfg(target_os = "macos")]
pub const RESOLUTION: f64 = 0.01;
#[cfg(windows)]
pub const RESOLUTION: f64 = 0.015625;
#[cfg(not(any(target_os = "macos", windows)))]
pub const RESOLUTION: f64 = 1.0;

#[cfg(unix)]
pub fn sample<const N: usize>(pids: &[u32; N]) -> Result<[f64; N]> {
    use anyhow::{Context, ensure};
    use std::{process::Command, time::Duration};
    let mut command = Command::new("ps");
    command.env("LC_ALL", "C");
    command.args([
        "-p",
        &pids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(","),
        "-o",
        "pid=,time=",
    ]);
    let output =
        cranpose_ide_host::process::capture(command, None, Duration::from_secs(2), || false)?;
    ensure!(output.status.success(), "CPU sampling failed");
    let mut values = [None; N];
    for line in std::str::from_utf8(&output.stdout)?.lines() {
        let mut columns = line.split_whitespace();
        let pid: u32 = columns.next().context("Missing process PID")?.parse()?;
        let time = parse_time(columns.next().context("Missing process CPU time")?)?;
        if let Some(index) = pids.iter().position(|candidate| *candidate == pid) {
            values[index] = Some(time);
        }
    }
    let mut samples = [0.0; N];
    for (index, value) in values.into_iter().enumerate() {
        samples[index] =
            value.with_context(|| format!("Process {} exited during measurement", pids[index]))?;
    }
    Ok(samples)
}
#[cfg(windows)]
pub fn sample<const N: usize>(pids: &[u32; N]) -> Result<[f64; N]> {
    let mut samples = [0.0; N];
    for (index, pid) in pids.iter().enumerate() {
        samples[index] = cranpose_ide_host::process::cpu_time(*pid)?.as_secs_f64();
    }
    Ok(samples)
}
#[cfg(not(any(unix, windows)))]
pub fn sample<const N: usize>(_pids: &[u32; N]) -> Result<[f64; N]> {
    anyhow::bail!("CPU sampling requires macOS, Linux or Windows")
}
#[cfg(any(unix, test))]
fn parse_time(text: &str) -> Result<f64> {
    use anyhow::ensure;
    let (days, clock) = match text.split_once('-') {
        Some((days, clock)) => (days.parse::<u64>()?, clock),
        None => (0, text),
    };
    let parts: Vec<_> = clock.split(':').collect();
    ensure!((2..=3).contains(&parts.len()), "Invalid CPU clock: {text}");
    let mut seconds = 0.0;
    for part in parts {
        seconds = seconds * 60.0 + part.parse::<f64>()?;
    }
    ensure!(
        seconds.is_finite() && seconds >= 0.0,
        "Invalid CPU time: {text}"
    );
    Ok(days as f64 * 86400.0 + seconds)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_macos_linux_and_long_running_process_clocks() {
        assert_eq!(parse_time("0:00.59").expect("macOS"), 0.59);
        assert_eq!(parse_time("01:02:03").expect("Linux"), 3723.0);
        assert_eq!(parse_time("2-03:04:05.25").expect("days"), 183845.25);
        for invalid in ["", "1", "00:NaN", "00:-1", "00:00:00:00"] {
            assert!(parse_time(invalid).is_err());
        }
    }
}
