# Substack Downloader (Desktop EXE)

Desktop GUI app for downloading public Substack posts and exporting them as EPUB and TXT.

## What this repo is now

- Desktop-only Tauri app.
- No Vite/React build pipeline in this repo.
- Frontend assets are prebuilt and stored in `dist/`.
- Rust backend/source is in `src-tauri/`.

## Features

- Load an entire Substack publication from URL.
- Export modes:
  - Entire profile (date sorted)
  - Specific posts (manual reorder screen)
- Output formats:
  - EPUB
  - TXT
- Granularity:
  - Per-post files
  - Combined single file
- EPUB cover options:
  - Upload custom cover image
  - Use Substack author/publication cover with generated title page text
- Footnote extraction and linked in-EPUB footnotes.

## Build the EXE

Prerequisites:
- Rust toolchain (`rustup`, stable)
- Visual Studio Build Tools on Windows (for native dependencies)

Command:

```powershell
cd src-tauri
cargo build --release
```

Output:

- EXE: `src-tauri/target/release/substack-downloader.exe`

## Run from source

```powershell
cd src-tauri
cargo run
```

## GitHub upload checklist

1. Keep `dist/` committed (required for desktop UI at runtime).
2. Do not commit `src-tauri/target/` or caches.
3. Commit `src-tauri/Cargo.lock` for reproducible Rust builds.
4. Create releases by attaching the built `.exe` from `src-tauri/target/release/`.

## Scope

- Public Substack posts only.
- PDF export is not included.
