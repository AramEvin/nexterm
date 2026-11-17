//! FTP and FTPS (explicit TLS) file systems for the file manager.
//!
//! - FTP sends the password and data **unencrypted**; the UI marks such panes.
//! - FTPS upgrades the control connection with `AUTH TLS` and protects data
//!   connections (`PROT P`). Certificates signed by a trusted CA (Mozilla's
//!   root list) are accepted as usual. Self-signed / internal-CA certificates
//!   are handled like SSH host keys: the user sees the SHA-256 fingerprint and
//!   decides; a saved certificate that later **changes** blocks the connection.
//!
//! FTP allows one transfer per connection, so each pane keeps a "browse"
//! connection (listings, renames, ...) plus a small pool of connections for
//! transfers. Idle connections are checked with `NOOP` and reopened.
//!
//! FTP commands are lines of text: any path containing CR or LF is refused,
//! so a file name can't smuggle a second command to the server.

use std::io;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, ready};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD_NO_PAD;
use secrecy::{ExposeSecret, SecretString};
use sha2::{Digest, Sha256};
use suppaftp::list::{File as FtpFile, ListParser, PosixPexQuery};
use suppaftp::tokio::{
    AsyncRustlsConnector, AsyncRustlsFtpStream, AsyncRustlsStream, TransferStream,
};
use suppaftp::tokio_rustls::TlsConnector;
use suppaftp::tokio_rustls::rustls::{
    self, DigitallySignedStruct, SignatureScheme,
    client::WebPkiServerVerifier,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::CryptoProvider,
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use suppaftp::types::FileType;
use suppaftp::{FtpError, Mode, Status};
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};

use super::fs::Meta;
use super::paths::{base_name, remote_join, remote_parent};
use super::{EntryKind, FileEntry, Listing, MAX_LISTING, sort_entries};
use crate::error::{AppError, AppResult};
use crate::ssh::known_hosts::{self, HostKeyStatus};
use crate::ssh::prompt::{
    HostKeyDecision, PROMPT_TIMEOUT, PromptBroker, PromptResponse, PromptSink, SshPrompt,
};
use crate::storage::Database;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(30);
/// Idle time after which a connection is checked with NOOP before use.
const CHECK_AFTER: Duration = Duration::from_secs(20);
/// Idle transfer connections kept per pane.
const POOL_SIZE: usize = 3;

type Stream = AsyncRustlsFtpStream;
type BoxFuture<T> = Pin<Box<dyn Future<Output = T> + Send>>;

/// How to reach and log in to the server. Already validated.
pub struct FtpParams {
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: SecretString,
    /// Explicit TLS (FTPS). `false` = plain FTP.
    pub secure: bool,
}

/// What a connection needs to ask the user about certificates.
pub struct FtpTrust {
    pub broker: Arc<PromptBroker>,
    pub prompts: Arc<dyn PromptSink>,
    pub known_hosts: Option<Database>,
}

struct Conn {
    stream: Stream,
    last_used: Instant,
}

pub struct FtpFs {
    params: FtpParams,
    trust: FtpTrust,
    /// Certificate fingerprint the user accepted (or that is saved) for this pane.
    accepted_cert: Mutex<Option<String>>,
    browse: tokio::sync::Mutex<Option<Conn>>,
    pool: Mutex<Vec<Conn>>,
    /// MLSD/MLST support: `None` = unknown yet.
    mlsx: Mutex<Option<bool>>,
    /// "user@host:port", for logs and the UI.
    pub label: String,
}

impl std::fmt::Debug for FtpFs {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FtpFs")
            .field("label", &self.label)
            .field("secure", &self.params.secure)
            .finish()
    }
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Why a connection attempt failed.
enum ConnectError {
    /// The certificate isn't CA-trusted and isn't the accepted one.
    UntrustedCert(String),
    App(AppError),
}

impl From<AppError> for ConnectError {
    fn from(e: AppError) -> Self {
        Self::App(e)
    }
}

