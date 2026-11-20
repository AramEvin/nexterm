//! MobaXterm `.mxtsessions` / `MobaXterm.ini` bookmarks.
//!
//! INI file in Windows-1252 with CRLF. Sessions live in `[Bookmarks]` and
//! `[Bookmarks_N]` sections; `SubRep=` is the folder path (`A\B`). A session
//! line is `Name= #icon#type%host%port%user%…#display…#…` (format notes:
//! <https://gist.github.com/Ruzgfpegk/ab597838e4abbe8de30d7224afd062ea>).

use super::{ImportPreview, ImportSource, ImportedHop, ImportedSession};
use crate::session::protocol::ProtocolKind;

/// MobaXterm session type ids (second number of `#icon#type`).
fn type_name(id: &str) -> &'static str {
    match id {
        "0" => "SSH",
        "1" => "Telnet",
        "2" => "Rsh",
        "3" => "Xdmcp",
        "4" => "RDP",
        "5" => "VNC",
        "6" => "FTP",
        "7" => "SFTP",
        "8" => "Serial",
        "9" => "File",
        "10" => "Shell",
        "11" => "Browser",
        "12" => "Mosh",
        _ => "this",
    }
}

/// Windows-1252 → `String` (0x80–0x9F differ from Latin-1).
pub fn decode_cp1252(bytes: &[u8]) -> String {
    const HIGH: [char; 32] = [
        '€', '\u{81}', '‚', 'ƒ', '„', '…', '†', '‡', 'ˆ', '‰', 'Š', '‹', 'Œ', '\u{8d}', 'Ž',
        '\u{8f}', '\u{90}', '‘', '’', '“', '”', '•', '–', '—', '˜', '™', 'š', '›', 'œ', '\u{9d}',
        'ž', 'Ÿ',
    ];
    if let Ok(s) = std::str::from_utf8(bytes) {
        return s.trim_start_matches('\u{feff}').to_owned(); // newer versions may use UTF-8
    }
    bytes
        .iter()
        .map(|&b| match b {
            0x80..=0x9F => HIGH[usize::from(b - 0x80)],
            _ => char::from(b),
        })
        .collect()
}

/// MobaXterm's escapes for characters it can't store.
fn unescape(s: &str) -> String {
    s.replace("__PTVIRG__", ";")
        .replace("__DBLQUO__", "\"")
        .replace("__DIEZE__", "#")
        .replace("__PERCENT__", "%")
        .replace("_CurrentDrive_", "C")
}

fn port(raw: Option<&&str>, default: u16) -> u16 {
    match raw.map(|s| s.trim()) {
        None | Some("") => default,
        Some(p) => p.parse().unwrap_or(0),
    }
}

pub fn parse(bytes: &[u8]) -> ImportPreview {
    let text = decode_cp1252(bytes);
    let mut out = ImportPreview::new(ImportSource::Mobaxterm);
    let mut in_bookmarks = false;
    let mut folder: Vec<String> = Vec::new();

    for raw in text.lines() {
        let line = raw.trim_end_matches('\r');
        let trimmed = line.trim();
        if trimmed.is_empty() || trimmed.starts_with(';') {
            continue;
        }
        if let Some(section) = trimmed.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            in_bookmarks = section == "Bookmarks" || section.starts_with("Bookmarks_");
            folder.clear();
            continue;
        }
        if !in_bookmarks {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key {
            "SubRep" => {
                folder = value
                    .split('\\')
                    .map(|s| unescape(s.trim()))
                    .filter(|s| !s.is_empty())
                    .collect();
            }
            "ImgNum" => {}
            name => session_line(&mut out, &unescape(name.trim()), value, &folder),
        }
    }
    out
}

fn session_line(out: &mut ImportPreview, name: &str, value: &str, folder: &[String]) {
    // " #109#0%host%22%user%…#…": [flag, icon, type%fields, display, …]
    let groups: Vec<&str> = value.split('#').collect();
    let Some(main) = groups.get(2) else {
        return out.skip(name, "Not a session entry MobaXterm wrote.");
    };
    let f: Vec<&str> = main.split('%').collect();
    let kind = f.first().copied().unwrap_or_default();
    let protocol = match kind {
        "0" | "7" => ProtocolKind::Ssh, // SFTP sessions open fine in the file manager
        "1" => ProtocolKind::Telnet,
        "6" => ProtocolKind::Ftp,
        other => {
            return out.skip(
                name,
                format!("{} sessions aren't supported yet.", type_name(other)),
            );
        }
    };
    let host = unescape(f.get(1).copied().unwrap_or_default().trim());
    let default_port = match protocol {
        ProtocolKind::Ftp => 21,
        ProtocolKind::Telnet => 23,
        _ => 22,
    };
    let mut session = ImportedSession {
        name: name.to_owned(),
        folder: folder.to_vec(),
        protocol,
        host,
        port: port(f.get(2), default_port),
        username: unescape(f.get(3).copied().unwrap_or_default().trim()),
        auth_method: "password".into(),
        key_path: None,
        jump: Vec::new(),
        options: Default::default(),
    };
    if protocol == ProtocolKind::Telnet {
        session.username.clear(); // Telnet logs in on screen
    }
    if kind == "0" {
        let key = unescape(f.get(14).copied().unwrap_or_default().trim());
        let agent = f.get(33).is_some_and(|v| v.trim() == "-1");
        if !key.is_empty() {
            session.auth_method = "publickey".into();
            session.key_path = Some(key);
        } else if agent {
            session.auth_method = "agent".into();
        }
        session.jump = gateways(&f);
    }
    out.push(session);
}

