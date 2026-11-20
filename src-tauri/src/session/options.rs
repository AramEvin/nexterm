//! Protocol-specific settings of a saved session (Phase 8), stored as JSON in
//! `sessions.options`. Everything is validated before it's saved or used:
//! a serial port name or a shell choice never reaches the OS unchecked.

use serde::{Deserialize, Serialize};

use crate::error::{AppError, AppResult};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SessionOptions {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub serial: Option<SerialOptions>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub local: Option<LocalOptions>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Parity {
    None,
    Odd,
    Even,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FlowControl {
    None,
    /// XON/XOFF.
    Software,
    /// RTS/CTS.
    Hardware,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct SerialOptions {
    /// `COM3` on Windows, `/dev/ttyUSB0` elsewhere.
    pub port: String,
    pub baud_rate: u32,
    pub data_bits: u8,
    pub parity: Parity,
    pub stop_bits: u8,
    pub flow_control: FlowControl,
}

impl Default for SerialOptions {
    fn default() -> Self {
        Self {
            port: String::new(),
            baud_rate: 9600,
            data_bits: 8,
            parity: Parity::None,
            stop_bits: 1,
            flow_control: FlowControl::None,
        }
    }
}

/// Shells a local terminal can start. Each maps to a fixed executable path
/// (see `local::command`); the UI can't name an arbitrary program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LocalShell {
    /// Windows PowerShell 5.1 (always installed).
    Powershell,
    /// PowerShell 7 (`pwsh.exe`), if installed.
    Pwsh,
    Cmd,
    /// Windows Subsystem for Linux.
    Wsl,
    /// The user's login shell (`$SHELL`) on Linux/macOS.
    Default,
}

impl LocalShell {
    pub fn label(self) -> &'static str {
        match self {
            Self::Powershell => "PowerShell",
            Self::Pwsh => "PowerShell 7",
            Self::Cmd => "Command Prompt",
            Self::Wsl => "WSL",
            Self::Default => "Shell",
        }
    }

    /// The shell a new local terminal opens by default on this OS.
    pub fn platform_default() -> Self {
        if cfg!(windows) {
            Self::Powershell
        } else {
            Self::Default
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct LocalOptions {
    pub shell: LocalShell,
    /// WSL distribution (`Ubuntu-24.04`); empty = the default one.
    #[serde(default)]
    pub wsl_distro: String,
}

const MAX_BAUD: u32 = 4_000_000;

/// `COM1`…`COM256`, or a Unix device such as `/dev/ttyUSB0`, `/dev/cu.usbserial-1`, `/dev/pts/3`.
pub fn serial_port_name(raw: &str) -> AppResult<String> {
    let name = raw.trim();
    let bad = || {
        AppError::Validation(format!(
            "\"{name}\" isn't a serial port name (e.g. COM3 or /dev/ttyUSB0)."
        ))
    };
    if let Some(num) = name.to_ascii_uppercase().strip_prefix("COM") {
        let n: u16 = num.parse().map_err(|_| bad())?;
        return if (1..=256).contains(&n) && !num.starts_with('0') {
            Ok(format!("COM{n}"))
        } else {
            Err(bad())
        };
    }
    let unix_ok = name.len() <= 64
        && (name.starts_with("/dev/tty")
            || name.starts_with("/dev/cu.")
            || name.starts_with("/dev/pts/"))
        && name[5..]
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-' | b'/'))
        && !name.contains("..")
        && !name.ends_with('/');
    if unix_ok {
        Ok(name.to_owned())
    } else {
        Err(bad())
    }
}

impl SerialOptions {
    pub fn validated(&self) -> AppResult<Self> {
        if !(50..=MAX_BAUD).contains(&self.baud_rate) {
            return Err(AppError::Validation(
                "Pick a speed between 50 and 4,000,000 baud.".into(),
            ));
        }
        if !(5..=8).contains(&self.data_bits) {
            return Err(AppError::Validation(
                "Data bits must be 5, 6, 7 or 8.".into(),
            ));
        }
        if !matches!(self.stop_bits, 1 | 2) {
            return Err(AppError::Validation("Stop bits must be 1 or 2.".into()));
        }
        Ok(Self {
            port: serial_port_name(&self.port)?,
            ..self.clone()
        })
    }

    /// Shown where a host would be (`COM3 · 115200 8N1`).
    pub fn summary(&self) -> String {
        let p = match self.parity {
            Parity::None => 'N',
            Parity::Odd => 'O',
            Parity::Even => 'E',
        };
        format!(
            "{} · {} {}{}{}",
            self.port, self.baud_rate, self.data_bits, p, self.stop_bits
        )
    }
}

impl LocalOptions {
    pub fn validated(&self) -> AppResult<Self> {
        let distro = self.wsl_distro.trim();
        if self.shell != LocalShell::Wsl && !distro.is_empty() {
            return Err(AppError::Validation(
                "Only WSL takes a distribution name.".into(),
            ));
        }
        if distro.len() > 64
            || !distro
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return Err(AppError::Validation(
                "A WSL distribution name uses letters, digits, '.', '_' and '-' only.".into(),
            ));
        }
        Ok(Self {
            shell: self.shell,
            wsl_distro: distro.to_owned(),
        })
    }

    /// Shown where a host would be (`WSL · Ubuntu`).
    pub fn summary(&self) -> String {
        if self.wsl_distro.is_empty() {
            self.shell.label().to_owned()
        } else {
            format!("{} · {}", self.shell.label(), self.wsl_distro)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serial_port_names() {
        assert_eq!(serial_port_name(" com3 ").unwrap(), "COM3");
        assert_eq!(serial_port_name("COM256").unwrap(), "COM256");
        for bad in [
            "COM0",
            "COM257",
            "COM03",
            "COMx",
            "COM",
            "\\\\.\\COM3",
            "/dev/../etc/passwd",
            "/etc/passwd",
            "/dev/tty;rm",
            "/dev/ttyUSB0/",
            "",
        ] {
            assert!(serial_port_name(bad).is_err(), "{bad}");
        }
        for ok in [
            "/dev/ttyUSB0",
            "/dev/ttyS1",
            "/dev/cu.usbserial-1410",
            "/dev/pts/3",
        ] {
            assert_eq!(serial_port_name(ok).unwrap(), ok);
        }
    }

    #[test]
    fn serial_settings_are_checked() {
        let good = SerialOptions {
            port: "com7".into(),
            baud_rate: 115_200,
            ..SerialOptions::default()
        };
        let v = good.validated().unwrap();
        assert_eq!(v.port, "COM7");
        assert_eq!(v.summary(), "COM7 · 115200 8N1");
        for bad in [
            SerialOptions {
                baud_rate: 10,
                ..good.clone()
            },
            SerialOptions {
                data_bits: 9,
                ..good.clone()
            },
            SerialOptions {
                stop_bits: 3,
                ..good.clone()
            },
        ] {
            assert!(bad.validated().is_err());
        }
    }

    #[test]
    fn local_options_are_checked() {
        let wsl = LocalOptions {
            shell: LocalShell::Wsl,
            wsl_distro: " Ubuntu-24.04 ".into(),
        };
        assert_eq!(wsl.validated().unwrap().wsl_distro, "Ubuntu-24.04");
        assert_eq!(wsl.validated().unwrap().summary(), "WSL · Ubuntu-24.04");
        assert!(
            LocalOptions {
                shell: LocalShell::Wsl,
                wsl_distro: "a b".into()
            }
            .validated()
            .is_err()
        );
        assert!(
            LocalOptions {
                shell: LocalShell::Wsl,
                wsl_distro: "x;calc".into()
            }
            .validated()
            .is_err()
        );
        assert!(
            LocalOptions {
                shell: LocalShell::Cmd,
                wsl_distro: "Ubuntu".into()
            }
            .validated()
            .is_err()
        );
    }

    #[test]
    fn json_round_trip_and_unknown_fields() {
        let o = SessionOptions {
            serial: Some(SerialOptions {
                port: "COM1".into(),
                ..SerialOptions::default()
            }),
            local: None,
        };
        let json = serde_json::to_string(&o).unwrap();
        assert!(json.contains("\"baudRate\":9600") && !json.contains("local"));
        assert_eq!(serde_json::from_str::<SessionOptions>(&json).unwrap(), o);
        assert_eq!(
            serde_json::from_str::<SessionOptions>("{}").unwrap(),
            SessionOptions::default()
        );
        assert!(serde_json::from_str::<SessionOptions>(r#"{"exe":"calc.exe"}"#).is_err());
    }
}