impl FtpFs {
    /// Connect and log in (asking about the certificate if needed).
    pub async fn open(params: FtpParams, trust: FtpTrust) -> AppResult<Arc<Self>> {
        let label = format!("{}@{}:{}", params.username, params.host, params.port);
        let fs = Arc::new(Self {
            params,
            trust,
            accepted_cert: Mutex::new(None),
            browse: tokio::sync::Mutex::new(None),
            pool: Mutex::new(Vec::new()),
            mlsx: Mutex::new(None),
            label,
        });
        let stream = fs.connect().await?;
        *fs.browse.lock().await = Some(Conn {
            stream,
            last_used: Instant::now(),
        });
        tracing::info!(
            host = %fs.params.host,
            port = fs.params.port,
            tls = fs.params.secure,
            "FTP session opened"
        );
        Ok(fs)
    }

    pub fn is_secure(&self) -> bool {
        self.params.secure
    }

    pub async fn close(&self) {
        if let Some(mut c) = self.browse.lock().await.take() {
            let _ = tokio::time::timeout(Duration::from_secs(3), c.stream.quit()).await;
        }
        lock(&self.pool).clear();
        tracing::info!(host = %self.params.host, port = self.params.port, "FTP session closed");
    }

    // ---- connecting ------------------------------------------------------

    async fn connect(&self) -> AppResult<Stream> {
        // Second round only after the user accepted the certificate.
        for _ in 0..2 {
            match self.try_connect().await {
                Ok(s) => return Ok(s),
                Err(ConnectError::App(e)) => return Err(e),
                Err(ConnectError::UntrustedCert(fp)) => self.decide_cert(fp).await?,
            }
        }
        Err(AppError::Connection(
            "The server's certificate could not be verified.".into(),
        ))
    }

    async fn try_connect(&self) -> Result<Stream, ConnectError> {
        let p = &self.params;
        let addr: SocketAddr = tokio::net::lookup_host((p.host.as_str(), p.port))
            .await
            .ok()
            .and_then(|mut a| a.next())
            .ok_or_else(|| AppError::Connection(format!("Could not resolve {}.", p.host)))?;
        let mut stream = Stream::connect_timeout(addr, CONNECT_TIMEOUT)
            .await
            .map_err(|e| connect_error(&e, &p.host))?;
        stream.set_mode(Mode::Passive);
        // Use the control connection's address if PASV answers with a private
        // one (servers behind NAT).
        stream.set_passive_nat_workaround(true);
        if p.secure {
            let seen = Arc::new(Mutex::new(None));
            let pinned = lock(&self.accepted_cert).clone();
            let connector = tls_connector(pinned, Arc::clone(&seen))?;
            stream = match timed(stream.into_secure(connector, &p.host)).await {
                Ok(s) => s,
                Err(FtpError::UnexpectedResponse(_)) => {
                    return Err(AppError::Connection(
                        "The server does not support FTPS (AUTH TLS). Use plain FTP only on \
                         trusted networks."
                            .into(),
                    )
                    .into());
                }
                Err(e) => {
                    if let Some(fp) = lock(&seen).take() {
                        return Err(ConnectError::UntrustedCert(fp));
                    }
                    tracing::warn!(host = %p.host, error = %e, "FTPS handshake failed");
                    return Err(AppError::Connection("The TLS handshake failed.".into()).into());
                }
            };
        }
        match timed(stream.login(p.username.as_str(), p.password.expose_secret())).await {
            Ok(()) => {}
            Err(FtpError::UnexpectedResponse(r)) if r.status == Status::NotLoggedIn => {
                tracing::info!(host = %p.host, user = %p.username, "FTP login failed");
                return Err(AppError::AuthFailed.into());
            }
            Err(e) => return Err(ftp_error(&e, "login").into()),
        }
        timed(stream.transfer_type(FileType::Binary))
            .await
            .map_err(|e| ftp_error(&e, "binary mode"))?;
        // Ask MLSD to include permissions and owners (ignored by servers that
        // don't support it).
        let _ = timed(stream.custom_command(
            "OPTS MLST type;size;modify;UNIX.mode;UNIX.uid;UNIX.gid;",
            &[Status::CommandOk],
        ))
        .await;
        Ok(stream)
    }

