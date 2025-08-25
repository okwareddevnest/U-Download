# U-Download

A beautiful, fast, and cross-platform YouTube downloader built with React, Tailwind CSS, and Tauri (Rust).

## Features

🚀 **High-Performance Downloads**
- Powered by `yt-dlp` + `aria2c` for maximum speed
- Real-time progress tracking with speed and ETA
- Support for multiple quality options (360p, 480p, 720p, 1080p, best)

🎨 **Modern UI/UX**
- Beautiful card-based interface built with Tailwind CSS  
- Dark/light theme toggle with persistence
- Responsive design and smooth animations
- URL validation with visual feedback

📁 **Flexible Output**
- MP4 (video) and MP3 (audio) download options
- Native file picker for output folder selection
- Settings persistence (remembers your preferences)

⚡ **Cross-Platform**
- Linux: AppImage, .deb packages
- Windows: .exe installer, .msi
- macOS: .app, .dmg

## Requirements

Before running U-Download, make sure you have these dependencies installed:

- **yt-dlp**: YouTube downloader
- **aria2c**: Multi-connection download accelerator

### Installation Commands

**Ubuntu/Debian:**
```bash
sudo apt update
sudo apt install yt-dlp aria2
```

**macOS (Homebrew):**
```bash
brew install yt-dlp aria2
```

**Windows (Chocolatey):**
```powershell
choco install yt-dlp aria2
```

## Installation & Desktop Icon

### Installing the Application

To install U-Download with proper desktop integration and icon:

1. Build the application:
```bash
npm run tauri build -- --no-bundle
```

2. Run the installation script:
```bash
./install.sh
```

This will:
- Install the application icon to `~/.local/share/icons/hicolor/256x256/apps/`
- Create a desktop entry in `~/.local/share/applications/`
- Update the icon cache for immediate availability

### Desktop Icon Issues Fixed

The following issues were resolved:
- **Corrupted ICO file**: The original Windows icon file was corrupted and has been recreated
- **Incorrect PNG format**: All PNG icons were converted to proper RGBA format
- **Missing desktop integration**: Added proper .desktop file and installation script
- **Icon sizing**: Created icons in all required sizes (32x32, 128x128, 256x256, 512x512)
- **High-DPI support**: Added 128x128@2x.png for high-DPI displays

### Video Quality Improvements

The video download quality has been significantly improved:

- **Better Format Selection**: Now uses `bestvideo[height<=RESOLUTION]+bestaudio` for optimal quality within each resolution
- **Consistent Output**: Added `--merge-output-format mp4` for consistent MP4 output
- **Codec Optimization**: Added `--prefer-free-formats` to avoid proprietary codecs when possible
- **Quality Assurance**: Each resolution now gets the best available quality for that specific height

**Quality Options:**
- 📱 **360p (Mobile)**: Best video quality up to 360p resolution
- 💻 **480p (Standard)**: Best video quality up to 480p resolution
- 🖥️ **720p (HD)**: Best video quality up to 720p resolution
- 🎯 **1080p (Full HD)**: Best video quality up to 1080p resolution

## Development

### Prerequisites
- Node.js (LTS version)
- Rust (latest stable)
- System dependencies for Tauri

### Setup
```bash
# Clone the repository
git clone <repository-url>
cd u-download

# Install frontend dependencies
npm install

# Run in development mode
npm run tauri dev
```

### Building
```bash
# Build for production
npm run tauri build
```

### Project Structure
```
u-download/
├── src/                    # React frontend
│   ├── App.jsx            # Main application component
│   └── App.css            # Tailwind CSS imports
├── src-tauri/             # Rust backend
│   ├── src/lib.rs         # Main Tauri application logic
│   ├── Cargo.toml         # Rust dependencies
│   └── tauri.conf.json    # Tauri configuration
└── package.json           # Node.js dependencies
```

## Usage

1. **Enter YouTube URL**: Paste any YouTube video URL
2. **Select Format**: Choose between MP4 (video) or MP3 (audio)
3. **Choose Quality**: Select from 360p to 1080p, or "Best Available"
4. **Pick Output Folder**: Use the file browser to select where to save
5. **Start Download**: Click the download button and watch real-time progress

## Technical Details

### Frontend (React + Tailwind)
- **React 19** with hooks for state management
- **Tailwind CSS** for styling and responsive design
- **Real-time progress** via Tauri event system
- **LocalStorage** for settings persistence

### Backend (Rust + Tauri)
- **yt-dlp integration** for YouTube video extraction
- **aria2c integration** for high-speed multi-connection downloads
- **Real-time progress parsing** from download output
- **Cross-platform file dialogs** for folder selection

### Download Command
```bash
yt-dlp \
  --external-downloader aria2c \
  --external-downloader-args "-x 16 -s 16 -k 1M" \
  --progress --newline \
  -o "output_folder/%(title)s.%(ext)s" \
  -f "format_selector" \
  "video_url"
```

## Packaging & Distribution

The project includes automated CI/CD with GitHub Actions:

- **Triggers**: On version tag push (`v*`)
- **Platforms**: Linux, Windows, macOS (both Intel and Apple Silicon)
- **Artifacts**: AppImage, .deb, .exe, .msi, .dmg, .app
- **Checksums**: SHA256SUMS.txt for verification

## License

Copyright © 2025 U-Download. All rights reserved.

---

**Made with ❤️ using React, Tailwind CSS, and Tauri**