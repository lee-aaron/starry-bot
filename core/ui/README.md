# Starry Bot - Iced UI

This is the new iced-based UI for Starry Bot, replacing the previous Tauri + Svelte setup.

## Features

- **Window Selection**: Browse and select windows to capture
- **Real-time Capture**: Start/stop minimap capture with live status
- **Frame Saving**: Captured frames are automatically saved as PNG files
- **Cross-platform**: Built with iced for native performance

## Running

### Development
```bash
npm run dev
# or directly:
cd core/ui && cargo run --features local
```

### Production Build
```bash
npm run build
# or directly:
cd core/ui && cargo build --release
```

## Architecture

- **UI Layer**: iced application (`core/ui/src/main.rs`)
- **Service Layer**: MinimapService (`core/interface/src/services/minimap.rs`)
- **Capture Layer**: Windows capture API (`core/platforms/src/windows_capture/`)

## Migration from Tauri

The following Tauri components have been removed/replaced:
- Tauri backend → Direct iced integration
- Svelte frontend → Native iced widgets
- Tauri events → Direct service calls
- Web-based UI → Native desktop application

## Output

Captured frames are saved as timestamped PNG files in the working directory:
- `capture_1693756123456.png`
- `capture_1693756123789.png`
- etc.
