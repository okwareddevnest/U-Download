# U‑Download

Fast, cross‑platform YouTube downloader with a modern UI. Powered by `yt-dlp` + `aria2c` for top speed.

## Download

- Latest release (always up to date):
  - https://github.com/okwareddevnet/u-download/releases/latest
- Current version: v2.1.0
  - https://github.com/okwareddevnet/u-download/releases/tag/v2.1.0

Pick the installer for your OS from the “Assets” section of the release:

- Linux: `.AppImage`, `.deb`, or `.rpm`
  - AppImage: `chmod +x *.AppImage && ./*.AppImage`
  - Deb: `sudo dpkg -i U-Download_*_amd64.deb`
  - Rpm: `sudo rpm -i U-Download-*.x86_64.rpm`
- Windows: NSIS `.exe` installer
- macOS: `.dmg` (Intel and Apple Silicon), drag to Applications

Note: binaries aren’t codesigned. On macOS, allow the app in System Settings → Privacy & Security if prompted.

## Features

- Fast downloads: `yt-dlp` + `aria2c` for multi‑connection speed
- Reliable: progress bar, current speed and ETA; clear errors on failure
- Video or audio: MP4 or MP3 with sensible defaults
- Quality presets: 360p / 480p / 720p / 1080p / Best
- Precise trimming:
  - Per‑second slider control
  - Manual time inputs (SS, MM:SS, or HH:MM:SS)
  - Uses FFmpeg when trimming is enabled
- Clean UI: dark/light theme, smooth animations, URL validation
- Folder control: choose any output folder; settings persist between runs
- Dependency check: quick test for yt‑dlp, aria2c, and FFmpeg
- Cross‑platform installers: Linux (.AppImage, .deb, .rpm), Windows (.exe), macOS (.dmg)

## Requirements

Install these once on your system:

- yt‑dlp (required)
- aria2c (required)
- ffmpeg (optional; required for trimming)

Quick install:

- Ubuntu/Debian: `sudo apt update && sudo apt install yt-dlp aria2 ffmpeg`
- macOS (Homebrew): `brew install yt-dlp aria2 ffmpeg`
- Windows (Chocolatey): `choco install yt-dlp aria2 ffmpeg`

## How to Use

1. Paste a YouTube URL
2. Choose MP4/MP3 and quality
3. Select an output folder
4. (Optional) Use Trim to set start/end times (per‑second accuracy)
5. Click Start Download and watch progress, speed, and ETA

## Troubleshooting

- “yt-dlp/aria2c not found”: install using the commands above
- Trimming fails: ensure `ffmpeg` is installed
- Linux AppImage won’t start: `chmod +x` then run from a writable folder

## License

MIT
