//! Agentless resource monitor over SSH (Phase 6).
//!
//! A second channel on the terminal's own connection runs a small read-only
//! `/bin/sh` loop that prints `/proc/stat`, `/proc/meminfo`, `/proc/loadavg`,
//! `/proc/uptime` and `df -Pk /` every few seconds. Nothing is installed on the
//! server and nothing goes through the user's shell or its history.
//!
//! The server's output is untrusted: lines and blocks are size-capped, unknown
//! lines are ignored and every number is parsed defensively.

use std::sync::Arc;

use russh::client::{self, Handle};
use russh::{Channel, ChannelMsg};
use tokio::task::JoinHandle;

use super::client::ClientHandler;
use crate::session::monitor::{MonitorSink, ResourceEvent, ResourceSample};

/// Seconds between readings on the server.
pub const INTERVAL_SECS: u32 = 3;

const BEGIN: &str = "NXT:begin";
const END: &str = "NXT:end";
const UNSUPPORTED: &str = "NXT:unsupported";
/// Longer lines are dropped (a real `/proc` line is far shorter).
const MAX_LINE: usize = 4096;
/// Lines per reading (≈ one per CPU core plus a dozen).
const MAX_BLOCK_LINES: usize = 1100;

/// The command sent in the exec request. Interpreted by the user's login
/// shell, which only has to run `/bin/sh -c '<script>'` (works from bash,
/// zsh, fish, csh). The script contains no single quotes.
pub fn command() -> String {
    let script = [
        r#"if [ "$(uname -s 2>/dev/null)" != Linux ]; then echo NXT:unsupported; exit 0; fi"#,
        // `do` is followed by a space, not `;` (that would be a syntax error).
        "while :; do echo NXT:begin",
        r#"grep "^cpu" /proc/stat"#,
        r#"grep -E "^(MemTotal|MemAvailable|MemFree|Buffers|Cached|SwapTotal|SwapFree):" /proc/meminfo"#,
        r#"echo "load $(cat /proc/loadavg)""#,
        r#"echo "up $(cat /proc/uptime)""#,
        r#"echo "disk $(LC_ALL=C df -Pk / 2>/dev/null | tail -n 1)""#,
        "echo NXT:end",
        &format!("sleep {INTERVAL_SECS}"),
    ]
    .join("; ");
    // One line on purpose: csh rejects newlines inside quotes.
    let script = format!("{script}; done");
    debug_assert!(!script.contains('\''));
    format!("/bin/sh -c '{script}'")
}

/// CPU counters from the aggregate `cpu` line.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct CpuTimes {
    total: u64,
    idle: u64,
}

#[derive(Debug, Default)]
struct Block {
    cpu: Option<CpuTimes>,
    cores: u32,
    mem_total: Option<u64>,
    mem_available: Option<u64>,
    mem_free: Option<u64>,
    buffers: Option<u64>,
    cached: Option<u64>,
    swap_total: Option<u64>,
    swap_free: Option<u64>,
    load: Option<[f32; 3]>,
    uptime: Option<u64>,
    /// (used, total, available) KiB.
    disk: Option<(u64, u64, u64)>,
}

/// Turns the script's output into events. Kept free of I/O for tests.
#[derive(Debug, Default)]
pub struct Parser {
    partial: Vec<u8>,
    in_block: bool,
    block: Block,
    block_lines: usize,
    previous_cpu: Option<CpuTimes>,
}