    /// The presented certificate isn't CA-signed: compare with what is saved,
    /// or ask the user.
    async fn decide_cert(&self, fingerprint: String) -> AppResult<()> {
        let (host, port) = (self.params.host.clone(), self.params.port);
        let status = match &self.trust.known_hosts {
            Some(db) => {
                let (h, fp) = (host.clone(), fingerprint.clone());
                db.run(move |c| known_hosts::check_cert(c, &h, port, &fp))
                    .await?
            }
            None => HostKeyStatus::Unknown {
                other_known_types: Vec::new(),
            },
        };
        match status {
            HostKeyStatus::Trusted { .. } => {}
            HostKeyStatus::Changed { stored } => {
                tracing::error!(%host, port, presented = %fingerprint, "TLS CERTIFICATE CHANGED");
                return Err(AppError::HostKeyChanged(format!(
                    "WARNING: the TLS certificate of {host}:{port} has CHANGED. Someone may be \
                     intercepting the connection, or the server's certificate was replaced. \
                     NexTerm blocked the connection.\n\nExpected: {stored}\nPresented: \
                     {fingerprint}\n\nVerify the new fingerprint with the server's administrator, \
                     then remove the saved certificate in Credentials → Known hosts."
                )));
            }
            HostKeyStatus::Unknown { .. } => {
                let response = self
                    .trust
                    .broker
                    .ask(
                        self.trust.prompts.as_ref(),
                        |prompt_id| SshPrompt::HostKey {
                            prompt_id,
                            kind: "tls",
                            host: host.clone(),
                            port,
                            key_type: "TLS certificate (not signed by a trusted CA)".into(),
                            fingerprint: fingerprint.clone(),
                            other_known_types: Vec::new(),
                        },
                        PROMPT_TIMEOUT,
                    )
                    .await?;
                let decision = match response {
                    PromptResponse::HostKey { decision } => decision,
                    _ => HostKeyDecision::Reject,
                };
                match decision {
                    HostKeyDecision::Reject => {
                        tracing::info!(%host, port, "TLS certificate rejected by user");
                        return Err(AppError::HostKeyRejected);
                    }
                    HostKeyDecision::AcceptOnce => {
                        tracing::info!(%host, port, fingerprint = %fingerprint, "TLS certificate accepted once");
                    }
                    HostKeyDecision::AcceptAndSave => {
                        if let Some(db) = &self.trust.known_hosts {
                            let (h, fp) = (host.clone(), fingerprint.clone());
                            db.run(move |c| known_hosts::save_cert(c, &h, port, &fp))
                                .await?;
                        }
                        tracing::info!(%host, port, fingerprint = %fingerprint, "TLS certificate saved");
                    }
                }
            }
        }
        *lock(&self.accepted_cert) = Some(fingerprint);
        Ok(())
    }

