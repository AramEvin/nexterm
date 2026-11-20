//! PuTTY saved sessions: a `.reg` export (any OS) or, on Windows, the live
//! registry key `HKCU\Software\SimonTatham\PuTTY\Sessions`.

use super::{ImportPreview, ImportSource, ImportedHop, ImportedSession};
use crate::session::options::{FlowControl, Parity, SerialOptions, SessionOptions};
use crate::session::protocol::ProtocolKind;

const SESSIONS_KEY: &str = r"Software\SimonTatham\PuTTY\Sessions";
/// PuTTY's proxy type "SSH to proxy and use port forwarding" (0.77+).
const PROXY_SSH: u32 = 6;

/// Values of one PuTTY session we care about.
#[derive(Debug, Default)]
pub struct RawSession {
    pub name: String,
    pub host: String,
    pub port: Option<u32>,
    pub username: String,
    pub protocol: String,
    pub key_file: String,
    pub proxy_method: Option<u32>,
    pub proxy_host: String,
    pub proxy_port: Option<u32>,
    pub proxy_user: String,
    pub serial_line: String,
    pub serial_speed: Option<u32>,
    pub serial_data_bits: Option<u32>,
    /// Stop bits × 2 (PuTTY stores 2 for 1, 4 for 2).
    pub serial_stop_halfbits: Option<u32>,
    /// 0 none, 1 odd, 2 even, 3 mark, 4 space.
    pub serial_parity: Option<u32>,
    /// 0 none, 1 XON/XOFF, 2 RTS/CTS, 3 DSR/DTR.
    pub serial_flow: Option<u32>,
}

impl RawSession {
    fn set_string(&mut self, key: &str, value: String) {
        match key {
            "HostName" => self.host = value,
            "UserName" => self.username = value,
            "Protocol" => self.protocol = value,
            "PublicKeyFile" => self.key_file = value,
            "ProxyHost" => self.proxy_host = value,
            "ProxyUsername" => self.proxy_user = value,
            "SerialLine" => self.serial_line = value,
            _ => {}
        }
    }

    fn set_dword(&mut self, key: &str, value: u32) {
        match key {
            "PortNumber" => self.port = Some(value),
            "ProxyMethod" => self.proxy_method = Some(value),
            "ProxyPort" => self.proxy_port = Some(value),
            "SerialSpeed" => self.serial_speed = Some(value),
            "SerialDataBits" => self.serial_data_bits = Some(value),
            "SerialStopHalfbits" => self.serial_stop_halfbits = Some(value),
            "SerialParity" => self.serial_parity = Some(value),
            "SerialFlowControl" => self.serial_flow = Some(value),
            _ => {}
        }
    }
}

/// PuTTY stores session names URL-encoded (`My%20Server`).
fn decode_name(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(v) = u8::from_str_radix(raw.get(i + 1..i + 3).unwrap_or(""), 16)
        {
            out.push(v);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `.reg` files from regedit are UTF-16 LE with a BOM; REGEDIT4 files are ANSI.
fn decode_reg(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        let units: Vec<u16> = rest
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| u16::from_le_bytes(*c))
            .collect();
        return String::from_utf16_lossy(&units);
    }
    super::mobaxterm::decode_cp1252(bytes)
}

/// `"C:\\keys\\a.ppk"` → `C:\keys\a.ppk`.
fn reg_string(raw: &str) -> Option<String> {
    let inner = raw.strip_prefix('"')?.strip_suffix('"')?;
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(n) = chars.next() {
                out.push(n);
            }
        } else {
            out.push(c);
        }
    }
    Some(out)
}

/// Parse a regedit export.
pub fn parse_reg(bytes: &[u8]) -> ImportPreview {
    let text = decode_reg(bytes);
    let mut sessions: Vec<RawSession> = Vec::new();
    let mut current: Option<RawSession> = None;
    let prefix = format!(r"\{SESSIONS_KEY}\").to_lowercase();

    for raw in text.lines() {
        let line = raw.trim();
        if let Some(path) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            if let Some(s) = current.take() {
                sessions.push(s);
            }
            let lower = path.to_lowercase();
            if let Some(pos) = lower.find(&prefix) {
                let name = &path[pos + prefix.len()..];
                if !name.is_empty() && !name.contains('\\') {
                    current = Some(RawSession {
                        name: decode_name(name),
                        ..RawSession::default()
                    });
                }
            }
            continue;
        }
        let Some(session) = current.as_mut() else {
            continue;
        };
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let Some(key) = reg_string(key.trim()) else {
            continue;
        };
        let value = value.trim();
        if let Some(hex) = value.strip_prefix("dword:") {
            if let Ok(v) = u32::from_str_radix(hex.trim(), 16) {
                session.set_dword(&key, v);
            }
        } else if let Some(s) = reg_string(value) {
            session.set_string(&key, s);
        }
    }
    if let Some(s) = current.take() {
        sessions.push(s);
    }
    convert(sessions)
}

