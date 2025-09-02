; NSIS Template for U-Download
; Zero Dependencies YouTube Downloader

Unicode True
SetCompressor /SOLID lzma

; Modern UI Configuration
!include "MUI2.nsh"
!include "FileFunc.nsh"
!include "WinVer.nsh"

; Application Information
!define APPNAME "U-Download"
!define DESCRIPTION "Fast YouTube Downloader with Zero Dependencies"
!define VERSIONMAJOR 2
!define VERSIONMINOR 2
!define VERSIONBUILD 0
!define COMPANY "U-Download Team"
!define INSTALLSIZE 603800  ; 603.8MB (approximate size with all binaries)

; Registry and installation paths
InstallDir "$PROGRAMFILES64\U-Download"
RequestExecutionLevel admin

; Version Information
VIProductVersion "${VERSIONMAJOR}.${VERSIONMINOR}.${VERSIONBUILD}.0"
VIAddVersionKey "ProductName" "${APPNAME}"
VIAddVersionKey "ProductVersion" "${VERSIONMAJOR}.${VERSIONMINOR}.${VERSIONBUILD}"
VIAddVersionKey "CompanyName" "${COMPANY}"
VIAddVersionKey "LegalCopyright" "© 2025 U-Download Team"
VIAddVersionKey "FileDescription" "${DESCRIPTION}"
VIAddVersionKey "FileVersion" "${VERSIONMAJOR}.${VERSIONMINOR}.${VERSIONBUILD}"

; Modern UI Pages Configuration
!define MUI_ABORTWARNING
!define MUI_UNABORTWARNING

; Welcome page with zero dependency message
!define MUI_WELCOMEPAGE_TITLE "Welcome to U-Download Setup"
!define MUI_WELCOMEPAGE_TEXT "This installer will install U-Download with ZERO external dependencies.$\r$\n$\r$\nU-Download includes all required tools:$\r$\n• yt-dlp (YouTube downloader) - BUNDLED$\r$\n• aria2c (Download accelerator) - BUNDLED$\r$\n• FFmpeg (Media processor) - BUNDLED$\r$\n$\r$\nNo additional software installation required!"

; Custom license page text
!define MUI_LICENSEPAGE_TEXT_TOP "Please review the license terms below. U-Download is free and open source software."
!define MUI_LICENSEPAGE_TEXT_BOTTOM "Click 'I Agree' if you accept the terms of the agreement."

; Components page customization
!define MUI_COMPONENTSPAGE_SMALLDESC
!define MUI_COMPONENTSPAGE_TEXT_TOP "Select the components you would like to install."

; Directory page customization
!define MUI_DIRECTORYPAGE_TEXT_TOP "Setup will install U-Download in the following folder."

; Installation page
!define MUI_INSTFILESPAGE_COLORS "FFFFFF 000000"

; Finish page customization
!define MUI_FINISHPAGE_TITLE "U-Download Installation Complete"
!define MUI_FINISHPAGE_TEXT "U-Download has been successfully installed with all dependencies bundled.$\r$\n$\r$\n✓ Ready to use immediately$\r$\n✓ No Python, yt-dlp, or FFmpeg setup needed$\r$\n✓ All tools included and configured$\r$\n$\r$\nClick Finish to complete the setup."
!define MUI_FINISHPAGE_RUN "$INSTDIR\u-download.exe"
!define MUI_FINISHPAGE_RUN_TEXT "Launch U-Download now"
!define MUI_FINISHPAGE_SHOWREADME ""
!define MUI_FINISHPAGE_SHOWREADME_TEXT "View release notes"
!define MUI_FINISHPAGE_LINK "Visit U-Download website"
!define MUI_FINISHPAGE_LINK_LOCATION "https://github.com/okwareddevnest/U-Download"

; UI Pages
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE
!insertmacro MUI_PAGE_COMPONENTS  
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

; Uninstaller pages
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

; Language files
!insertmacro MUI_LANGUAGE "English"