impl Parser {
    /// Feed raw bytes; returns the events completed by them.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<ResourceEvent> {
        let mut events = Vec::new();
        for &b in bytes {
            if b == b'\n' {
                let line = std::mem::take(&mut self.partial);
                if let Some(ev) = self.line(&String::from_utf8_lossy(&line)) {
                    events.push(ev);
                }
            } else if self.partial.len() < MAX_LINE {
                self.partial.push(b);
            }
        }
        events
    }

    fn line(&mut self, raw: &str) -> Option<ResourceEvent> {
        let line = raw.trim();
        match line {
            BEGIN => {
                self.in_block = true;
                self.block = Block::default();
                self.block_lines = 0;
                return None;
            }
            END if self.in_block => {
                self.in_block = false;
                return self.finish();
            }
            UNSUPPORTED => {
                return Some(ResourceEvent::Unsupported {
                    reason: "Only Linux servers report resources.".into(),
                });
            }
            _ => {}
        }
        if !self.in_block {
            return None; // shell start-up noise (.bashrc echo, motd)
        }
        self.block_lines += 1;
        if self.block_lines > MAX_BLOCK_LINES {
            self.in_block = false; // runaway output: drop this reading
            return None;
        }
        self.parse_line(line);
        None
    }

    fn parse_line(&mut self, line: &str) {
        let mut parts = line.split_whitespace();
        let Some(key) = parts.next() else { return };
        let b = &mut self.block;
        match key {
            "cpu" => b.cpu = parse_cpu(parts),
            k if k.starts_with("cpu") && k[3..].bytes().all(|c| c.is_ascii_digit()) => {
                b.cores = b.cores.saturating_add(1);
            }
            "MemTotal:" => b.mem_total = parse_u64(parts.next()),
            "MemAvailable:" => b.mem_available = parse_u64(parts.next()),
            "MemFree:" => b.mem_free = parse_u64(parts.next()),
            "Buffers:" => b.buffers = parse_u64(parts.next()),
            "Cached:" => b.cached = parse_u64(parts.next()),
            "SwapTotal:" => b.swap_total = parse_u64(parts.next()),
            "SwapFree:" => b.swap_free = parse_u64(parts.next()),
            "load" => {
                let v: Vec<f32> = parts
                    .take(3)
                    .filter_map(|p| p.parse::<f32>().ok())
                    .filter(|f| f.is_finite() && *f >= 0.0)
                    .collect();
                if let [a, b5, c] = v[..] {
                    b.load = Some([a, b5, c]);
                }
            }
            "up" => {
                b.uptime = parts
                    .next()
                    .and_then(|p| p.parse::<f64>().ok())
                    .filter(|f| f.is_finite() && *f >= 0.0)
                    // Truncation intended: whole seconds, clamped to u64.
                    .map(|f| f.min(u64::MAX as f64) as u64);
            }
            // `disk <fs> <1K-blocks> <used> <available> <capacity> <mount>`
            "disk" => {
                let cols: Vec<&str> = parts.collect();
                if cols.len() >= 5
                    && let (Some(total), Some(used), Some(avail)) = (
                        parse_u64(cols.get(1).copied()),
                        parse_u64(cols.get(2).copied()),
                        parse_u64(cols.get(3).copied()),
                    )
                    && total > 0
                {
                    b.disk = Some((used.min(total), total, avail.min(total)));
                }
            }
            _ => {}
        }
    }

    fn finish(&mut self) -> Option<ResourceEvent> {
        let b = std::mem::take(&mut self.block);
        let mem_total = b.mem_total.filter(|t| *t > 0)?;
        // MemAvailable exists since Linux 3.14; estimate on older kernels.
        let available = b.mem_available.unwrap_or_else(|| {
            b.mem_free
                .unwrap_or(0)
                .saturating_add(b.buffers.unwrap_or(0))
                .saturating_add(b.cached.unwrap_or(0))
        });
        let swap_total = b.swap_total.unwrap_or(0);
        let cpu_percent = match (self.previous_cpu, b.cpu) {
            (Some(prev), Some(now)) if now.total > prev.total && now.idle >= prev.idle => {
                let total = now.total - prev.total;
                let idle = (now.idle - prev.idle).min(total);
                #[allow(clippy::cast_precision_loss)]
                let busy = (total - idle) as f64 / total as f64 * 100.0;
                #[allow(clippy::cast_possible_truncation)]
                Some(busy.clamp(0.0, 100.0) as f32)
            }
            _ => None,
        };
        if b.cpu.is_some() {
            self.previous_cpu = b.cpu;
        }
        Some(ResourceEvent::Sample(ResourceSample {
            cpu_percent,
            cores: b.cores.max(1),
            mem_used_kib: mem_total.saturating_sub(available.min(mem_total)),
            mem_total_kib: mem_total,
            swap_used_kib: swap_total
                .saturating_sub(b.swap_free.unwrap_or(swap_total).min(swap_total)),
            swap_total_kib: swap_total,
            disk_used_kib: b.disk.map(|d| d.0),
            disk_total_kib: b.disk.map(|d| d.1),
            disk_avail_kib: b.disk.map(|d| d.2),
            load: b.load.unwrap_or([0.0; 3]),
            uptime_secs: b.uptime.unwrap_or(0),
        }))
    }
}

