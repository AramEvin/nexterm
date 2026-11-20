//! Telnet client (Phase 8). Unencrypted by design; the UI shows a badge.
//!
//! [`Negotiator`] is a pure state machine (bytes in → screen data + replies)
//! so option negotiation is unit-tested without a network. It agrees to the
//! options a terminal needs (echo, suppress-go-ahead, terminal type, window
//! size, binary) and refuses everything else. The server's bytes are
//! untrusted: subnegotiations are size-capped and unknown options ignored.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;

use crate::error::{AppError, AppResult};
use crate::session::monitor::ResourceEvent;
use crate::session::protocol::{
    OutputSink, ProtocolKind, SessionCommand, SessionHandle, SessionStatus, TerminalProtocol,
    TerminalSize,
};

const IAC: u8 = 255;
const DONT: u8 = 254;
const DO: u8 = 253;
const WONT: u8 = 252;
const WILL: u8 = 251;
const SB: u8 = 250;
const SE: u8 = 240;

const OPT_BINARY: u8 = 0;
const OPT_ECHO: u8 = 1;
const OPT_SGA: u8 = 3;
const OPT_TTYPE: u8 = 24;
const OPT_NAWS: u8 = 31;

const TTYPE_IS: u8 = 0;
const TTYPE_SEND: u8 = 1;
const TERMINAL_TYPE: &[u8] = b"XTERM-256COLOR";
/// Longest subnegotiation kept (real ones are a few bytes).
const MAX_SB: usize = 1024;
const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Data,
    /// Previous byte was CR (a following NUL is padding).
    Cr,
    Iac,
    Verb(u8),
    Sb,
    SbIac,
}

/// Telnet option negotiation and stream decoding.
#[derive(Debug)]
pub struct Negotiator {
    state: State,
    sb: Vec<u8>,
    /// Options we perform (WILL accepted by the server's DO).
    local: [bool; 256],
    /// Options the server performs.
    remote: [bool; 256],
    size: TerminalSize,
}

/// Result of feeding server bytes.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Decoded {
    /// Bytes for the screen.
    pub data: Vec<u8>,
    /// Bytes to send back to the server.
    pub reply: Vec<u8>,
}

impl Negotiator {
    pub fn new(size: TerminalSize) -> Self {
        Self {
            state: State::Data,
            sb: Vec::new(),
            local: [false; 256],
            remote: [false; 256],
            size,
        }
    }

    /// What a client says first: it will report type and size, and asks the
    /// server to echo and drop go-aheads.
    pub fn greeting(&self) -> Vec<u8> {
        vec![
            IAC, WILL, OPT_TTYPE, IAC, WILL, OPT_NAWS, IAC, DO, OPT_SGA, IAC, DO, OPT_ECHO,
        ]
    }

    fn accept_local(opt: u8) -> bool {
        matches!(opt, OPT_TTYPE | OPT_NAWS | OPT_BINARY | OPT_SGA)
    }

