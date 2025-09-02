; English Language File for U-Download NSIS Installer
; Zero Dependencies Messaging

!define LANG_ENGLISH_ZERO_DEPS "Zero External Dependencies"
!define LANG_ENGLISH_BUNDLED_TOOLS "All Tools Bundled"
!define LANG_ENGLISH_READY_TO_USE "Ready to Use Immediately"

; Custom strings for U-Download
LangString ZERO_DEPS_TITLE ${LANG_ENGLISH} "Zero Dependencies Installation"
LangString ZERO_DEPS_DESC ${LANG_ENGLISH} "U-Download comes with everything needed to work immediately. No external dependencies required."

LangString BUNDLED_COMPONENTS ${LANG_ENGLISH} "Bundled Components"
LangString BUNDLED_YTDLP ${LANG_ENGLISH} "yt-dlp (YouTube downloader) - INCLUDED"
LangString BUNDLED_ARIA2C ${LANG_ENGLISH} "aria2c (Download accelerator) - INCLUDED" 
LangString BUNDLED_FFMPEG ${LANG_ENGLISH} "FFmpeg (Media processor) - INCLUDED"

LangString NO_PYTHON_NEEDED ${LANG_ENGLISH} "✓ No Python installation required"
LangString NO_EXTERNAL_TOOLS ${LANG_ENGLISH} "✓ No external tool setup needed"
LangString NO_PATH_CONFIG ${LANG_ENGLISH} "✓ No PATH configuration required"
LangString IMMEDIATE_USE ${LANG_ENGLISH} "✓ Use immediately after installation"

LangString INSTALL_COMPLETE_TITLE ${LANG_ENGLISH} "Installation Complete!"
LangString INSTALL_COMPLETE_MSG ${LANG_ENGLISH} "U-Download is ready to use with all dependencies bundled.$\r$\n$\r$\nAll required tools are included and configured."

LangString UNINSTALL_CONFIRM ${LANG_ENGLISH} "Are you sure you want to completely remove U-Download and all of its components?"
LangString UNINSTALL_SUCCESS ${LANG_ENGLISH} "U-Download was successfully removed from your computer."

; Version-specific messages
LangString VERSION_CHECK ${LANG_ENGLISH} "Checking for existing installation..."
LangString VERSION_UPGRADE ${LANG_ENGLISH} "Upgrading U-Download with preserved settings..."
LangString VERSION_CLEAN ${LANG_ENGLISH} "Clean installation of U-Download..."

; Error messages
LangString ERROR_ADMIN_REQUIRED ${LANG_ENGLISH} "Administrator privileges are required to install U-Download."
LangString ERROR_WINDOWS_VERSION ${LANG_ENGLISH} "U-Download requires Windows 7 or later."
LangString ERROR_DISK_SPACE ${LANG_ENGLISH} "Insufficient disk space. U-Download requires approximately 604 MB of free space."
LangString ERROR_EXISTING_PROCESS ${LANG_ENGLISH} "U-Download is currently running. Please close it before continuing with the installation."

; Success messages  
LangString SUCCESS_DESKTOP_SHORTCUT ${LANG_ENGLISH} "Desktop shortcut created successfully."
LangString SUCCESS_STARTMENU ${LANG_ENGLISH} "Start menu shortcuts created successfully."
LangString SUCCESS_REGISTRY ${LANG_ENGLISH} "Registry entries created successfully."

; Feature descriptions
LangString DESC_MAIN_COMPONENT ${LANG_ENGLISH} "U-Download application with all dependencies bundled (603 MB). This component is required and includes yt-dlp, aria2c, and FFmpeg."
LangString DESC_DESKTOP_SHORTCUT ${LANG_ENGLISH} "Creates a convenient desktop shortcut for quick access to U-Download."
LangString DESC_STARTMENU_SHORTCUT ${LANG_ENGLISH} "Adds U-Download to the Start Menu for easy discovery and launching."

; Post-installation messages
LangString POST_INSTALL_TIPS ${LANG_ENGLISH} "Installation Tips:"
LangString TIP_FIRST_RUN ${LANG_ENGLISH} "• Launch U-Download from desktop or Start Menu"
LangString TIP_PASTE_URL ${LANG_ENGLISH} "• Paste any YouTube URL to start downloading"
LangString TIP_ZERO_SETUP ${LANG_ENGLISH} "• No additional setup or configuration required"
LangString TIP_SUPPORT ${LANG_ENGLISH} "• Visit our GitHub page for support and updates"