    /// The browse connection, reconnected if it went stale.
    async fn browse(&self) -> AppResult<tokio::sync::MutexGuard<'_, Option<Conn>>> {
        let mut guard = self.browse.lock().await;
        let alive = match guard.as_mut() {
            Some(c) if c.last_used.elapsed() < CHECK_AFTER => true,
            Some(c) => timed(c.stream.noop()).await.is_ok(),
            None => false,
        };
        if !alive {
            *guard = Some(Conn {
                stream: self.connect().await?,
                last_used: Instant::now(),
            });
        }
        Ok(guard)
    }

    /// A connection for one transfer (from the pool, or new).
    async fn take_conn(&self) -> AppResult<Conn> {
        loop {
            let pooled = lock(&self.pool).pop();
            let Some(mut c) = pooled else { break };
            if c.last_used.elapsed() < CHECK_AFTER || timed(c.stream.noop()).await.is_ok() {
                return Ok(c);
            }
        }
        Ok(Conn {
            stream: self.connect().await?,
            last_used: Instant::now(),
        })
    }

    fn put_back(&self, mut conn: Conn) {
        conn.last_used = Instant::now();
        let mut pool = lock(&self.pool);
        if pool.len() < POOL_SIZE {
            pool.push(conn);
        }
    }

    // ---- browsing ----------------------------------------------------------

    pub async fn home(&self) -> AppResult<String> {
        let mut g = self.browse().await?;
        let c = conn(&mut g)?;
        timed(c.stream.pwd())
            .await
            .map_err(|e| ftp_error(&e, "home folder"))
    }

    pub async fn list(&self, path: &str) -> AppResult<Listing> {
        let path = check_path(path)?;
        let mut g = self.browse().await?;
        let c = conn(&mut g)?;
        timed(c.stream.cwd(&path))
            .await
            .map_err(|e| ftp_error(&e, &path))?;
        let canonical = timed(c.stream.pwd())
            .await
            .map_err(|e| ftp_error(&e, &path))?;
        let files = self.list_current(&mut c.stream).await?;
        c.last_used = Instant::now();
        let mut entries = Vec::new();
        let mut truncated = false;
        for (f, mode_known) in files {
            if f.name() == "." || f.name() == ".." {
                continue;
            }
            if entries.len() >= MAX_LISTING {
                truncated = true;
                break;
            }
            let path = remote_join(&canonical, f.name());
            let mut e = entry(&f, path);
            if !mode_known {
                e.permissions = None;
            }
            entries.push(e);
        }
        sort_entries(&mut entries);
        Ok(Listing {
            parent: remote_parent(&canonical),
            path: canonical,
            entries,
            truncated,
        })
    }

    /// Entries of the current working directory: MLSD if supported, else LIST.
    /// Each entry comes with whether the server reported its permissions.
    async fn list_current(&self, s: &mut Stream) -> AppResult<Vec<(FtpFile, bool)>> {
        if *lock(&self.mlsx) != Some(false) {
            match timed(s.mlsd(None)).await {
                Ok(lines) => {
                    *lock(&self.mlsx) = Some(true);
                    return Ok(lines.iter().filter_map(|l| parse_mlsx_line(l)).collect());
                }
                Err(FtpError::UnexpectedResponse(r))
                    if matches!(r.status, Status::BadCommand | Status::NotImplemented)
                        || r.status as u32 == 501 =>
                {
                    *lock(&self.mlsx) = Some(false);
                }
                Err(e) => return Err(ftp_error(&e, "folder listing")),
            }
        }
        let lines = timed(s.list(None))
            .await
            .map_err(|e| ftp_error(&e, "folder listing"))?;
        Ok(lines
            .iter()
            .filter(|l| !l.starts_with("total "))
            .filter_map(|l| {
                ListParser::parse_posix(l)
                    .map(|f| (f, true))
                    .or_else(|_| ListParser::parse_dos(l).map(|f| (f, false)))
                    .ok()
            })
            .collect())
    }

    /// Metadata for one path. FTP has no stat: MLST if available, else
    /// "can I CWD into it" (folder) / SIZE + MDTM (file).
    pub async fn stat(&self, path: &str) -> AppResult<Meta> {
        let path = check_path(path)?;
        let mut g = self.browse().await?;
        let c = conn(&mut g)?;
        c.last_used = Instant::now();
        if *lock(&self.mlsx) != Some(false) {
            match timed(c.stream.mlst(Some(&path))).await {
                Ok(line) => {
                    if let Ok(f) = ListParser::parse_mlst(&line) {
                        return Ok(meta(&f));
                    }
                }
                Err(FtpError::UnexpectedResponse(r))
                    if matches!(r.status, Status::BadCommand | Status::NotImplemented) =>
                {
                    *lock(&self.mlsx) = Some(false);
                }
                Err(FtpError::UnexpectedResponse(_)) => {
                    return Err(not_found(&path));
                }
                Err(e) => return Err(ftp_error(&e, &path)),
            }
        }
        if timed(c.stream.cwd(&path)).await.is_ok() {
            return Ok(Meta {
                is_dir: true,
                is_symlink: false,
                size: 0,
                mtime: None,
            });
        }
        match timed(c.stream.size(&path)).await {
            Ok(size) => {
                let mtime = timed(c.stream.mdtm(&path))
                    .await
                    .ok()
                    .and_then(|t| u32::try_from(t.and_utc().timestamp()).ok());
                Ok(Meta {
                    is_dir: false,
                    is_symlink: false,
                    size: size as u64,
                    mtime,
                })
            }
            Err(_) => Err(not_found(&path)),
        }
    }

    pub async fn exists(&self, path: &str) -> bool {
        self.stat(path).await.is_ok()
    }

    pub async fn read_dir(&self, path: &str) -> AppResult<Vec<(String, Meta)>> {
        let listing = self.list(path).await?;
        Ok(listing
            .entries
            .into_iter()
            .map(|e| {
                (
                    e.name,
                    Meta {
                        is_dir: e.kind == EntryKind::Dir,
                        is_symlink: e.kind == EntryKind::Symlink,
                        size: e.size,
                        mtime: e.modified.and_then(|m| u32::try_from(m).ok()),
                    },
                )
            })
            .collect())
    }

    pub async fn mkdir(&self, path: &str) -> AppResult<()> {
        let path = check_path(path)?;
        let mut g = self.browse().await?;
        let c = conn(&mut g)?;
        c.last_used = Instant::now();
        timed(c.stream.mkdir(&path))
            .await
            .map_err(|e| ftp_error(&e, &path))
    }

    pub async fn rename(&self, from: &str, to: &str) -> AppResult<()> {
        let (from, to) = (check_path(from)?, check_path(to)?);
        if self.exists(&to).await {
            return Err(AppError::FileOp(format!(
                "\"{}\" already exists.",
                base_name(&to)
            )));
        }
        let mut g = self.browse().await?;
        let c = conn(&mut g)?;
        c.last_used = Instant::now();
        timed(c.stream.rename(&from, &to))
            .await
            .map_err(|e| ftp_error(&e, &from))
    }

    /// Delete a file or a folder tree. Returns the number of items deleted.
    pub async fn remove(&self, path: &str) -> AppResult<u64> {
        let path = check_path(path)?;
        let m = self.stat(&path).await?;
        let mut count = 0;
        if m.is_dir {
            Box::pin(self.remove_tree(path, 0, &mut count)).await?;
        } else {
            self.remove_file(&path).await?;
            count = 1;
        }
        Ok(count)
    }

    async fn remove_tree(&self, dir: String, depth: usize, count: &mut u64) -> AppResult<()> {
        if depth > super::fs::MAX_DEPTH {
            return Err(AppError::FileOp("Folder nesting is too deep.".into()));
        }
        for (name, m) in self.read_dir(&dir).await? {
            let child = remote_join(&dir, &name);
            if m.is_dir {
                Box::pin(self.remove_tree(child, depth + 1, count)).await?;
            } else {
                self.remove_file(&child).await?;
                *count += 1;
            }
        }
        let mut g = self.browse().await?;
        let c = conn(&mut g)?;
        // Leave the folder before removing it (some servers refuse otherwise).
        if let Some(parent) = remote_parent(&dir) {
            let _ = timed(c.stream.cwd(&parent)).await;
        }
        timed(c.stream.rmdir(&dir))
            .await
            .map_err(|e| ftp_error(&e, &dir))?;
        *count += 1;
        Ok(())
    }

    pub async fn remove_file(&self, path: &str) -> AppResult<()> {
        let path = check_path(path)?;
        let mut g = self.browse().await?;
        let c = conn(&mut g)?;
        c.last_used = Instant::now();
        timed(c.stream.rm(&path))
            .await
            .map_err(|e| ftp_error(&e, &path))
    }

    /// `SITE CHMOD` (supported by most Unix FTP servers).
    pub async fn chmod(&self, path: &str, mode: u32) -> AppResult<()> {
        let path = check_path(path)?;
        let mut g = self.browse().await?;
        let c = conn(&mut g)?;
        c.last_used = Instant::now();
        timed(c.stream.site(format!("CHMOD {:o} {path}", mode & 0o7777)))
            .await
            .map(|_| ())
            .map_err(|e| ftp_error(&e, &path))
    }

    /// Best-effort `MFMT` (keeps the source's modification time).
    pub async fn set_mtime(&self, path: &str, mtime: u32) {
        let Ok(path) = check_path(path) else { return };
        let Ok(mut g) = self.browse().await else {
            return;
        };
        let Ok(c) = conn(&mut g) else { return };
        let _ = timed(c.stream.custom_command(
            format!("MFMT {} {path}", mfmt_time(u64::from(mtime))),
            &[Status::File],
        ))
        .await;
    }

    // ---- transfers -----------------------------------------------------------

    pub async fn open_read(self: &Arc<Self>, path: &str) -> AppResult<FtpReader> {
        let path = check_path(path)?;
        let mut c = self.take_conn().await?;
        let data = timed(c.stream.retr_as_stream(&path))
            .await
            .map_err(|e| ftp_error(&e, &path))?;
        Ok(FtpReader {
            fs: Arc::clone(self),
            conn: Some(c),
            data: Some(data),
            finishing: None,
        })
    }

    pub async fn open_write(self: &Arc<Self>, path: &str) -> AppResult<FtpWriter> {
        let path = check_path(path)?;
        let mut c = self.take_conn().await?;
        let data = timed(c.stream.put_with_stream(&path))
            .await
            .map_err(|e| ftp_error(&e, &path))?;
        Ok(FtpWriter {
            fs: Arc::clone(self),
            conn: Some(c),
            data: Some(data),
            finishing: None,
        })
    }
}