/// Fields 8/9/10: gateway hosts, ports, users (`__PIPE__`-separated).
fn gateways(f: &[&str]) -> Vec<ImportedHop> {
    let list = |i: usize| -> Vec<String> {
        f.get(i)
            .map(|v| v.split("__PIPE__").map(|s| unescape(s.trim())).collect())
            .unwrap_or_default()
    };
    let (hosts, ports, users) = (list(8), list(9), list(10));
    hosts
        .iter()
        .enumerate()
        .filter(|(_, h)| !h.is_empty())
        .map(|(i, h)| ImportedHop {
            host: h.clone(),
            port: ports.get(i).and_then(|p| p.parse().ok()).unwrap_or(22),
            username: users.get(i).cloned().unwrap_or_default(),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &[u8] = b"[Bookmarks]\r\n\
SubRep=\r\n\
ImgNum=42\r\n\
web-01= #109#0%10.0.0.11%22%deploy%%-1%-1%%%%%0%0%0%%%-1%0%0%0%%1080%%0%0%1#MobaFont%10#0# #-1\r\n\
\r\n\
[Bookmarks_1]\r\n\
SubRep=Prod\\DB\r\n\
ImgNum=41\r\n\
db-01= #109#0%db01.internal%2222%postgres%%-1%-1%%bastion.example.com%22%jump%0%0%0%_CurrentDrive_:\\keys\\db.ppk%%-1%0%0%0%%1080%%0%0%1#MobaFont%10#0# #-1\r\n\
files= #140#7%sftp.example.com%22%up%%-1%-1#MobaFont#0# #-1\r\n\
win-01= #91#4%10.0.0.50%3389%admin%0%0#MobaFont#0# #-1\r\n\
old-ftp= #130#6%ftp.example.com%21%anon#MobaFont#0# #-1\r\n\
bad host= #109#0%bad host!%22%x#MobaFont#0# #-1\r\n\
\r\n\
[Passwords]\r\n\
ssh22:deploy@10.0.0.11=ENCRYPTED\r\n";

    #[test]
    fn parses_sessions_folders_gateways_and_keys() {
        let p = parse(SAMPLE);
        let names: Vec<&str> = p.sessions.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["web-01", "db-01", "files", "old-ftp"]);

        let web = &p.sessions[0];
        assert_eq!(
            (web.host.as_str(), web.port, web.username.as_str()),
            ("10.0.0.11", 22, "deploy")
        );
        assert!(web.folder.is_empty());
        assert_eq!(web.auth_method, "password");

        let db = &p.sessions[1];
        assert_eq!(db.folder, ["Prod", "DB"]);
        assert_eq!(db.port, 2222);
        assert_eq!(db.auth_method, "publickey");
        assert_eq!(db.key_path.as_deref(), Some("C:\\keys\\db.ppk"));
        assert_eq!(
            db.jump,
            [ImportedHop {
                host: "bastion.example.com".into(),
                port: 22,
                username: "jump".into()
            }]
        );

        assert_eq!(p.sessions[2].protocol, ProtocolKind::Ssh); // SFTP
        assert_eq!(p.sessions[3].protocol, ProtocolKind::Ftp);

        let skipped: Vec<(&str, &str)> = p
            .skipped
            .iter()
            .map(|s| (s.name.as_str(), s.reason.as_str()))
            .collect();
        assert!(skipped.contains(&("win-01", "RDP sessions aren't supported yet.")));
        assert!(skipped.iter().any(|(n, _)| *n == "bad host"));
        // The [Passwords] section is never read.
        assert!(!format!("{p:?}").contains("ENCRYPTED"));
    }

    #[test]
    fn decodes_windows_1252() {
        assert_eq!(decode_cp1252(&[0x80, b' ', 0xE9]), "€ é");
        assert_eq!(decode_cp1252("ok".as_bytes()), "ok");
    }

    #[test]
    fn garbage_never_panics() {
        let mut junk = b"[Bookmarks]\nSubRep=a\\\\\\b\n=\n x=#\n y=##\n z=#1#0%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%%\n".to_vec();
        junk.extend(std::iter::repeat_n(0xFFu8, 1000));
        let p = parse(&junk);
        assert!(p.sessions.is_empty());
    }

    #[test]
    fn multiple_gateways_are_kept_in_order() {
        let line = "[Bookmarks]\nSubRep=\nx= #109#0%target%22%u%%-1%-1%%gw1__PIPE__gw2%22__PIPE__2200%a__PIPE__b%0#F#0# #-1\n";
        let p = parse(line.as_bytes());
        let hops: Vec<(&str, u16, &str)> = p.sessions[0]
            .jump
            .iter()
            .map(|h| (h.host.as_str(), h.port, h.username.as_str()))
            .collect();
        assert_eq!(hops, [("gw1", 22, "a"), ("gw2", 2200, "b")]);
    }
}