    fn accept_remote(opt: u8) -> bool {
        matches!(opt, OPT_ECHO | OPT_SGA | OPT_BINARY)
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Decoded {
        let mut out = Decoded::default();
        for &b in bytes {
            self.state = match self.state {
                State::Data | State::Cr => match b {
                    IAC => State::Iac,
                    0 if self.state == State::Cr => State::Data, // CR NUL → CR
                    b'\r' => {
                        out.data.push(b);
                        State::Cr
                    }
                    _ => {
                        out.data.push(b);
                        State::Data
                    }
                },
                State::Iac => match b {
                    IAC => {
                        out.data.push(IAC); // escaped 0xFF
                        State::Data
                    }
                    WILL | WONT | DO | DONT => State::Verb(b),
                    SB => {
                        self.sb.clear();
                        State::Sb
                    }
                    _ => State::Data, // NOP, GA, AYT, …: nothing to do
                },
                State::Verb(verb) => {
                    self.negotiate(verb, b, &mut out.reply);
                    State::Data
                }
                State::Sb => match b {
                    IAC => State::SbIac,
                    _ => {
                        if self.sb.len() < MAX_SB {
                            self.sb.push(b);
                        }
                        State::Sb
                    }
                },
                State::SbIac => match b {
                    SE => {
                        self.subnegotiation(&mut out.reply);
                        State::Data
                    }
                    IAC => {
                        if self.sb.len() < MAX_SB {
                            self.sb.push(IAC);
                        }
                        State::Sb
                    }
                    _ => State::Sb,
                },
            };
        }
        out
    }

    fn negotiate(&mut self, verb: u8, opt: u8, reply: &mut Vec<u8>) {
        let i = usize::from(opt);
        match verb {
            DO if Self::accept_local(opt) => {
                if !self.local[i] {
                    self.local[i] = true;
                    reply.extend([IAC, WILL, opt]);
                }
                if opt == OPT_NAWS {
                    reply.extend(self.naws());
                }
            }
            DO => reply.extend([IAC, WONT, opt]),
            DONT if self.local[i] => {
                self.local[i] = false;
                reply.extend([IAC, WONT, opt]);
            }
            WILL if Self::accept_remote(opt) => {
                if !self.remote[i] {
                    self.remote[i] = true;
                    reply.extend([IAC, DO, opt]);
                }
            }
            WILL => reply.extend([IAC, DONT, opt]),
            WONT if self.remote[i] => {
                self.remote[i] = false;
                reply.extend([IAC, DONT, opt]);
            }
            _ => {}
        }
    }

    fn subnegotiation(&mut self, reply: &mut Vec<u8>) {
        if self.sb.first() == Some(&OPT_TTYPE) && self.sb.get(1) == Some(&TTYPE_SEND) {
            reply.extend([IAC, SB, OPT_TTYPE, TTYPE_IS]);
            reply.extend_from_slice(TERMINAL_TYPE);
            reply.extend([IAC, SE]);
        }
    }

    /// Window size report (RFC 1073); 0xFF bytes are doubled.
    fn naws(&self) -> Vec<u8> {
        let mut v = vec![IAC, SB, OPT_NAWS];
        for b in self
            .size
            .cols
            .to_be_bytes()
            .into_iter()
            .chain(self.size.rows.to_be_bytes())
        {
            v.push(b);
            if b == IAC {
                v.push(IAC);
            }
        }
        v.extend([IAC, SE]);
        v
    }

    /// Remember the new size; returns the report to send if NAWS is on.
    pub fn resize(&mut self, size: TerminalSize) -> Option<Vec<u8>> {
        self.size = size;
        self.local[usize::from(OPT_NAWS)].then(|| self.naws())
    }

    /// Keystrokes → wire bytes: 0xFF doubled; Enter (CR) becomes CR LF unless
    /// both sides agreed on binary mode.
    pub fn encode(&self, input: &[u8]) -> Vec<u8> {
        let binary = self.local[usize::from(OPT_BINARY)];
        let mut out = Vec::with_capacity(input.len() + 4);
        for (i, &b) in input.iter().enumerate() {
            match b {
                IAC => out.extend([IAC, IAC]),
                b'\r' if !binary && input.get(i + 1) != Some(&b'\n') => out.extend(*b"\r\n"),
                _ => out.push(b),
            }
        }
        out
    }
}

pub struct TelnetParams {
    pub host: String,
    pub port: u16,
}

pub struct TelnetProtocol;

impl TerminalProtocol for TelnetProtocol {
    type Params = TelnetParams;