/// Download stream: at the end it reads the server's "226 Transfer complete"
/// and returns the connection to the pool.
pub struct FtpReader {
    fs: Arc<FtpFs>,
    conn: Option<Conn>,
    data: Option<TransferStream<AsyncRustlsStream>>,
    finishing: Option<BoxFuture<Result<(), FtpError>>>,
}

impl AsyncRead for FtpReader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        loop {
            if let Some(fut) = this.finishing.as_mut() {
                let result = ready!(fut.as_mut().poll(cx));
                this.finishing = None;
                return match result {
                    Ok(()) => {
                        if let Some(c) = this.conn.take() {
                            this.fs.put_back(c);
                        }
                        Poll::Ready(Ok(()))
                    }
                    Err(e) => Poll::Ready(Err(io::Error::other(e.to_string()))),
                };
            }
            let Some(data) = this.data.as_mut() else {
                return Poll::Ready(Ok(())); // finished: EOF
            };
            let before = buf.filled().len();
            ready!(Pin::new(data).poll_read(cx, buf))?;
            if buf.filled().len() > before {
                return Poll::Ready(Ok(()));
            }
            // End of data: complete the transfer, then report EOF.
            if let Some(d) = this.data.take() {
                this.finishing = Some(Box::pin(d.finish()));
            }
        }
    }
}