fn parse_u64(raw: Option<&str>) -> Option<u64> {
    raw?.parse().ok()
}

/// `cpu user nice system idle iowait irq softirq steal [guest guest_nice]`.
/// Guest time is already counted in user/nice, so it's left out of the total.
fn parse_cpu<'a>(parts: impl Iterator<Item = &'a str>) -> Option<CpuTimes> {
    let v: Vec<u64> = parts.take(8).map_while(|p| p.parse().ok()).collect();
    if v.len() < 4 {
        return None;
    }
    let idle = v[3].saturating_add(v.get(4).copied().unwrap_or(0));
    let total = v.iter().fold(0u64, |acc, x| acc.saturating_add(*x));
    Some(CpuTimes { total, idle })
}

/// A running monitor. Dropping it stops the readings (the exec channel is
/// closed with the task).
pub struct Monitor(JoinHandle<()>);

impl Drop for Monitor {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// Open the exec channel and start reading. Errors are reported to `sink`
/// as `Unsupported`; the terminal session is never affected.
pub async fn start(handle: &Handle<ClientHandler>, sink: Arc<dyn MonitorSink>) -> Option<Monitor> {
    let channel = match open(handle).await {
        Ok(c) => c,
        Err(e) => {
            tracing::debug!(error = %e, "resource monitor: exec channel refused");
            sink.event(ResourceEvent::Unsupported {
                reason: "The server refused the monitoring channel.".into(),
            });
            return None;
        }
    };
    Some(Monitor(tokio::spawn(run(channel, sink))))
}

async fn open(handle: &Handle<ClientHandler>) -> Result<Channel<client::Msg>, russh::Error> {
    let channel = handle.channel_open_session().await?;
    channel.exec(false, command()).await?;
    Ok(channel)
}

async fn run(mut channel: Channel<client::Msg>, sink: Arc<dyn MonitorSink>) {
    let mut parser = Parser::default();
    let mut unsupported = false;
    loop {
        match channel.wait().await {
            Some(ChannelMsg::Data { data }) => {
                for event in parser.feed(&data) {
                    unsupported |= matches!(event, ResourceEvent::Unsupported { .. });
                    if !sink.event(event) {
                        let _ = channel.close().await;
                        return;
                    }
                }
            }
            Some(ChannelMsg::Close | ChannelMsg::Eof) | None => break,
            Some(ChannelMsg::ExitStatus { exit_status }) if exit_status != 0 && !unsupported => {
                unsupported = true;
                sink.event(ResourceEvent::Unsupported {
                    reason: "This server can't report resources (no /bin/sh or /proc).".into(),
                });
            }
            Some(_) => {}
        }
    }
    if !unsupported {
        sink.event(ResourceEvent::Stopped {
            reason: "Resource readings stopped.".into(),
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BLOCK_1: &str = "NXT:begin
cpu  100 0 100 800 0 0 0 0 0 0
cpu0 50 0 50 400 0 0 0 0 0 0
cpu1 50 0 50 400 0 0 0 0 0 0
MemTotal:        8000000 kB
MemFree:         1000000 kB
MemAvailable:    6000000 kB
Buffers:          100000 kB
Cached:          2000000 kB
SwapTotal:       2000000 kB
SwapFree:        1500000 kB
load 0.50 1.25 2.00 3/456 7890
up 93784.12 180000.00
disk /dev/sda1 100000 25000 75000 25% /
NXT:end
";

    fn sample(ev: &ResourceEvent) -> &ResourceSample {
        match ev {
            ResourceEvent::Sample(s) => s,
            other => panic!("expected sample, got {other:?}"),
        }
    }

    #[test]
    fn parses_a_full_reading() {
        let mut p = Parser::default();
        let evs = p.feed(BLOCK_1.as_bytes());
        assert_eq!(evs.len(), 1);
        let s = sample(&evs[0]);
        assert_eq!(s.cpu_percent, None); // first reading
        assert_eq!(s.cores, 2);
        assert_eq!(s.mem_total_kib, 8_000_000);
        assert_eq!(s.mem_used_kib, 2_000_000);
        assert_eq!(s.swap_used_kib, 500_000);
        assert_eq!(s.disk_used_kib, Some(25_000));
        assert_eq!(s.disk_total_kib, Some(100_000));
        assert_eq!(s.disk_avail_kib, Some(75_000));
        assert_eq!(s.load, [0.5, 1.25, 2.0]);
        assert_eq!(s.uptime_secs, 93_784);
    }

    #[test]
    fn cpu_percent_comes_from_the_difference_of_two_readings() {
        let mut p = Parser::default();
        p.feed(BLOCK_1.as_bytes());
        // +100 busy, +100 idle → 50 %.
        let second = BLOCK_1.replace("cpu  100 0 100 800", "cpu  150 0 150 900");
        let evs = p.feed(second.as_bytes());
        let cpu = sample(&evs[0]).cpu_percent.unwrap();
        assert!((cpu - 50.0).abs() < 0.01, "{cpu}");
    }

    #[test]
    fn chunked_input_and_noise_are_handled() {
        let mut p = Parser::default();
        let text = format!("Welcome!\nrandom noise\n{BLOCK_1}");
        let mut events = Vec::new();
        for chunk in text.as_bytes().chunks(7) {
            events.extend(p.feed(chunk));
        }
        assert_eq!(events.len(), 1);
    }

    #[test]
    fn unsupported_marker_is_reported() {
        let mut p = Parser::default();
        let evs = p.feed(b"NXT:unsupported\n");
        assert!(matches!(evs[0], ResourceEvent::Unsupported { .. }));
    }

    #[test]
    fn old_kernels_without_memavailable_are_estimated() {
        let mut p = Parser::default();
        let text = BLOCK_1.replace("MemAvailable:    6000000 kB\n", "");
        let s = p.feed(text.as_bytes()).remove(0);
        // free 1.0M + buffers 0.1M + cached 2.0M = 3.1M available.
        assert_eq!(sample(&s).mem_used_kib, 4_900_000);
    }

    #[test]
    fn hostile_output_is_bounded_and_does_not_panic() {
        let mut p = Parser::default();
        let mut text = String::from("NXT:begin\n");
        text.push_str(&"x".repeat(1_000_000));
        text.push('\n');
        text.push_str("MemTotal: 99999999999999999999999 kB\n");
        text.push_str("load NaN inf -1\nup -5\ndisk a b c d e f\ncpu 1 2\n");
        text.push_str("NXT:end\n");
        assert!(p.partial.capacity() <= 2 * MAX_LINE || p.partial.is_empty());
        // MemTotal overflowed → no reading rather than garbage.
        assert!(p.feed(text.as_bytes()).is_empty());

        let mut many = String::from("NXT:begin\n");
        for _ in 0..(MAX_BLOCK_LINES + 10) {
            many.push_str("cpu0 1 1 1 1\n");
        }
        many.push_str("MemTotal: 10 kB\nNXT:end\n");
        assert!(p.feed(many.as_bytes()).is_empty());
    }

    #[test]
    fn disk_used_never_exceeds_total() {
        let mut p = Parser::default();
        let text = BLOCK_1.replace(
            "disk /dev/sda1 100000 25000",
            "disk /dev/sda1 100000 250000",
        );
        let s = p.feed(text.as_bytes()).remove(0);
        assert_eq!(sample(&s).disk_used_kib, Some(100_000));
    }

    #[test]
    fn command_has_no_single_quotes_inside_the_script() {
        let cmd = command();
        assert!(cmd.starts_with("/bin/sh -c '") && cmd.ends_with('\''));
        assert_eq!(cmd.matches('\'').count(), 2);
        assert!(!cmd.contains('\n') && !cmd.contains("do;"));
    }
}