    async fn open(
        params: TelnetParams,
        size: TerminalSize,
        sink: Arc<dyn OutputSink>,
    ) -> AppResult<SessionHandle> {
        let TelnetParams { host, port } = params;
        tracing::info!(%host, port, "Telnet connecting");
        let stream =
            tokio::time::timeout(CONNECT_TIMEOUT, TcpStream::connect((host.as_str(), port)))
                .await
                .map_err(|_| {
                    AppError::Connection(format!("Timed out connecting to {host}:{port}."))
                })?
                .map_err(|e| {
                    tracing::debug!(error = %e, "telnet connect failed");
                    AppError::Connection(format!("Could not connect to {host}:{port}."))
                })?;
        let _ = stream.set_nodelay(true);
        let (tx, rx) = mpsc::channel(256);
        let task = tokio::spawn(async move {
            let reason = run(stream, size, rx, sink.as_ref()).await;
            sink.status(SessionStatus::Closed { reason });
        });
        Ok(SessionHandle {
            kind: ProtocolKind::Telnet,
            commands: tx,
            task,
        })
    }
}

async fn run(
    mut stream: TcpStream,
    size: TerminalSize,
    mut rx: mpsc::Receiver<SessionCommand>,
    sink: &dyn OutputSink,
) -> String {
    let mut neg = Negotiator::new(size);
    if stream.write_all(&neg.greeting()).await.is_err() {
        return "The connection was closed.".into();
    }
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        tokio::select! {
            read = stream.read(&mut buf) => match read {
                Ok(0) | Err(_) => return "The server closed the connection.".into(),
                Ok(n) => {
                    let decoded = neg.feed(&buf[..n]);
                    if !decoded.reply.is_empty() && stream.write_all(&decoded.reply).await.is_err() {
                        return "The connection was closed.".into();
                    }
                    if !decoded.data.is_empty() && !sink.data(decoded.data) {
                        return "Closed.".into();
                    }
                }
            },
            cmd = rx.recv() => match cmd {
                Some(SessionCommand::Write(bytes)) => {
                    if stream.write_all(&neg.encode(&bytes)).await.is_err() {
                        return "The connection was closed.".into();
                    }
                }
                Some(SessionCommand::Resize(size)) => {
                    if let Some(report) = neg.resize(size) {
                        let _ = stream.write_all(&report).await;
                    }
                }
                Some(SessionCommand::Monitor(Some(m))) => {
                    m.event(ResourceEvent::Unsupported { reason: "The resource bar is for SSH servers.".into() });
                }
                Some(SessionCommand::Monitor(None)) => {}
                Some(SessionCommand::Close) | None => {
                    let _ = stream.shutdown().await;
                    return "Closed.".into();
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: TerminalSize = TerminalSize {
        cols: 120,
        rows: 40,
    };

    #[test]
    fn plain_text_passes_and_iac_iac_is_0xff() {
        let mut n = Negotiator::new(SIZE);
        let d = n.feed(b"login: \xff\xffx");
        assert_eq!(d.data, b"login: \xffx");
        assert!(d.reply.is_empty());
    }

    #[test]
    fn agrees_to_terminal_options_and_refuses_others() {
        let mut n = Negotiator::new(SIZE);
        // Server: WILL ECHO, DO TTYPE, DO NAWS, DO 39 (new-environ), WILL 34 (linemode).
        let d = n.feed(&[
            IAC, WILL, OPT_ECHO, IAC, DO, OPT_TTYPE, IAC, DO, OPT_NAWS, IAC, DO, 39, IAC, WILL, 34,
        ]);
        let mut expected = vec![IAC, DO, OPT_ECHO, IAC, WILL, OPT_TTYPE, IAC, WILL, OPT_NAWS];
        expected.extend([IAC, SB, OPT_NAWS, 0, 120, 0, 40, IAC, SE]);
        expected.extend([IAC, WONT, 39, IAC, DONT, 34]);
        assert_eq!(d.reply, expected);
        assert!(d.data.is_empty());
        // Repeating a request we already agreed to doesn't loop.
        assert!(n.feed(&[IAC, WILL, OPT_ECHO]).reply.is_empty());
    }

    #[test]
    fn answers_terminal_type_request() {
        let mut n = Negotiator::new(SIZE);
        let d = n.feed(&[IAC, SB, OPT_TTYPE, TTYPE_SEND, IAC, SE, b'o', b'k']);
        let mut expected = vec![IAC, SB, OPT_TTYPE, TTYPE_IS];
        expected.extend_from_slice(b"XTERM-256COLOR");
        expected.extend([IAC, SE]);
        assert_eq!(d.reply, expected);
        assert_eq!(d.data, b"ok");
    }

    #[test]
    fn negotiation_split_across_reads() {
        let mut n = Negotiator::new(SIZE);
        let mut reply = Vec::new();
        for b in [IAC, WILL, OPT_ECHO] {
            reply.extend(n.feed(&[b]).reply);
        }
        assert_eq!(reply, [IAC, DO, OPT_ECHO]);
    }

    #[test]
    fn resize_reports_only_once_naws_is_on_and_escapes_255() {
        let mut n = Negotiator::new(SIZE);
        assert_eq!(n.resize(TerminalSize { cols: 80, rows: 24 }), None);
        n.feed(&[IAC, DO, OPT_NAWS]);
        let r = n
            .resize(TerminalSize {
                cols: 255,
                rows: 24,
            })
            .unwrap();
        assert_eq!(r, [IAC, SB, OPT_NAWS, 0, IAC, IAC, 0, 24, IAC, SE]);
    }

    #[test]
    fn input_encoding() {
        let n = Negotiator::new(SIZE);
        assert_eq!(n.encode(b"ls\r"), b"ls\r\n");
        assert_eq!(n.encode(b"a\r\nb"), b"a\r\nb");
        assert_eq!(n.encode(&[1, IAC, 2]), [1, IAC, IAC, 2]);
    }

    #[test]
    fn cr_nul_padding_is_dropped() {
        let mut n = Negotiator::new(SIZE);
        assert_eq!(n.feed(b"a\r\0b\r\n").data, b"a\rb\r\n");
    }

    #[test]
    fn hostile_input_is_bounded() {
        let mut n = Negotiator::new(SIZE);
        let mut evil = vec![IAC, SB, OPT_TTYPE];
        evil.extend(std::iter::repeat_n(7u8, 100_000));
        n.feed(&evil);
        assert!(n.sb.len() <= MAX_SB);
        // Never terminated: following text stays inside the subnegotiation.
        assert!(n.feed(b"still sb").data.is_empty());
        assert_eq!(n.feed(&[IAC, SE, b'x']).data, b"x");
    }
}