/// Upload stream: `shutdown` closes the data connection and checks the
/// server's reply, so a refused upload (quota, permissions) is an error.
pub struct FtpWriter {
    fs: Arc<FtpFs>,
    conn: Option<Conn>,
    data: Option<TransferStream<AsyncRustlsStream>>,
    finishing: Option<BoxFuture<Result<(), FtpError>>>,
}

fn closed() -> io::Error {
    io::Error::new(io::ErrorKind::BrokenPipe, "upload already finished")
}

impl AsyncWrite for FtpWriter {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut().data.as_mut() {
            Some(d) => Pin::new(d).poll_write(cx, buf),
            None => Poll::Ready(Err(closed())),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut().data.as_mut() {
            Some(d) => Pin::new(d).poll_flush(cx),
            None => Poll::Ready(Ok(())),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.finishing.is_none() {
            match this.data.take() {
                Some(d) => this.finishing = Some(Box::pin(d.finish())),
                None => return Poll::Ready(Ok(())),
            }
        }
        let Some(fut) = this.finishing.as_mut() else {
            return Poll::Ready(Ok(()));
        };
        let result = ready!(fut.as_mut().poll(cx));
        this.finishing = None;
        match result {
            Ok(()) => {
                if let Some(c) = this.conn.take() {
                    this.fs.put_back(c);
                }
                Poll::Ready(Ok(()))
            }
            Err(e) => Poll::Ready(Err(io::Error::other(match &e {
                FtpError::UnexpectedResponse(r) => {
                    format!("the server rejected the upload: {}", response_text(r))
                }
                other => other.to_string(),
            }))),
        }
    }
}

// ---- TLS -------------------------------------------------------------------

/// SHA-256 of the certificate (DER), formatted like SSH fingerprints.
pub fn cert_fingerprint(der: &[u8]) -> String {
    format!("SHA256:{}", STANDARD_NO_PAD.encode(Sha256::digest(der)))
}

/// Accept CA-signed certificates normally; otherwise accept only the pinned
/// fingerprint, and record what was presented so the user can be asked.
#[derive(Debug)]
struct PinningVerifier {
    inner: Arc<WebPkiServerVerifier>,
    provider: Arc<CryptoProvider>,
    pinned: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
}

impl ServerCertVerifier for PinningVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        match self.inner.verify_server_cert(
            end_entity,
            intermediates,
            server_name,
            ocsp_response,
            now,
        ) {
            Ok(v) => Ok(v),
            Err(e) => {
                let fp = cert_fingerprint(end_entity);
                if self.pinned.as_deref() == Some(fp.as_str()) {
                    Ok(ServerCertVerified::assertion())
                } else {
                    *lock(&self.seen) = Some(fp);
                    Err(e)
                }
            }
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

fn tls_connector(
    pinned: Option<String>,
    seen: Arc<Mutex<Option<String>>>,
) -> AppResult<AsyncRustlsConnector> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(rustls_webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), Arc::clone(&provider))
        .build()
        .map_err(|e| AppError::Internal(format!("TLS verifier: {e}")))?;
    let verifier = Arc::new(PinningVerifier {
        inner,
        provider: Arc::clone(&provider),
        pinned,
        seen,
    });
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| AppError::Internal(format!("TLS config: {e}")))?
        .dangerous() // custom verifier: CA check first, then the pinned fingerprint
        .with_custom_certificate_verifier(verifier)
        .with_no_client_auth();
    Ok(AsyncRustlsConnector::from(TlsConnector::from(Arc::new(
        config,
    ))))
}

