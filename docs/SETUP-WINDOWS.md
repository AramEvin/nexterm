# Windows development setup

Tested target: Windows 10 22H2 / Windows 11, x64. Run everything in **PowerShell**
(not "PowerShell as Administrator" unless a step says so).

Total download: ~6–8 GB (mostly the C++ Build Tools). Allow 30–45 minutes.

---

## 1. Microsoft C++ Build Tools

**Why:** Rust on Windows uses the MSVC linker and the Windows SDK to produce `.exe` files.
Tauri also links against WebView2 and Win32 APIs from the SDK.

**Option A: winget (recommended, one command):**

```powershell
winget install --id Microsoft.VisualStudio.2022.BuildTools --override "--passive --wait --add Microsoft.VisualStudio.Workload.VCTools --includeRecommended"
```

`--add Microsoft.VisualStudio.Workload.VCTools --includeRecommended` is the
"Desktop development with C++" workload: the MSVC compiler/linker plus the Windows 11 SDK.

**Option B: GUI:** download "Build Tools for Visual Studio 2022" from
<https://visualstudio.microsoft.com/visual-cpp-build-tools/>, run it, tick
**Desktop development with C++**, keep the default components (MSVC v143, Windows 11 SDK), install.

**Changes:** installs into `C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools`.
No PATH changes; Rust finds it automatically.

## 2. WebView2 Runtime (check only)

**Why:** Tauri renders the UI with Microsoft Edge WebView2. It ships with Windows 11
and with current Windows 10 builds.

Check:

```powershell
Get-ItemProperty "HKLM:\SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}" -ErrorAction SilentlyContinue | Select-Object pv
```

If you get a version number (`pv`), you're done. If you get nothing, install the
**Evergreen Bootstrapper** from <https://developer.microsoft.com/microsoft-edge/webview2/>
or: `winget install --id Microsoft.EdgeWebView2Runtime`.

## 3. Rust (MSVC toolchain)

**Why:** the backend is Rust. `rustup` manages compiler versions; the repo pins
`stable` via `src-tauri/rust-toolchain.toml`.

```powershell
winget install --id Rustlang.Rustup
```

Close and reopen PowerShell, then:

```powershell
rustup default stable-x86_64-pc-windows-msvc
rustup component add rustfmt clippy
rustc --version     # expect 1.85 or newer
cargo --version
```

**Changes:** installs into `%USERPROFILE%\.rustup` and `%USERPROFILE%\.cargo`, adds
`%USERPROFILE%\.cargo\bin` to your user PATH.

## 4. Node.js LTS

**Why:** builds the React frontend (Vite) and runs the Tauri CLI.

```powershell
winget install --id OpenJS.NodeJS.LTS
```

Reopen PowerShell, then `node --version` (expect v22.x or v24.x) and `npm --version`.

If `npm` scripts are blocked with _"running scripts is disabled on this system"_:

```powershell
Set-ExecutionPolicy -Scope CurrentUser RemoteSigned
```

(This only lets locally-created scripts like `npm.ps1` run for your user. It does not
change the machine-wide policy.)

## 5. Git

```powershell
winget install --id Git.Git
git config --global user.name  "Your Name"
git config --global user.email "you@example.com"
git config --global core.autocrlf false   # the repo enforces LF via .gitattributes
```

## 6. VS Code + extensions

```powershell
winget install --id Microsoft.VisualStudioCode
```

Open the repo folder; VS Code will offer the recommended extensions from
`.vscode/extensions.json`: **rust-analyzer**, **Tauri**, **ESLint**, **Prettier**,
**Tailwind CSS IntelliSense**. Accept.

## 7. Clone, install, run

```powershell
git clone <your-repo-url> nexterm
cd nexterm
npm ci                 # installs exact versions from package-lock.json
npm run tauri dev      # first run compiles ~400 Rust crates: 3–8 minutes
```

A **NexTerm** window opens with the sidebar, a Home tab and the status bar showing
`NexTerm v0.1.0`. Later runs start in seconds. Edits to `src/` hot-reload; edits to
`src-tauri/` trigger a Rust rebuild and app restart.

Logs: `%APPDATA%\NexTerm\logs\nexterm.YYYY-MM-DD.log`. More detail:
`$env:NEXTERM_LOG="debug"; npm run tauri dev`.

## Troubleshooting

| Symptom                                            | Fix                                                                                              |
| -------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| `link.exe not found` / `LNK1181`                   | C++ Build Tools missing or incomplete; re-run step 1                                             |
| `error: linker 'link.exe' not found` after install | Reopen the terminal (PATH refresh)                                                               |
| Antivirus slows or blocks the build                | Exclude `nexterm\src-tauri\target` and `%USERPROFILE%\.cargo` from real-time scanning            |
| `Port 1420 is already in use`                      | Another `tauri dev` is still running; close it                                                   |
| `Found version mismatched Tauri packages`          | `@tauri-apps/api` and the `tauri` crate must share major.minor; see CONTRIBUTING notes in README |
