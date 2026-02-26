# Substack EPUB Downloader

## Run From Releases (No Commands Needed)

1. Open the GitHub **Releases** page for this repo.
2. Download `substack-downloader.exe`.
3. Run the EXE.

That is all end users need.

## Features

- Desktop EXE app (Tauri).
- Download public Substack publications.
- Export formats: `EPUB`, `TXT`.
- Modes:
  - Entire profile (date sorted)
  - Specific post selection
- Final reorder step for selected posts (number rank + up/down).
- Combined export or per-post export.
- EPUB cover options:
  - Use Substack author/publication cover
  - Upload custom cover
- Footnote handling for EPUB output.
- Animated game-style UI with Three.js background.

## Development (Optional)

```powershell
npm install
npm run tauri dev
```