// ---- helpers -----------------------------------------------------------------

async fn timed<T>(fut: impl Future<Output = Result<T, FtpError>>) -> Result<T, FtpError> {
    tokio::time::timeout(COMMAND_TIMEOUT, fut)
        .await
        .unwrap_or_else(|_| {
            Err(FtpError::ConnectionError(io::Error::new(
                io::ErrorKind::TimedOut,
                "timed out",
            )))
        })
}

fn conn<'a>(guard: &'a mut tokio::sync::MutexGuard<'_, Option<Conn>>) -> AppResult<&'a mut Conn> {
    guard
        .as_mut()
        .ok_or_else(|| AppError::Connection("The FTP connection is closed.".into()))
}

/// Refuse paths that could inject FTP commands or are malformed.
pub fn check_path(path: &str) -> AppResult<String> {
    if path.contains(['\r', '\n']) {
        return Err(AppError::Validation(
            "FTP paths can't contain line breaks.".into(),
        ));
    }
    super::paths::remote_path(path)
}

fn not_found(path: &str) -> AppError {
    AppError::FileOp(format!("\"{}\" does not exist.", base_name(path)))
}

fn response_text(r: &suppaftp::types::Response) -> String {
    r.as_string()
        .unwrap_or_default()
        .trim()
        .chars()
        .take(200)
        .collect()
}

fn connect_error(e: &FtpError, host: &str) -> AppError {
    tracing::warn!(host, error = %e, "FTP connect failed");
    AppError::Connection(format!("Could not connect to {host}."))
}

/// Short, user-facing message for an FTP failure about `what`.
pub fn ftp_error(e: &FtpError, what: &str) -> AppError {
    let name = base_name(what);
    tracing::debug!(error = %e, "FTP request failed");
    match e {
        FtpError::UnexpectedResponse(r) => {
            let text = response_text(r);
            match r.status as u32 {
                530 => AppError::AuthFailed,
                421 => AppError::Connection("The FTP server closed the connection.".into()),
                550 => AppError::FileOp(format!(
                    "\"{name}\": not found or permission denied ({text})."
                )),
                553 => AppError::FileOp(format!("The name \"{name}\" is not allowed here.")),
                452 | 552 => AppError::FileOp("The server is out of space.".into()),
                500 | 502 | 504 => {
                    AppError::FileOp("The server does not support this operation.".into())
                }
                _ => AppError::FileOp(format!("The server refused: {text}")),
            }
        }
        FtpError::ConnectionError(io) if io.kind() == io::ErrorKind::TimedOut => {
            AppError::Connection("The FTP server did not respond in time.".into())
        }
        FtpError::ConnectionError(_) => AppError::Connection("The FTP connection was lost.".into()),
        FtpError::SecureError(_) => AppError::Connection("The TLS connection failed.".into()),
        _ => AppError::Connection("The FTP server sent an unexpected response.".into()),
    }
}

