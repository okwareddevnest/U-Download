import { useState, useEffect } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import VideoPreview from "./VideoPreview";
import "./App.css";

function App() {
  const [url, setUrl] = useState("");
  const [downloadType, setDownloadType] = useState("mp4");
  const [quality, setQuality] = useState("best");
  const [outputFolder, setOutputFolder] = useState("");
  const [progress, setProgress] = useState(0);
  const [speed, setSpeed] = useState("");
  const [eta, setEta] = useState("");
  const [status, setStatus] = useState("idle");
  const [isSelectingFolder, setIsSelectingFolder] = useState(false);
  const [isDarkMode, setIsDarkMode] = useState(() => {
    const saved = localStorage.getItem("isDarkMode");
    return saved ? JSON.parse(saved) : false;
  });
  const [appVersion, setAppVersion] = useState("");

  // Video trimming state
  const [showVideoPreview, setShowVideoPreview] = useState(false);
  const [trimStartTime, setTrimStartTime] = useState(null);
  const [trimEndTime, setTrimEndTime] = useState(null);
  const [isTrimMode, setIsTrimMode] = useState(false);

  useEffect(() => {
    localStorage.setItem("isDarkMode", JSON.stringify(isDarkMode));
  }, [isDarkMode]);

  // Load app version from Tauri (fallback to dev if unavailable)
  useEffect(() => {
    (async () => {
      try {
        const v = await getVersion();
        setAppVersion(v);
      } catch (e) {
        // Fallback for non-tauri contexts
        try {
          // Optional: embed package.json version via Vite define if present
          // eslint-disable-next-line no-undef
          const envVersion = import.meta?.env?.VITE_APP_VERSION;
          setAppVersion(envVersion || "dev");
        } catch (_) {
          setAppVersion("dev");
        }
      }
    })();
  }, []);

  useEffect(() => {
    const savedFolder = localStorage.getItem("outputFolder");
    const savedType = localStorage.getItem("downloadType");
    const savedQuality = localStorage.getItem("quality");
    
    if (savedFolder) setOutputFolder(savedFolder);
    if (savedType) setDownloadType(savedType);
    if (savedQuality) setQuality(savedQuality);
  }, []);

  useEffect(() => {
    localStorage.setItem("outputFolder", outputFolder);
    localStorage.setItem("downloadType", downloadType);
    localStorage.setItem("quality", quality);
  }, [outputFolder, downloadType, quality]);

  useEffect(() => {
    const setupListeners = async () => {
      const progressUnlisten = await listen("download-progress", (event) => {
        const progressData = event.payload;
        setProgress(progressData.percentage);
        setSpeed(progressData.speed);
        setEta(progressData.eta);
        setStatus(progressData.status);
      });

      const errorUnlisten = await listen("download-error", (event) => {
        console.error("Download error:", event.payload);
        alert(`Download Error: ${event.payload}`);
        setStatus("error");
        setProgress(0);
        setSpeed("");
        setEta("");
      });

      return () => {
        progressUnlisten();
        errorUnlisten();
      };
    };

    setupListeners();
  }, []);

  const isValidYouTubeUrl = (url) => {
    const youtubeRegex = /^(https?\:\/\/)?(www\.)?(youtube\.com|youtu\.be|m\.youtube\.com)\/.+/;
    return youtubeRegex.test(url);
  };

  const selectOutputFolder = async () => {
    setIsSelectingFolder(true);
    try {
      const folder = await invoke("select_output_folder");
      if (folder && folder.length > 0) {
        setOutputFolder(folder);
        console.log("Selected folder:", folder);
      }
    } catch (error) {
      console.error("Failed to select folder:", error);
      // Show user-friendly error message
      if (error.includes("timeout")) {
        alert("Dialog timed out. Please try again.");
      } else if (error.includes("No folder selected")) {
        console.log("User cancelled folder selection");
      } else {
        alert("Failed to open folder dialog. Please try again.");
      }
    } finally {
      setIsSelectingFolder(false);
    }
  };

  const handleTimeSelect = (action, time) => {
    switch (action) {
      case 'start':
        setTrimStartTime(time);
        break;
      case 'end':
        setTrimEndTime(time);
        break;
      case 'clear':
        setTrimStartTime(null);
        setTrimEndTime(null);
        break;
      default:
        break;
    }
  };

  const toggleTrimMode = () => {
    if (!isValidYouTubeUrl(url)) {
      alert("Please enter a valid YouTube URL first");
      return;
    }
    setIsTrimMode(!isTrimMode);
    setShowVideoPreview(!showVideoPreview);
  };

  const startDownload = async () => {
    if (!isValidYouTubeUrl(url)) {
      alert("Please enter a valid YouTube URL");
      return;
    }
    if (!outputFolder) {
      alert("Please select an output folder");
      return;
    }

    // Check if FFmpeg is available when trimming is enabled
    if (isTrimMode && (trimStartTime !== null || trimEndTime !== null)) {
      try {
        await invoke("check_ffmpeg");
      } catch (error) {
        alert(`FFmpeg is required for video trimming but is not installed.\n\nPlease install FFmpeg:\n• Ubuntu/Debian: sudo apt install ffmpeg\n• macOS: brew install ffmpeg\n• Windows: Download from ffmpeg.org\n\nError: ${error}`);
        return;
      }

      // Validate trim times
      if (trimStartTime !== null && trimEndTime !== null && trimStartTime >= trimEndTime) {
        alert("Start time must be before end time");
        return;
      }
    }

    setStatus("downloading");
    setProgress(0);
    setSpeed("");
    setEta("");

    try {
      await invoke("start_download", {
        url,
        downloadType,
        quality,
        outputFolder,
        startTime: trimStartTime,
        endTime: trimEndTime
      });
    } catch (error) {
      console.error("Download failed:", error);

      // Provide more specific error messages for trimming operations
      let errorMessage = "Download failed: " + error;

      if (error.includes("FFmpeg")) {
        errorMessage = "Trimming failed. Please ensure FFmpeg is properly installed.\n\nError: " + error;
      } else if (error.includes("aria2c")) {
        errorMessage = "Download accelerator failed. The download will continue without acceleration.\n\nError: " + error;
      } else if (error.includes("yt-dlp")) {
        errorMessage = "YouTube downloader failed. Please check your internet connection and try again.\n\nError: " + error;
      }

      alert(errorMessage);
      setStatus("error");
    }
  };

  const toggleTheme = () => {
    setIsDarkMode(!isDarkMode);
  };

  const testDependencies = async () => {
    try {
      const result = await invoke("test_dependencies");

      // Also check FFmpeg
      let ffmpegResult = "";
      try {
        const ffmpeg = await invoke("check_ffmpeg");
        ffmpegResult = `\n\n${ffmpeg}`;
      } catch (error) {
        ffmpegResult = `\n\n❌ FFmpeg: Not found (${error})`;
      }

      alert(`Dependencies Check:\n\n${result}${ffmpegResult}`);
    } catch (error) {
      alert(`Dependencies Check Failed:\n\n${error}`);
    }
  };

  return (
    <div className={`min-h-screen transition-all duration-500 ${
      isDarkMode 
        ? 'bg-gradient-to-br from-gray-900 via-gray-800 to-gray-900' 
        : 'bg-gradient-to-br from-blue-50 via-white to-purple-50'
    }`}>
      <div className="container mx-auto px-6 py-8 max-w-5xl">
        {/* Header */}
        <div className="flex justify-between items-center mb-12">
          <div className="flex items-center gap-4">
            <div className="relative group">
              <div className={`absolute -inset-2 rounded-xl blur opacity-20 group-hover:opacity-40 transition duration-700 ${
                isDarkMode ? 'bg-gradient-to-r from-red-600 to-pink-600' : 'bg-gradient-to-r from-red-500 to-pink-500'
              }`}></div>
              <div className={`relative w-14 h-14 rounded-xl p-1 border-2 transition-all duration-300 ${
                isDarkMode 
                  ? 'bg-gray-800/50 border-gray-700/50 group-hover:border-red-500/50' 
                  : 'bg-white/80 border-gray-200/50 group-hover:border-red-500/50'
              }`}>
                <img 
                  src="/logo.png" 
                  alt="U-Download Logo" 
                  className="w-full h-full object-contain group-hover:scale-105 transition-transform duration-300"
                />
              </div>
            </div>
            <div>
              <h1 className={`text-4xl font-bold bg-gradient-to-r ${
                isDarkMode 
                  ? 'from-white via-gray-200 to-gray-400 text-transparent bg-clip-text' 
                  : 'from-gray-800 via-gray-900 to-black text-transparent bg-clip-text'
              }`}>
                U-Download
              </h1>
              <p className={`text-sm font-medium mt-1 ${
                isDarkMode ? 'text-gray-400' : 'text-gray-600'
              }`}>
                Fast & Beautiful YouTube Downloader
              </p>
            </div>
          </div>
          <div className="flex items-center gap-3">
            <button
              onClick={testDependencies}
              className={`px-3 py-2 rounded-full text-xs font-semibold transition-colors hover:scale-105 ${
                isDarkMode 
                  ? 'bg-blue-900/30 text-blue-400 border border-blue-400/30 hover:bg-blue-800/40' 
                  : 'bg-blue-100 text-blue-700 border border-blue-200 hover:bg-blue-200'
              }`}
            >
            </button>
            <div className={`px-3 py-2 rounded-full text-xs font-semibold ${
              isDarkMode 
                ? 'bg-green-900/30 text-green-400 border border-green-400/30' 
                : 'bg-green-100 text-green-700 border border-green-200'
            }`}>
              v {appVersion || 'dev'}
            </div>
            <button
              onClick={toggleTheme}
              className={`p-3 rounded-full transition-all duration-300 transform hover:scale-110 ${
                isDarkMode 
                  ? 'bg-gradient-to-r from-yellow-400 to-orange-500 text-white shadow-lg shadow-yellow-500/25' 
                  : 'bg-gradient-to-r from-indigo-500 to-purple-600 text-white shadow-lg shadow-indigo-500/25'
              }`}
            >
              {isDarkMode ? '☀️' : '🌙'}
            </button>
          </div>
        </div>

        {/* Main Card */}
        <div className={`relative p-8 rounded-3xl backdrop-blur-sm border transition-all duration-500 ${
          isDarkMode 
            ? 'bg-gray-800/70 border-gray-700/50 shadow-2xl shadow-gray-900/50' 
            : 'bg-white/70 border-gray-200/50 shadow-2xl shadow-gray-900/10'
        }`}>
          {/* Animated background decoration */}
          <div className={`absolute top-0 left-0 w-full h-full rounded-3xl opacity-5 ${
            isDarkMode ? 'bg-gradient-to-br from-blue-500 to-purple-600' : 'bg-gradient-to-br from-blue-400 to-purple-500'
          }`}></div>

          {/* URL Input */}
          <div className="relative mb-8">
            <label className={`block text-sm font-semibold mb-3 flex items-center gap-2 ${isDarkMode ? 'text-gray-200' : 'text-gray-800'}`}>
              <span className="text-red-500">🔗</span>
              YouTube URL
            </label>
            <div className="relative group">
              <input
                type="url"
                value={url}
                onChange={(e) => setUrl(e.target.value)}
                placeholder="https://www.youtube.com/watch?v=dQw4w9WgXcQ"
                className={`w-full px-4 py-4 rounded-2xl border-2 focus:outline-none transition-all duration-300 text-lg ${
                  isDarkMode 
                    ? 'bg-gray-700/50 border-gray-600/50 text-white placeholder-gray-400 focus:border-red-500/50 focus:bg-gray-700' 
                    : 'bg-white/50 border-gray-300/50 text-gray-900 placeholder-gray-500 focus:border-red-500/50 focus:bg-white'
                } ${!isValidYouTubeUrl(url) && url ? 'border-red-500 animate-pulse' : ''} group-hover:shadow-lg`}
              />
              <div className={`absolute right-4 top-1/2 -translate-y-1/2 transition-all duration-300 ${
                isValidYouTubeUrl(url) ? 'text-green-500 scale-110' : 'text-gray-400'
              }`}>
                {isValidYouTubeUrl(url) ? '✅' : '📎'}
              </div>
            </div>
            {!isValidYouTubeUrl(url) && url && (
              <div className="flex items-center gap-2 mt-2 text-red-500 text-sm animate-slide-in">
                <span>⚠️</span>
                <p>Please enter a valid YouTube URL</p>
              </div>
            )}
          </div>

          {/* Video Preview and Trimming */}
          {isTrimMode && (
            <div className="relative mb-8">
              <VideoPreview
                url={url}
                onTimeSelect={handleTimeSelect}
                isVisible={showVideoPreview}
              />
            </div>
          )}

          {/* Trim Mode Toggle */}
          <div className="relative mb-8">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <button
                  onClick={toggleTrimMode}
                  className={`px-6 py-3 rounded-xl font-semibold text-lg transition-all duration-300 transform hover:scale-105 ${
                    isTrimMode
                      ? 'bg-gradient-to-r from-green-500 to-emerald-600 text-white shadow-lg shadow-green-500/25'
                      : 'bg-gradient-to-r from-gray-600 to-gray-700 text-white shadow-lg shadow-gray-600/25 hover:from-gray-700 hover:to-gray-800'
                  }`}
                >
                  {isTrimMode ? '✂️ Exit Trim Mode' : '✂️ Trim Video'}
                </button>

                {isTrimMode && (trimStartTime !== null || trimEndTime !== null) && (
                  <div className="flex items-center gap-2 bg-gray-700/50 px-4 py-2 rounded-lg">
                    <span className="text-white text-sm">Trim:</span>
                    {trimStartTime !== null && (
                      <span className="text-green-400 text-sm">
                        {Math.floor(trimStartTime / 60)}:{Math.floor(trimStartTime % 60).toString().padStart(2, '0')}
                      </span>
                    )}
                    <span className="text-white text-sm">-</span>
                    {trimEndTime !== null && (
                      <span className="text-red-400 text-sm">
                        {Math.floor(trimEndTime / 60)}:{Math.floor(trimEndTime % 60).toString().padStart(2, '0')}
                      </span>
                    )}
                  </div>
                )}
              </div>
            </div>
          </div>

          {/* Download Options */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-8 mb-8">
            {/* Download Type */}
            <div className="relative">
              <label className={`block text-sm font-semibold mb-3 flex items-center gap-2 ${isDarkMode ? 'text-gray-200' : 'text-gray-800'}`}>
                <span className="text-blue-500">🎬</span>
                Download Format
              </label>
              <div className="relative group">
                <select
                  value={downloadType}
                  onChange={(e) => setDownloadType(e.target.value)}
                  className={`w-full px-4 py-4 rounded-2xl border-2 focus:outline-none transition-all duration-300 text-lg cursor-pointer ${
                    isDarkMode 
                      ? 'bg-gray-700/50 border-gray-600/50 text-white focus:border-blue-500/50 focus:bg-gray-700' 
                      : 'bg-white/50 border-gray-300/50 text-gray-900 focus:border-blue-500/50 focus:bg-white'
                  } group-hover:shadow-lg`}
                >
                  <option value="mp4">🎥 MP4 (Video)</option>
                  <option value="mp3">🎵 MP3 (Audio Only)</option>
                </select>
                <div className="absolute right-4 top-1/2 -translate-y-1/2 text-gray-400 pointer-events-none">
                  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                  </svg>
                </div>
              </div>
            </div>

            {/* Quality */}
            <div className="relative">
              <label className={`block text-sm font-semibold mb-3 flex items-center gap-2 ${isDarkMode ? 'text-gray-200' : 'text-gray-800'}`}>
                <span className="text-green-500">⚡</span>
                Video Quality
              </label>
              <div className="relative group">
                <select
                  value={quality}
                  onChange={(e) => setQuality(e.target.value)}
                  className={`w-full px-4 py-4 rounded-2xl border-2 focus:outline-none transition-all duration-300 text-lg cursor-pointer ${
                    isDarkMode 
                      ? 'bg-gray-700/50 border-gray-600/50 text-white focus:border-green-500/50 focus:bg-gray-700' 
                      : 'bg-white/50 border-gray-300/50 text-gray-900 focus:border-green-500/50 focus:bg-white'
                  } group-hover:shadow-lg`}
                >
                  <option value="360">📱 360p (Mobile)</option>
                  <option value="480">💻 480p (Standard)</option>
                  <option value="720">🖥️ 720p (HD)</option>
                  <option value="1080">🎯 1080p (Full HD)</option>
                  <option value="best">✨ Best Available</option>
                </select>
                <div className="absolute right-4 top-1/2 -translate-y-1/2 text-gray-400 pointer-events-none">
                  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                    <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 9l-7 7-7-7" />
                  </svg>
                </div>
              </div>
            </div>
          </div>

          {/* Output Folder */}
          <div className="relative mb-8">
            <label className={`block text-sm font-semibold mb-3 flex items-center gap-2 ${isDarkMode ? 'text-gray-200' : 'text-gray-800'}`}>
              <span className="text-purple-500">📁</span>
              Output Folder
            </label>
            <div className="flex gap-3">
              <div className="flex-1 relative group">
                <input
                  type="text"
                  value={outputFolder || "No folder selected"}
                  readOnly
                  className={`w-full px-4 py-4 rounded-2xl border-2 focus:outline-none transition-all duration-300 text-lg cursor-pointer ${
                    isDarkMode 
                      ? 'bg-gray-700/50 border-gray-600/50 text-white' 
                      : 'bg-white/50 border-gray-300/50 text-gray-900'
                  } ${!outputFolder ? 'italic text-gray-500' : ''} group-hover:shadow-lg`}
                />
                <div className="absolute right-4 top-1/2 -translate-y-1/2 text-gray-400">
                  📂
                </div>
              </div>
              <button
                type="button"
                onClick={selectOutputFolder}
                disabled={isSelectingFolder}
                className={`px-8 py-4 rounded-2xl font-semibold text-lg transition-all duration-300 transform hover:scale-105 hover:shadow-xl disabled:opacity-50 disabled:cursor-not-allowed disabled:hover:scale-100 ${
                  isDarkMode
                    ? 'bg-gradient-to-r from-purple-600 to-pink-600 text-white shadow-lg shadow-purple-500/25 hover:from-purple-700 hover:to-pink-700'
                    : 'bg-gradient-to-r from-purple-500 to-pink-500 text-white shadow-lg shadow-purple-500/25 hover:from-purple-600 hover:to-pink-600'
                }`}
              >
                {isSelectingFolder ? (
                  <div className="flex items-center gap-2">
                    <div className="animate-spin rounded-full h-4 w-4 border-b-2 border-current"></div>
                    <span>Opening...</span>
                  </div>
                ) : (
                  'Browse'
                )}
              </button>
            </div>
          </div>

          {/* Progress Section */}
          {status !== "idle" && (
            <div className="relative mb-8">
              <div className={`p-6 rounded-2xl border-2 transition-all duration-500 ${
                status === "downloading" 
                  ? (isDarkMode ? 'bg-blue-900/30 border-blue-500/50' : 'bg-blue-50/80 border-blue-300/50')
                  : status === "completed"
                  ? (isDarkMode ? 'bg-green-900/30 border-green-500/50' : 'bg-green-50/80 border-green-300/50')
                  : (isDarkMode ? 'bg-red-900/30 border-red-500/50' : 'bg-red-50/80 border-red-300/50')
              }`}>
                <div className="flex justify-between items-center mb-4">
                  <div className="flex items-center gap-2">
                    <span className={`text-lg font-semibold ${isDarkMode ? 'text-gray-200' : 'text-gray-800'}`}>
                      {status === "downloading" ? "🚀 Downloading..." : status === "completed" ? "✅ Complete!" : "❌ Error"}
                    </span>
                  </div>
                  <div className={`text-2xl font-bold ${
                    status === "downloading" ? 'text-blue-500' : 
                    status === "completed" ? 'text-green-500' : 'text-red-500'
                  }`}>
                    {Math.round(progress)}%
                  </div>
                </div>
                
                {/* Animated Progress Bar */}
                <div className={`w-full h-4 rounded-full overflow-hidden ${
                  isDarkMode ? 'bg-gray-700/50' : 'bg-gray-200/50'
                }`}>
                  <div 
                    className={`h-full rounded-full transition-all duration-500 relative overflow-hidden ${
                      status === "downloading" ? 'bg-gradient-to-r from-blue-500 to-cyan-500' :
                      status === "completed" ? 'bg-gradient-to-r from-green-500 to-emerald-500' :
                      'bg-gradient-to-r from-red-500 to-pink-500'
                    }`}
                    style={{ width: `${progress}%` }}
                  >
                    {status === "downloading" && (
                      <div className="absolute top-0 left-0 w-full h-full bg-gradient-to-r from-transparent via-white/20 to-transparent animate-pulse"></div>
                    )}
                  </div>
                </div>
                
                {/* Speed and ETA */}
                {(speed || eta) && (
                  <div className="flex justify-between items-center mt-4 text-sm">
                    <div className={`flex items-center gap-2 ${isDarkMode ? 'text-gray-300' : 'text-gray-600'}`}>
                      <span>⚡</span>
                      <span className="font-medium">{speed || 'Calculating...'}</span>
                    </div>
                    <div className={`flex items-center gap-2 ${isDarkMode ? 'text-gray-300' : 'text-gray-600'}`}>
                      <span>⏱️</span>
                      <span className="font-medium">ETA: {eta || '--:--'}</span>
                    </div>
                  </div>
                )}
              </div>
            </div>
          )}

          {/* Download Button */}
          <div className="relative">
            <button
              onClick={startDownload}
              disabled={status === "downloading" || !isValidYouTubeUrl(url) || !outputFolder}
              className={`relative w-full py-6 px-8 rounded-2xl font-bold text-xl transition-all duration-300 transform overflow-hidden ${
                status === "downloading" || !isValidYouTubeUrl(url) || !outputFolder
                  ? (isDarkMode ? 'bg-gray-700 text-gray-400 cursor-not-allowed' : 'bg-gray-300 text-gray-500 cursor-not-allowed')
                  : `bg-gradient-to-r from-red-500 to-pink-500 text-white hover:from-red-600 hover:to-pink-600 hover:scale-105 hover:shadow-2xl ${
                      isDarkMode ? 'shadow-red-500/25' : 'shadow-red-500/25'
                    } active:scale-95`
              }`}
            >
              {/* Animated background for active state */}
              {!(status === "downloading" || !isValidYouTubeUrl(url) || !outputFolder) && (
                <div className="absolute top-0 left-0 w-full h-full bg-gradient-to-r from-red-400 to-pink-400 opacity-0 hover:opacity-20 transition-opacity duration-300"></div>
              )}
              
              <div className="relative flex items-center justify-center gap-3">
                {status === "downloading" ? (
                  <>
                    <div className="animate-spin rounded-full h-6 w-6 border-b-2 border-current"></div>
                    <span>{isTrimMode ? 'Trimming & Downloading...' : 'Downloading...'}</span>
                  </>
                ) : (
                  <>
                    <span className="text-2xl">{isTrimMode ? '✂️' : '⬇️'}</span>
                    <span>{isTrimMode ? 'Trim & Download' : 'Start Download'}</span>
                  </>
                )}
              </div>
            </button>

            {/* Download requirements indicator */}
            {(!isValidYouTubeUrl(url) || !outputFolder) && (
              <div className={`mt-4 p-4 rounded-xl border-2 border-dashed ${
                isDarkMode ? 'border-gray-600 bg-gray-800/30' : 'border-gray-300 bg-gray-50/30'
              }`}>
                <div className="flex flex-col gap-2 text-sm">
                  <div className={`font-semibold flex items-center gap-2 ${isDarkMode ? 'text-gray-300' : 'text-gray-700'}`}>
                    📋 Required to start download:
                  </div>
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-xs">
                    <div className={`flex items-center gap-2 ${
                      isValidYouTubeUrl(url) ? 'text-green-500' : (isDarkMode ? 'text-gray-400' : 'text-gray-500')
                    }`}>
                      {isValidYouTubeUrl(url) ? '✅' : '⏳'}
                      Valid YouTube URL
                    </div>
                    <div className={`flex items-center gap-2 ${
                      outputFolder ? 'text-green-500' : (isDarkMode ? 'text-gray-400' : 'text-gray-500')
                    }`}>
                      {outputFolder ? '✅' : '⏳'}
                      Output folder selected
                    </div>
                  </div>
                </div>
              </div>
            )}
          </div>
        </div>
      </div>
    </div>
  );
}

export default App;
