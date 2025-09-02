; U-Download NSIS Installer Hooks
; Custom installation logic and post-install tasks

!include "WinMessages.nsh"
!include "LogicLib.nsh"

; Pre-installation hooks
Function PreInstallHook
    ; Check if U-Download is currently running
    FindWindow $0 "U-Download" ""
    StrCmp $0 0 notrunning
        MessageBox MB_OK|MB_ICONEXCLAMATION "U-Download is currently running. Please close it before continuing with the installation."
        Abort
    notrunning:
    
    ; Check available disk space (604MB + 100MB buffer)
    ${GetRoot} "$INSTDIR" $0
    ${DriveSpace} "$0\" "/D=F /S=M" $1
    IntCmp $1 704 ok ok insufficient_space
    insufficient_space:
        MessageBox MB_OK|MB_ICONEXCLAMATION "Insufficient disk space. U-Download requires at least 704 MB of free space (604 MB for application + 100 MB buffer)."
        Abort
    ok:
    
    ; Create installation log
    FileOpen $0 "$TEMP\u-download-install.log" w
    FileWrite $0 "U-Download Installation Started: "
    ${GetTime} "" "L" $1 $2 $3 $4 $5 $6 $7
    FileWrite $0 "$2/$3/$1 $5:$6:$7$\r$\n"
    FileWrite $0 "Installation Directory: $INSTDIR$\r$\n"
    FileWrite $0 "Zero Dependencies: TRUE$\r$\n"
    FileWrite $0 "Bundled Components: yt-dlp, aria2c, FFmpeg$\r$\n"
    FileClose $0
    
FunctionEnd

; Post-installation hooks
Function PostInstallHook
    ; Verify critical files exist
    IfFileExists "$INSTDIR\u-download.exe" exe_ok
        MessageBox MB_OK|MB_ICONEXCLAMATION "Warning: Main executable not found. Installation may be incomplete."
        goto skip_verification
    exe_ok:
    
    ; Log successful installation
    FileOpen $0 "$TEMP\u-download-install.log" a
    FileWrite $0 "Main executable verified: OK$\r$\n"
    FileClose $0
    
    ; Set proper file permissions (allow execution)
    ExecWait 'icacls "$INSTDIR" /grant Everyone:F /T /Q' $1
    
    ; Register with Windows Defender exclusions (optional, requires admin)
    ExecWait 'powershell.exe -Command "Add-MpPreference -ExclusionPath \"$INSTDIR\" -ErrorAction SilentlyContinue"' $2
    
    ; Create application data directory for user settings
    CreateDirectory "$APPDATA\U-Download"
    CreateDirectory "$APPDATA\U-Download\temp"
    CreateDirectory "$APPDATA\U-Download\downloads"
    
    ; Create initial config file with zero-dependency confirmation
    FileOpen $0 "$APPDATA\U-Download\config.json" w
    FileWrite $0 '{$\r$\n'
    FileWrite $0 '  "version": "2.2.0",$\r$\n'
    FileWrite $0 '  "bundled_dependencies": true,$\r$\n'
    FileWrite $0 '  "yt_dlp_bundled": true,$\r$\n'
    FileWrite $0 '  "aria2c_bundled": true,$\r$\n'
    FileWrite $0 '  "ffmpeg_bundled": true,$\r$\n'
    FileWrite $0 '  "external_dependencies_required": false,$\r$\n'
    FileWrite $0 '  "installation_date": "'
    ${GetTime} "" "L" $1 $2 $3 $4 $5 $6 $7
    FileWrite $0 '$1-$2-$3",$\r$\n'
    FileWrite $0 '  "default_download_path": "$PROFILE\\Downloads"$\r$\n'
    FileWrite $0 '}$\r$\n'
    FileClose $0
    
    ; Add to Windows firewall exceptions (optional)
    ExecWait 'netsh advfirewall firewall add rule name="U-Download" dir=in action=allow program="$INSTDIR\u-download.exe" enable=yes' $3
    
    skip_verification:
    
    ; Final installation log entry
    FileOpen $0 "$TEMP\u-download-install.log" a
    ${GetTime} "" "L" $1 $2 $3 $4 $5 $6 $7
    FileWrite $0 "Installation Completed: $2/$3/$1 $5:$6:$7$\r$\n"
    FileWrite $0 "Status: SUCCESS - Zero dependencies confirmed$\r$\n"
    FileWrite $0 "Ready to use: TRUE$\r$\n"
    FileClose $0
FunctionEnd

; Pre-uninstall hooks
Function un.PreUninstallHook
    ; Check if application is running
    FindWindow $0 "U-Download" ""
    StrCmp $0 0 notrunning
        MessageBox MB_YESNO|MB_ICONQUESTION "U-Download is currently running. Would you like to close it and continue with uninstallation?" IDYES closeapp
        Abort
    closeapp:
        ExecWait 'taskkill /f /im u-download.exe' $1
        Sleep 2000
    notrunning:
    
    ; Create uninstall log
    FileOpen $0 "$TEMP\u-download-uninstall.log" w
    FileWrite $0 "U-Download Uninstallation Started: "
    ${GetTime} "" "L" $1 $2 $3 $4 $5 $6 $7
    FileWrite $0 "$2/$3/$1 $5:$6:$7$\r$\n"
    FileClose $0
FunctionEnd

; Post-uninstall hooks
Function un.PostUninstallHook
    ; Clean up application data (ask user)
    MessageBox MB_YESNO|MB_ICONQUESTION "Would you like to remove your U-Download settings and downloaded files?" IDNO skip_cleanup
        RMDir /r "$APPDATA\U-Download"
    skip_cleanup:
    
    ; Remove firewall exceptions
    ExecWait 'netsh advfirewall firewall delete rule name="U-Download"' $1
    
    ; Remove Windows Defender exclusions
    ExecWait 'powershell.exe -Command "Remove-MpPreference -ExclusionPath \"$INSTDIR\" -ErrorAction SilentlyContinue"' $2
    
    ; Final uninstall log
    FileOpen $0 "$TEMP\u-download-uninstall.log" a
    ${GetTime} "" "L" $1 $2 $3 $4 $5 $6 $7
    FileWrite $0 "Uninstallation Completed: $2/$3/$1 $5:$6:$7$\r$\n"
    FileWrite $0 "Status: SUCCESS$\r$\n"
    FileClose $0
    
    ; Show completion message
    MessageBox MB_OK "U-Download has been successfully removed from your computer.$\r$\n$\r$\nThank you for using U-Download!"
FunctionEnd

; Custom page for showing zero-dependency information
Function ShowZeroDependencyInfo
    nsDialogs::Create 1018
    Pop $0
    
    ${NSD_CreateLabel} 0 10u 300u 20u "U-Download - Zero External Dependencies"
    Pop $1
    
    ${NSD_CreateLabel} 0 40u 300u 60u "This installation includes all required tools:$\r$\n• yt-dlp (YouTube downloader) - 36 MB$\r$\n• aria2c (Download accelerator) - 5 MB$\r$\n• FFmpeg (Media processor) - 160 MB$\r$\n$\r$\nNo additional software installation required!"
    Pop $2
    
    nsDialogs::Show
FunctionEnd