; Check Windows version compatibility
Function .onInit
    ${IfNot} ${AtLeastWin7}
        MessageBox MB_OK "U-Download requires Windows 7 or later."
        Abort
    ${EndIf}
    
    ; Check for existing installation
    ReadRegStr $R0 HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "UninstallString"
    StrCmp $R0 "" done
    
    MessageBox MB_OKCANCEL|MB_ICONEXCLAMATION \
    "U-Download is already installed. $\n$\nClick 'OK' to remove the previous version or 'Cancel' to cancel this upgrade." \
    IDOK uninst
    Abort

uninst:
    ClearErrors
    ExecWait '$R0 _?=$INSTDIR'
    
    IfErrors no_remove_uninstaller done
    no_remove_uninstaller:

done:
FunctionEnd

; Installation sections
Section "U-Download (required)" SecMain
    SectionIn RO  ; Read-only, always installed
    
    ; Set output path
    SetOutPath "$INSTDIR"
    
    ; Include all application files
    File /r "${TAURI_SOURCE_DIR}\*"
    
    ; Create uninstaller
    WriteUninstaller "$INSTDIR\Uninstall.exe"
    
    ; Registry entries for Add/Remove Programs
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "DisplayName" "U-Download"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "UninstallString" "$\"$INSTDIR\Uninstall.exe$\""
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "QuietUninstallString" "$\"$INSTDIR\Uninstall.exe$\" /S"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "DisplayIcon" "$INSTDIR\u-download.exe"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "Publisher" "${COMPANY}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "DisplayVersion" "${VERSIONMAJOR}.${VERSIONMINOR}.${VERSIONBUILD}"
    WriteRegStr HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "URLInfoAbout" "https://github.com/okwareddevnest/U-Download"
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "EstimatedSize" ${INSTALLSIZE}
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "NoModify" 1
    WriteRegDWORD HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download" "NoRepair" 1
    
SectionEnd

Section "Desktop Shortcut" SecDesktop
    CreateShortcut "$DESKTOP\U-Download.lnk" "$INSTDIR\u-download.exe" "" "$INSTDIR\u-download.exe" 0
SectionEnd

Section "Start Menu Shortcut" SecStartMenu
    CreateDirectory "$SMPROGRAMS\U-Download"
    CreateShortcut "$SMPROGRAMS\U-Download\U-Download.lnk" "$INSTDIR\u-download.exe" "" "$INSTDIR\u-download.exe" 0
    CreateShortcut "$SMPROGRAMS\U-Download\Uninstall.lnk" "$INSTDIR\Uninstall.exe" "" "$INSTDIR\Uninstall.exe" 0
SectionEnd

; Section descriptions
!insertmacro MUI_FUNCTION_DESCRIPTION_BEGIN
    !insertmacro MUI_DESCRIPTION_TEXT ${SecMain} "U-Download application with all dependencies bundled (yt-dlp, aria2c, FFmpeg). This component is required."
    !insertmacro MUI_DESCRIPTION_TEXT ${SecDesktop} "Create a shortcut on the desktop for easy access to U-Download."
    !insertmacro MUI_DESCRIPTION_TEXT ${SecStartMenu} "Create shortcuts in the Start Menu for U-Download and its uninstaller."
!insertmacro MUI_FUNCTION_DESCRIPTION_END

; Uninstaller
Section "Uninstall"
    ; Remove files
    RMDir /r "$INSTDIR"
    
    ; Remove shortcuts
    Delete "$DESKTOP\U-Download.lnk"
    RMDir /r "$SMPROGRAMS\U-Download"
    
    ; Remove registry entries
    DeleteRegKey HKLM "Software\Microsoft\Windows\CurrentVersion\Uninstall\U-Download"
    
    ; Remove from PATH if added (future feature)
    ; EnVar::DeleteValue "PATH" "$INSTDIR"
    
SectionEnd

; Custom functions can be added here
Function .onInstSuccess
    MessageBox MB_OK "U-Download has been installed successfully!$\r$\n$\r$\n✓ All dependencies bundled and ready$\r$\n✓ No additional setup required$\r$\n✓ Start downloading immediately"
FunctionEnd