/// Turn raw PuTTY values into a preview.
pub fn convert(raw: Vec<RawSession>) -> ImportPreview {
    let mut out = ImportPreview::new(ImportSource::Putty);
    for r in raw {
        if r.name == "Default Settings" {
            continue;
        }
        let protocol = if r.protocol.is_empty() {
            "ssh"
        } else {
            r.protocol.as_str()
        };
        match protocol {
            "ssh" | "telnet" => {}
            "serial" => {
                serial_session(&mut out, &r);
                continue;
            }
            other => {
                out.skip(&r.name, format!("{other} sessions aren't supported."));
                continue;
            }
        }
        let telnet = protocol == "telnet";
        if r.host.trim().is_empty() {
            out.skip(&r.name, "No host name.");
            continue;
        }
        // PuTTY accepts "user@host" in the host field.
        let (mut user, host) = match r.host.trim().rsplit_once('@') {
            Some((u, h)) => (u.to_owned(), h.to_owned()),
            None => (String::new(), r.host.trim().to_owned()),
        };
        if !r.username.trim().is_empty() {
            user = r.username.trim().to_owned();
        }
        let jump = if r.proxy_method == Some(PROXY_SSH) && !r.proxy_host.trim().is_empty() {
            vec![ImportedHop {
                host: r.proxy_host.trim().to_owned(),
                port: r
                    .proxy_port
                    .and_then(|p| u16::try_from(p).ok())
                    .unwrap_or(22),
                username: r.proxy_user.trim().to_owned(),
            }]
        } else {
            Vec::new()
        };
        let key = r.key_file.trim();
        if telnet {
            out.push(ImportedSession {
                name: r.name.clone(),
                folder: Vec::new(),
                protocol: ProtocolKind::Telnet,
                host,
                port: r.port.map_or(23, |p| u16::try_from(p).unwrap_or(0)),
                username: String::new(),
                auth_method: "password".into(),
                key_path: None,
                jump: Vec::new(),
                options: SessionOptions::default(),
            });
            continue;
        }
        out.push(ImportedSession {
            name: r.name.clone(),
            folder: Vec::new(),
            protocol: ProtocolKind::Ssh,
            host,
            port: r.port.map_or(22, |p| u16::try_from(p).unwrap_or(0)),
            username: user,
            auth_method: if key.is_empty() {
                "password"
            } else {
                "publickey"
            }
            .into(),
            key_path: (!key.is_empty()).then(|| key.to_owned()),
            jump,
            options: SessionOptions::default(),
        });
    }
    out
}

fn serial_session(out: &mut ImportPreview, r: &RawSession) {
    let parity = match r.serial_parity.unwrap_or(0) {
        0 => Parity::None,
        1 => Parity::Odd,
        2 => Parity::Even,
        _ => return out.skip(&r.name, "Mark/space parity isn't supported."),
    };
    let flow_control = match r.serial_flow.unwrap_or(0) {
        0 => FlowControl::None,
        1 => FlowControl::Software,
        2 => FlowControl::Hardware,
        _ => return out.skip(&r.name, "DSR/DTR flow control isn't supported."),
    };
    let serial = SerialOptions {
        port: r.serial_line.trim().to_owned(),
        baud_rate: r.serial_speed.unwrap_or(9600),
        data_bits: r
            .serial_data_bits
            .and_then(|b| u8::try_from(b).ok())
            .unwrap_or(8),
        parity,
        stop_bits: if r.serial_stop_halfbits == Some(4) {
            2
        } else {
            1
        },
        flow_control,
    };
    out.push(ImportedSession {
        name: r.name.clone(),
        folder: Vec::new(),
        protocol: ProtocolKind::Serial,
        host: serial.port.clone(),
        port: 1,
        username: String::new(),
        auth_method: "password".into(),
        key_path: None,
        jump: Vec::new(),
        options: SessionOptions {
            serial: Some(serial),
            local: None,
        },
    });
}