fn entry(f: &FtpFile, path: String) -> FileEntry {
    let kind = if f.is_directory() {
        EntryKind::Dir
    } else if f.is_symlink() {
        EntryKind::Symlink
    } else if f.is_file() {
        EntryKind::File
    } else {
        EntryKind::Other
    };
    FileEntry {
        hidden: f.name().starts_with('.'),
        name: f.name().to_owned(),
        path,
        kind,
        link_to_dir: false,
        size: f.size() as u64,
        modified: unix_secs(f.modified()),
        permissions: Some(mode_bits(f)),
        owner: f.uid().map(|u| u.to_string()),
        group: f.gid().map(|g| g.to_string()),
    }
}

/// Parse one MLSD line. Some servers write the mode as `unix.mode=0o644`
/// (Python's octal syntax); normalise that. Returns whether a mode was given.
fn parse_mlsx_line(line: &str) -> Option<(FtpFile, bool)> {
    let lower = line.to_ascii_lowercase();
    let line = match lower.find("unix.mode=0o") {
        Some(i) => format!(
            "{}{}",
            &line[..i + "unix.mode=0".len()],
            &line[i + "unix.mode=0o".len()..]
        ),
        None => line.to_owned(),
    };
    let known = lower.contains("unix.mode=");
    ListParser::parse_mlsd(&line).ok().map(|f| (f, known))
}

fn meta(f: &FtpFile) -> Meta {
    Meta {
        is_dir: f.is_directory(),
        is_symlink: f.is_symlink(),
        size: f.size() as u64,
        mtime: unix_secs(f.modified()).and_then(|m| u32::try_from(m).ok()),
    }
}

fn unix_secs(t: SystemTime) -> Option<i64> {
    t.duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_secs()).ok())
        .filter(|s| *s > 0)
}

fn mode_bits(f: &FtpFile) -> u32 {
    let mut mode = 0;
    for (who, shift) in [
        (PosixPexQuery::Owner, 6),
        (PosixPexQuery::Group, 3),
        (PosixPexQuery::Others, 0),
    ] {
        if f.can_read(who) {
            mode |= 4 << shift;
        }
        if f.can_write(who) {
            mode |= 2 << shift;
        }
        if f.can_execute(who) {
            mode |= 1 << shift;
        }
    }
    mode
}

/// Unix seconds -> `YYYYMMDDHHMMSS` (UTC), the MFMT/MDTM time format.
pub fn mfmt_time(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    // Civil-from-days (Howard Hinnant), valid for all dates after 1970.
    let z = days as i64 + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}{month:02}{day:02}{:02}{:02}{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crlf_paths_are_refused() {
        assert!(check_path("/a\r\nDELE /etc/passwd").is_err());
        assert!(check_path("/a\nb").is_err());
        assert!(check_path("/ok path/file.txt").is_ok());
    }

    #[test]
    fn mfmt_time_format() {
        assert_eq!(mfmt_time(0), "19700101000000");
        assert_eq!(mfmt_time(1_500_000_000), "20170714024000");
        assert_eq!(mfmt_time(951_782_400), "20000229000000"); // leap day
    }

    #[test]
    fn fingerprint_format() {
        let fp = cert_fingerprint(b"abc");
        assert_eq!(fp, "SHA256:ungWv48Bz+pBQUDeXa4iI7ADYaOWF3qctBD/YfIAFa0");
    }

    #[test]
    fn listing_lines_parse() {
        let f = ListParser::parse_posix("-rw-r--r-- 1 1000 1000 1234 Nov 16 10:00 a.txt").unwrap();
        let e = entry(&f, "/a.txt".into());
        assert_eq!(e.kind, EntryKind::File);
        assert_eq!(e.size, 1234);
        assert_eq!(e.permissions, Some(0o644));
        let d =
            ListParser::parse_mlsd("type=dir;modify=20201019151930;UNIX.mode=0755; pub").unwrap();
        assert_eq!(entry(&d, "/pub".into()).kind, EntryKind::Dir);
        let (f, known) = parse_mlsx_line("perm=r;type=file;unix.mode=0o600; x").unwrap();
        assert!(known);
        assert_eq!(mode_bits(&f), 0o600);
        let (_, known) = parse_mlsx_line("type=file;size=1; y").unwrap();
        assert!(!known);
    }
}
