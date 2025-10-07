# U‑Download

**Fast, beautiful YouTube downloader with ZERO dependencies!** ✨

Download YouTube videos and audio instantly - no installation of external tools required. Everything you need is bundled right in the app.

## ⚡ What Makes U-Download Special

- 🎯 **Zero Dependencies**: All tools bundled (yt-dlp, aria2c, ffmpeg) - just install and go!
- 🚀 **Lightning Fast**: Multi-connection downloads with aria2c acceleration
- 💎 **Beautiful UI**: Modern, clean interface with smooth animations
- 🎨 **Cross-Platform**: Works on Linux, Windows, and macOS
- 🎵 **Flexible Formats**: Download as MP4 video or MP3 audio
- ✂️ **Precise Trimming**: Cut videos with per-second accuracy
- 📊 **Live Progress**: Real-time speed, ETA, and progress tracking

## 📥 Download & Install

**Latest Version: v2.2.5** (October 2025)

### Quick Links
- **Latest Release**: https://github.com/okwareddevnet/u-download/releases/latest
- **v2.2.5 Release**: https://github.com/okwareddevnet/u-download/releases/tag/v2.2.5

### Installation by Platform

#### Linux
```bash
# AppImage (any distribution)
chmod +x U-Download_*.AppImage && ./U-Download_*.AppImage

# Debian/Ubuntu
sudo dpkg -i U-Download_*_amd64.deb

# Fedora/RHEL/openSUSE
sudo rpm -i U-Download-*.x86_64.rpm
```

#### Windows
1. Download `U-Download_*_x64-setup.exe`
2. Run the installer
3. Launch from Start Menu

#### macOS
1. Download `U-Download_*.dmg`
2. Open DMG and drag to Applications
3. **First launch**: Right-click → Open (to bypass unsigned app warning)

> **Note**: Binaries aren't codesigned. On macOS, allow the app in System Settings → Privacy & Security if prompted.

## ✨ Features

### Download Options
- **Multiple Formats**: Download as MP4 (video) or MP3 (audio)
- **Quality Presets**: 360p, 480p, 720p, 1080p, or Best available
- **Smart Defaults**: Optimized settings for best quality and speed

### Advanced Features
- **✂️ Precise Video Trimming**:
  - Per-second slider control
  - Manual time inputs (SS, MM:SS, or HH:MM:SS)
  - Powered by bundled FFmpeg
- **📊 Real-Time Progress**:
  - Live download speed (MB/s)
  - Accurate time remaining (ETA)
  - Visual progress bar
- **📁 Folder Control**:
  - Choose any output directory
  - Settings persist between sessions
  - Smart file naming

### User Experience
- **🎨 Modern UI**: Clean, intuitive interface with smooth animations
- **🌓 Theme Support**: Automatic dark/light theme
- **✅ URL Validation**: Instant feedback on valid YouTube links
- **🔍 Dependency Check**: Built-in tool verification (all bundled!)
- **🚨 Clear Error Messages**: Helpful troubleshooting information

### Technical
- **Zero Installation Required**: All tools bundled (no PATH configuration needed)
- **Multi-threaded Downloads**: aria2c with 16 connections for maximum speed
- **Latest YouTube Support**: yt-dlp 2025.09.26 with current API compatibility
- **Cross-Platform**: Native installers for all major operating systems

## 🚀 How to Use

1. **Launch U-Download** - All dependencies are already bundled!
2. **Paste YouTube URL** - Supports various YouTube link formats
3. **Choose Format**:
   - MP4 for video downloads
   - MP3 for audio-only downloads
4. **Select Quality** - From 360p to 1080p or Best available
5. **Pick Output Folder** - Choose where to save your downloads
6. **Optional: Trim Video**:
   - Toggle "Trim Video" checkbox
   - Use sliders or manual time inputs (HH:MM:SS)
   - Set precise start and end times
7. **Start Download** - Click the big download button!
8. **Monitor Progress** - Watch real-time speed, ETA, and progress bar

### First Time Setup

On first launch, the app will verify bundled binaries (yt-dlp, aria2c, ffmpeg). You should see:
- ✅ yt-dlp: 2025.09.26
- ✅ aria2c: 1.37.0  
- ✅ FFmpeg: 7.0.2-static

If all show ✅, you're ready to download!

## 🔧 Troubleshooting

### Common Issues

**"Download failed" or "Format not available"**
- YouTube may have updated their API - check for U-Download updates
- Try a different quality setting
- Ensure you have a stable internet connection

**Linux AppImage won't start**
```bash
# Make sure it's executable
chmod +x U-Download_*.AppImage

# Run from a writable location (not /tmp in some distros)
mv U-Download_*.AppImage ~/Downloads/
cd ~/Downloads/
./U-Download_*.AppImage
```

**macOS: "App can't be opened because it's from an unidentified developer"**
1. Right-click the app → Open
2. Click "Open" in the dialog
3. Or go to System Settings → Privacy & Security → Allow app

**Windows: "Windows protected your PC" warning**
1. Click "More info"
2. Click "Run anyway"
3. This is normal for unsigned applications

### Still Having Issues?

1. **Check for updates**: Visit the [releases page](https://github.com/okwareddevnet/u-download/releases)
2. **View bundled tools**: Click "Check Dependencies" button in the app
3. **Report bugs**: Open an [issue on GitHub](https://github.com/okwareddevnet/u-download/issues)

## 📚 Documentation

- **User Guide**: See `USER_GUIDE.md` for detailed instructions
- **Installation Guide**: See `INSTALLATION.md` for platform-specific notes
- **Developer Guide**: See `DEVELOPER.md` for building from source

## 🔒 Privacy & Security

- **No Telemetry**: We don't track or collect any data
- **No Network Calls**: Except to download videos (yt-dlp → YouTube directly)
- **Open Source**: Full source code available for review
- **Local Processing**: All downloads and trimming happen on your machine

## 🤝 Contributing

Contributions are welcome! Please see `DEVELOPER.md` for:
- Building from source
- Development setup
- Testing guidelines
- Commit conventions

## 📄 License

MIT License - See `LICENSE` file for details

## 🙏 Credits

U-Download is powered by excellent open-source tools:
- **[yt-dlp](https://github.com/yt-dlp/yt-dlp)**: YouTube video extraction
- **[aria2](https://aria2.github.io/)**: Multi-connection download acceleration
- **[FFmpeg](https://ffmpeg.org/)**: Video processing and trimming
- **[Tauri](https://tauri.app/)**: Cross-platform desktop framework
- **[React](https://react.dev/)**: UI framework

## ⭐ Support

If you find U-Download useful, please:
- ⭐ Star the repository
- 🐛 Report bugs
- 💡 Suggest features
- 📢 Share with others

---

**Made with ❤️ by the U-Download team**

