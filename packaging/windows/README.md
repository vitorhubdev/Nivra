Build `cargo xtask package` using Rust 1.98.1 MSVC and Visual Studio C++ build tools. The package is a single executable, `dist/Nivra.exe`: download it and open it, nothing to extract. The login flow requires WebView2. Source builds require CMake for the built-in voice engine.

Run `dist/nivra.exe --demo` for an offline synthetic preview with no saved-login lookup or account storage. Run without `--demo` only when the owner is ready to operate their account. See [platform support](../../docs/platform-support.md) for the live-test boundary and limitations.

September 10, 2026: Windows x64 workspace checks pass, including all 70 offline Rust tests. The text release created a responsive native window in a process smoke check; visual interaction could not be inspected because the Computer Use helper was unavailable. Installer creation, signing, native Save As, authentication, physical audio and accessibility remain unverified.

System notifications require a Start Menu shortcut carrying Nivra's own AppUserModelID (`io.github.vitorhubdev.Nivra`). The executable registers this shortcut itself on first run (and refreshes it when it moved): only the current user's `Nivra.lnk`, without administrator rights or autostart. No script is needed.

Desktop notifications are on by default for new installs and can be turned off in Nivra; Windows may still block them in Settings. Message alerts include a bounded sender name and content preview; an already-cached avatar PNG is added when available. Other alerts remain generic. Windows may retain notification content in its history; logout/disable requests removal of Nivra's outstanding notification history through WinRT, without claiming forensic erasure. Native activation/deep links are not implemented. On Windows 11, `ToastNotifier.Setting()` can return `0x80070490` for Nivra even when its notifier can submit a toast; this no longer blocks delivery. The [Microsoft shortcut/AUMID requirement](https://learn.microsoft.com/windows/win32/shell/enable-desktop-toast-with-appusermodelid) and the pinned notify-rust/WinRT source APIs were reviewed September 16, 2026.

## Windows Installer & Setup

Nivra ships as a single executable and also offers a per-user Windows installer:
- **NSIS Installer**: `packaging/windows/installer.nsi` builds `dist-installer/nivra-<version>-setup.exe` via `makensis`. It installs per-user to `%LOCALAPPDATA%\Programs\Nivra` (`RequestExecutionLevel user`) without requiring administrator elevation. This preserves full user write permissions for the in-app autoupdater.
- **PowerShell Setup**: `packaging/windows/setup.ps1` provides a zero-dependency installer/uninstaller script (`powershell -File .\setup.ps1` to install, `powershell -File .\setup.ps1 -Uninstall` to remove).
- **Autoupdate Compatibility**: Both installers register Nivra in `HKCU\Software\Microsoft\Windows\CurrentVersion\Uninstall\Nivra`. When Nivra autoupdates, the update helper automatically synchronizes `DisplayVersion` in the registry upon file replacement, keeping Windows Settings and Installed Apps accurate. The single-exe release asset stages ready to move, so in-app updates never unpack an archive on Windows.
