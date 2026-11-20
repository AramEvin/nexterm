//! Shared plumbing for protocols with blocking I/O (serial ports, local
//! PTYs): a reader thread pushes output to the sink, a writer thread drains a
//! queue, and an async task handles UI commands.

use std::io::{Read, Write};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc as std_mpsc;

use tokio::sync::mpsc;

use super::protocol::OutputSink;

/// Start a thread that copies `reader` to `sink` until EOF, an error, `stop`,
/// or the sink going away; then reports on `done`.
pub fn spawn_reader(
    name: &str,
    mut reader: Box<dyn Read + Send>,
    sink: Arc<dyn OutputSink>,
    stop: Arc<AtomicBool>,
    done: mpsc::Sender<ReaderEnd>,
) {
    let _ = std::thread::Builder::new()
        .name(format!("nexterm-{name}-read"))
        .spawn(move || {
            let mut buf = vec![0u8; 16 * 1024];
            let end = loop {
                if stop.load(Ordering::Relaxed) {
                    break ReaderEnd::Stopped;
                }
                match reader.read(&mut buf) {
                    Ok(0) => break ReaderEnd::Eof,
                    Ok(n) => {
                        if !sink.data(buf[..n].to_vec()) {
                            break ReaderEnd::Stopped;
                        }
                    }
                    // Serial ports read with a timeout so `stop` is noticed.
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted
                        ) => {}
                    Err(e) => {
                        tracing::debug!(error = %e, "reader ended");
                        break ReaderEnd::Error;
                    }
                }
            };
            let _ = done.blocking_send(end);
        });
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReaderEnd {
    Eof,
    Error,
    Stopped,
}

/// A thread that writes queued input; dropping the sender ends it.
pub fn spawn_writer(name: &str, mut writer: Box<dyn Write + Send>) -> std_mpsc::Sender<Vec<u8>> {
    let (tx, rx) = std_mpsc::channel::<Vec<u8>>();
    let _ = std::thread::Builder::new()
        .name(format!("nexterm-{name}-write"))
        .spawn(move || {
            while let Ok(bytes) = rx.recv() {
                if writer
                    .write_all(&bytes)
                    .and_then(|()| writer.flush())
                    .is_err()
                {
                    break;
                }
            }
        });
    tx
}
