# Substack Downloader (Tauri Desktop EXE)

Desktop GUI app for downloading public Substack posts and exporting them as EPUB and TXT.

## Structure

- `src/`: editable React + TypeScript frontend source.
- `dist/`: built frontend assets used by Tauri at runtime.
- `src-tauri/`: Rust backend + Tauri app shell.

## End users

End users only need the released `.exe`. No commands are required.

## Development

Prerequisites:
- Node.js 18+
- Rust stable toolchain
- Visual Studio Build Tools (Windows)

Install dependencies:

```powershell
npm install
```

Run desktop app in dev mode:

```powershell
npm run tauri dev
```

Build frontend only:

```powershell
npm run build
```

Build release EXE:

```powershell
npm run tauri build
```

Or (if `dist/` is already up to date):

```powershell
cd src-tauri
cargo build --release
```

## Notes

- Public Substack posts only.
- PDF export is not included.
