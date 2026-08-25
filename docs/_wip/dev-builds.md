# Platform dev-build commands (local-only)

Local dev convenience for Windows/Linux. This file never enters upstream PRs.

## Windows

The Rust build fails when it picks up Git's bundled `link.exe`. Run from a Visual
Studio-enabled shell (Developer PowerShell for VS 2022), or use
`apps/desktop/tauri-dev.cmd`, which calls `vcvarsall.bat x64` before
`npm run tauri dev`:

```powershell
cd apps/desktop
npm install
npm run tauri dev
```

## Linux (Debian/Ubuntu)

Other distros: see [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/).

```bash
sudo apt-get install -y libwebkit2gtk-4.1-dev build-essential curl wget file \
  libxdo-dev libssl-dev libayatana-appindicator3-dev librsvg2-dev
cd apps/desktop
npm install
npm run tauri dev
```

Debug build without launching:

```bash
npm run tauri build -- --debug
```
