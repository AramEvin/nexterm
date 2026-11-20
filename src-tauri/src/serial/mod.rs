//! Serial line terminal (Phase 8): a COM port as a terminal tab.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::session::blocking::{ReaderEnd, spawn_reader, spawn_writer};
use crate::session::monitor::ResourceEvent;
use crate::session::options::{FlowControl, Parity, SerialOptions};
use crate::session::protocol::{
    OutputSink, ProtocolKind, SessionCommand, SessionHandle, SessionStatus, TerminalProtocol,
    TerminalSize,
};

/// Read timeout: how often the reader checks whether it should stop.
const POLL: Duration = Duration::from_millis(100);

pub struct SerialProtocol;

/// Device path to open: `COM10` and above need the `\\.\` prefix on Windows.
fn device_path(port: &str) -> String {
    if cfg!(windows) && port.starts_with("COM") {
        format!(r"\\.\{port}")
    } else {
        port.to_owned()
    }
}

fn open_port(o: &SerialOptions) -> AppResult<Box<dyn serialport::SerialPort>> {
    let data_bits = match o.data_bits {
        5 => serialport::DataBits::Five,
        6 => serialport::DataBits::Six,
        7 => serialport::DataBits::Seven,
        _ => serialport::DataBits::Eight,
    };
    let parity = match o.parity {
        Parity::None => serialport::Parity::None,
        Parity::Odd => serialport::Parity::Odd,
        Parity::Even => serialport::Parity::Even,
    };
    let stop_bits = if o.stop_bits == 2 {
        serialport::StopBits::Two
    } else {
        serialport::StopBits::One
    };
    let flow = match o.flow_control {
        FlowControl::None => serialport::FlowControl::None,
        FlowControl::Software => serialport::FlowControl::Software,
        FlowControl::Hardware => serialport::FlowControl::Hardware,
    };
    serialport::new(device_path(&o.port), o.baud_rate)
        .data_bits(data_bits)
        .parity(parity)
        .stop_bits(stop_bits)
        .flow_control(flow)
        .timeout(POLL)
        .open()
        .map_err(|e| {
            tracing::debug!(error = %e, kind = ?e.kind, "serial open failed");
            AppError::Connection(match e.kind {
                serialport::ErrorKind::NoDevice
                | serialport::ErrorKind::Io(std::io::ErrorKind::NotFound) => {
                    format!("{} isn't there (unplugged?).", o.port)
                }
                serialport::ErrorKind::Io(std::io::ErrorKind::PermissionDenied) => {
                    format!(
                        "{} is in use by another program, or access is denied.",
                        o.port
                    )
                }
                _ => format!("Could not open {}.", o.port),
            })
        })
}

impl TerminalProtocol for SerialProtocol {
    type Params = SerialOptions;

    async fn open(
        params: SerialOptions,
        _size: TerminalSize,
        sink: Arc<dyn OutputSink>,
    ) -> AppResult<SessionHandle> {
        let options = params.validated()?;
        tracing::info!(port = %options.port, baud = options.baud_rate, "serial opening");
        let opts = options.clone();
        let port = tokio::task::spawn_blocking(move || open_port(&opts))
            .await
            .map_err(|_| AppError::Internal("serial open task".into()))??;
        let reader = port
            .try_clone()
            .map_err(|_| AppError::Connection(format!("Could not use {}.", options.port)))?;

        let stop = Arc::new(AtomicBool::new(false));
        let (done_tx, mut done_rx) = mpsc::channel(1);
        spawn_reader(
            "serial",
            Box::new(reader),
            Arc::clone(&sink),
            Arc::clone(&stop),
            done_tx,
        );
        let writer = spawn_writer("serial", Box::new(port));

        let (tx, mut rx) = mpsc::channel(256);
        let name = options.port.clone();
        let task = tokio::spawn(async move {
            let reason = loop {
                tokio::select! {
                    end = done_rx.recv() => break match end {
                        Some(ReaderEnd::Error) => format!("{name} was disconnected."),
                        _ => format!("{name} closed."),
                    },
                    cmd = rx.recv() => match cmd {
                        Some(SessionCommand::Write(bytes)) => {
                            if writer.send(bytes).is_err() {
                                break format!("{name} was disconnected.");
                            }
                        }
                        Some(SessionCommand::Resize(_)) | Some(SessionCommand::Monitor(None)) => {}
                        Some(SessionCommand::Monitor(Some(m))) => {
                            m.event(ResourceEvent::Unsupported { reason: "The resource bar is for SSH servers.".into() });
                        }
                        Some(SessionCommand::Close) | None => break "Closed.".into(),
                    },
                }
            };
            stop.store(true, Ordering::Relaxed);
            drop(writer); // closes the port once the reader stops too
            sink.status(SessionStatus::Closed { reason });
        });
        Ok(SessionHandle {
            kind: ProtocolKind::Serial,
            commands: tx,
            task,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn windows_com_paths() {
        if cfg!(windows) {
            assert_eq!(device_path("COM12"), r"\\.\COM12");
        } else {
            assert_eq!(device_path("/dev/ttyUSB0"), "/dev/ttyUSB0");
        }
    }
}