/// Read PuTTY's sessions straight from this PC's registry.
#[cfg(windows)]
pub fn read_registry() -> crate::error::AppResult<ImportPreview> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let root = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(SESSIONS_KEY) {
        Ok(k) => k,
        Err(_) => return Ok(convert(Vec::new())), // PuTTY never saved a session
    };
    let mut raw = Vec::new();
    for name in root.enum_keys().flatten().take(super::MAX_SESSIONS) {
        let Ok(key) = root.open_subkey(&name) else {
            continue;
        };
        let mut s = RawSession {
            name: decode_name(&name),
            ..RawSession::default()
        };
        for k in [
            "HostName",
            "UserName",
            "Protocol",
            "PublicKeyFile",
            "ProxyHost",
            "ProxyUsername",
            "SerialLine",
        ] {
            if let Ok(v) = key.get_value::<String, _>(k) {
                s.set_string(k, v);
            }
        }
        for k in [
            "PortNumber",
            "ProxyMethod",
            "ProxyPort",
            "SerialSpeed",
            "SerialDataBits",
            "SerialStopHalfbits",
            "SerialParity",
            "SerialFlowControl",
        ] {
            if let Ok(v) = key.get_value::<u32, _>(k) {
                s.set_dword(k, v);
            }
        }
        raw.push(s);
    }
    Ok(convert(raw))
}

#[cfg(not(windows))]
pub fn read_registry() -> crate::error::AppResult<ImportPreview> {
    Err(crate::error::AppError::Validation(
        "Reading PuTTY's sessions from the registry only works on Windows. Import a .reg export instead."
            .into(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const REG: &str = r#"Windows Registry Editor Version 5.00

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions]

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Default%20Settings]
"HostName"=""

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\Prod%20Web]
"HostName"="web-01.example.com"
"PortNumber"=dword:00000016
"UserName"="deploy"
"Protocol"="ssh"
"PublicKeyFile"="C:\\Users\\me\\keys\\web.ppk"
"ProxyMethod"=dword:00000000

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\db%20via%20bastion]
"HostName"="admin@10.1.2.3"
"PortNumber"=dword:000008ae
"Protocol"="ssh"
"ProxyMethod"=dword:00000006
"ProxyHost"="bastion.example.com"
"ProxyPort"=dword:00000016
"ProxyUsername"="jump"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\switch]
"HostName"="10.0.0.2"
"Protocol"="telnet"

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\console]
"Protocol"="serial"
"SerialLine"="COM4"
"SerialSpeed"=dword:0001c200
"SerialDataBits"=dword:00000008
"SerialStopHalfbits"=dword:00000002
"SerialParity"=dword:00000000
"SerialFlowControl"=dword:00000001

[HKEY_CURRENT_USER\Software\SimonTatham\PuTTY\Sessions\old]
"HostName"="10.0.0.3"
"Protocol"="rlogin"
"#;

    fn utf16(s: &str) -> Vec<u8> {
        let mut out = vec![0xFF, 0xFE];
        for u in s.encode_utf16() {
            out.extend_from_slice(&u.to_le_bytes());
        }
        out
    }

    #[test]
    fn parses_a_regedit_export() {
        let p = parse_reg(&utf16(REG));
        let names: Vec<&str> = p.sessions.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["Prod Web", "db via bastion", "switch", "console"]);

        let web = &p.sessions[0];
        assert_eq!(
            (web.host.as_str(), web.port, web.username.as_str()),
            ("web-01.example.com", 22, "deploy")
        );
        assert_eq!(web.auth_method, "publickey");
        assert_eq!(web.key_path.as_deref(), Some(r"C:\Users\me\keys\web.ppk"));
        assert!(web.jump.is_empty());

        let db = &p.sessions[1];
        assert_eq!(
            (db.host.as_str(), db.port, db.username.as_str()),
            ("10.1.2.3", 2222, "admin")
        );
        assert_eq!(db.jump[0].host, "bastion.example.com");
        assert_eq!(db.jump[0].username, "jump");

        let switch = &p.sessions[2];
        assert_eq!((switch.protocol, switch.port), (ProtocolKind::Telnet, 23));
        let console = &p.sessions[3];
        assert_eq!(console.protocol, ProtocolKind::Serial);
        let serial = console.options.serial.as_ref().unwrap();
        assert_eq!(serial.summary(), "COM4 · 115200 8N1");
        assert_eq!(serial.flow_control, FlowControl::Software);

        assert_eq!(p.skipped.len(), 1);
        assert_eq!(p.skipped[0].reason, "rlogin sessions aren't supported.");
    }

    #[test]
    fn ansi_exports_and_odd_names_work() {
        let p = parse_reg(REG.replace("Prod%20Web", "a%2Fb%zz").as_bytes());
        assert_eq!(p.sessions[0].name, "a/b%zz");
    }

    #[test]
    fn garbage_never_panics() {
        let p = parse_reg(b"\xFF\xFE\x00");
        assert!(p.sessions.is_empty());
        let p = parse_reg(b"[HKEY_CURRENT_USER\\Software\\SimonTatham\\PuTTY\\Sessions\\x]\n\"PortNumber\"=dword:zz\n\"HostName\"=\"unterminated\n");
        assert!(p.sessions.is_empty());
    }
}
