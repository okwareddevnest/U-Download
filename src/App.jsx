import { useState, useEffect, useRef, useCallback } from "react";
import { getVersion } from "@tauri-apps/api/app";
import { invoke } from "@tauri-apps/api/core";
import { downloadDir, videoDir, join, dirname } from "@tauri-apps/api/path";
import { isPermissionGranted as notifGranted, requestPermission as notifRequest, sendNotification } from "@tauri-apps/plugin-notification";
import { useJobs } from "./hooks/useJobs";
import TrimWorkbench from "./components/TrimWorkbench";
import { formatTime } from "./lib/time";
import soundNotifications from "./SoundNotifications";
import "./App.css";

// yt-dlp supports 1000+ sites; the extractor decides what is downloadable, and
// reports back through the normal job error path. This only rejects input that
// is not a URL at all.
const isValidUrl = (value) => {
  try {
    const parsed = new URL(String(value).trim());
    return parsed.protocol === "http:" || parsed.protocol === "https:";
  } catch {
    return false;
  }
};

const formatSpeed = (bytesPerSec) => {
  if (!bytesPerSec) return null;
  const kb = bytesPerSec / 1024;
  if (kb < 1024) return `${kb.toFixed(0)} KB/s`;
  return `${(kb / 1024).toFixed(1)} MB/s`;
};

function App() {
  const isAndroid = typeof navigator !== 'undefined' && /android/i.test(navigator.userAgent);
  const [url, setUrl] = useState("");
  const [downloadType, setDownloadType] = useState("mp4");
  const [quality, setQuality] = useState("best");
  const [outputFolder, setOutputFolder] = useState("");
  const [isSelectingFolder, setIsSelectingFolder] = useState(false);
  const [isDarkMode, setIsDarkMode] = useState(() => {
    const saved = localStorage.getItem("isDarkMode");
    return saved ? JSON.parse(saved) : false;
  });
  const [appVersion, setAppVersion] = useState("");

  // Trim selection: { start, end } | null. `showTrim` only controls panel
  // visibility, so it can never desync from whether a selection exists.
  const [trim, setTrim] = useState(null);
  const [showTrim, setShowTrim] = useState(false);

  // A trim range is meaningful only for the video it was drawn against.
  // Changing the URL must invalidate it, or the old range is silently applied
  // to a different video.
  useEffect(() => { setTrim(null); }, [url]);

  const handleJobDone = useCallback(({ title, output_path }) => {
    soundNotifications.playDownloadComplete();
    try {
      sendNotification({ title: 'Download Complete', body: title || output_path || 'Your download is ready.' });
    } catch {}
  }, []);

  const { jobs, error: jobsError, enqueue, cancel } = useJobs({ onDone: handleJobDone });

  // Play a failure sound / notification the first time a job transitions into
  // "failed", mirroring the old failed-download handling without popping an
  // alert per job (multiple jobs can fail independently in the queue model).
  const prevStatusesRef = useRef({});
  useEffect(() => {
    jobs.forEach((job) => {
      const { status: jobStatus } = job;
      const prevStatus = prevStatusesRef.current[job.id];
      if (jobStatus === 'failed' && prevStatus !== 'failed') {
        soundNotifications.playDownloadError();
        try {
          sendNotification({ title: 'Download Failed', body: job.error || job.title || job.url });
        } catch {}
      }
      prevStatusesRef.current[job.id] = jobStatus;
    });
  }, [jobs]);

  useEffect(() => {
    if (jobsError) {
      console.error("Failed to load jobs:", jobsError);
    }
  }, [jobsError]);

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

  // Initialize Android defaults: set Videos dir and capture share intents
  useEffect(() => {
    (async () => {
      try {
        // Default output folder
        let vdir = null;
        try { vdir = await videoDir(); } catch (_) {}
        if (!vdir) {
          try { vdir = await invoke('get_android_videos_dir'); } catch (_) {}
        }
        if (!vdir) {
          try {
            const ddir = await downloadDir();
            const parent = await dirname(ddir);
            vdir = await join(parent, 'Movies');
          } catch (_) {}
        }
        if (vdir && !outputFolder) {
          setOutputFolder(vdir);
          localStorage.setItem("outputFolder", vdir);
        }

        // Android Share intent via native bridge (file-based)
        try {
          const shared = await invoke('get_shared_url');
          if (shared && isValidUrl(shared)) {
            setUrl(shared);
          }
        } catch {}

        // Notification permission
        try {
          const granted = await notifGranted();
          if (!granted) await notifRequest();
        } catch {}
      } catch {}
    })();
    // eslint-disable-next-line react-hooks/exhaustive-deps
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

  const selectOutputFolder = async () => {
    if (isAndroid) {
      alert('On Android, U-Download saves to the Videos folder by default.');
      return;
    }
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

  const toggleTrim = () => {
    if (!isValidUrl(url)) {
      alert("Please enter a valid URL first");
      return;
    }
    setShowTrim((prev) => !prev);
  };

  const startDownload = async () => {
    if (!isValidUrl(url)) {
      alert("Please enter a valid URL");
      return;
    }

    let folder = outputFolder;
    if (!folder) {
      if (isAndroid) {
        try {
          const vdir = await videoDir();
          if (vdir) {
            folder = vdir;
            setOutputFolder(vdir);
          }
        } catch {}
      }
      if (!folder) {
        alert("Please select an output folder");
        return;
      }
    }

    // Check if FFmpeg is available when trimming is enabled
    if (trim) {
      try {
        await invoke("check_ffmpeg");
      } catch (error) {
        alert(`FFmpeg (bundled) is required for video trimming but was not found.\n\nThis indicates a damaged or incomplete installation.\nPlease reinstall U-Download or download the full installer.\n\nDetails: ${error}`);
        return;
      }

      if (trim.start !== null && trim.end !== null && trim.start >= trim.end) {
        alert("Start time must be before end time");
        return;
      }
    }

    try {
      await enqueue({
        url,
        format: downloadType === 'mp3'
          ? { mode: 'quick', kind: 'mp3', height: null }
          : { mode: 'quick', kind: 'mp4', height: quality === 'best' ? null : Number(quality) },
        trim,
        outputFolder: folder,
      });
    } catch (error) {
      console.error("Could not queue download:", error);

      let errorMessage = "Could not queue download: " + error;
      if (String(error).includes("FFmpeg")) {
        errorMessage = "Trimming failed. Please ensure FFmpeg is properly installed.\n\nError: " + error;
      } else if (String(error).includes("aria2c")) {
        errorMessage = "Download accelerator failed. The download will continue without acceleration.\n\nError: " + error;
      } else if (String(error).includes("yt-dlp")) {
        errorMessage = "Downloader failed. Please check your internet connection and try again.\n\nError: " + error;
      }

      alert(errorMessage);
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
        ffmpegResult = `\n\n❌ FFmpeg: Bundled binary not found (${error})`;
      }

      alert(`Dependencies Check:\n\n${result}${ffmpegResult}`);
    } catch (error) {
      alert(`Dependencies Check Failed:\n\n${error}`);
    }
  };

  return (
    <div data-theme={isDarkMode ? 'dark' : 'light'} className={`min-h-screen transition-all duration-500 ${
      isDarkMode
        ? 'theme-dark bg-gradient-to-br from-gray-900 via-gray-800 to-gray-900'
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
              Video URL
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
                } ${!isValidUrl(url) && url ? 'border-red-500 animate-pulse' : ''} group-hover:shadow-lg`}
              />
              <div className={`absolute right-4 top-1/2 -translate-y-1/2 transition-all duration-300 ${
                isValidUrl(url) ? 'text-green-500 scale-110' : 'text-gray-400'
              }`}>
                {isValidUrl(url) ? '✅' : '📎'}
              </div>
            </div>
            {!isValidUrl(url) && url && (
              <div className="flex items-center gap-2 mt-2 text-red-500 text-sm animate-slide-in">
                <span>⚠️</span>
                <p>Please enter a valid URL</p>
              </div>
            )}
          </div>

          {/* Video Preview and Trimming */}
          {showTrim && (
            <div className="relative mb-8">
              <TrimWorkbench url={url} onChange={setTrim} />
            </div>
          )}

          {/* Trim Mode Toggle */}
          <div className="relative mb-8">
            <div className="flex items-center justify-between">
              <div className="flex items-center gap-3">
                <button
                  onClick={toggleTrim}
                  className={`px-6 py-3 rounded-xl font-semibold text-lg transition-all duration-300 transform hover:scale-105 ${
                    showTrim
                      ? 'bg-gradient-to-r from-green-500 to-emerald-600 text-white shadow-lg shadow-green-500/25'
                      : 'bg-gradient-to-r from-gray-600 to-gray-700 text-white shadow-lg shadow-gray-600/25 hover:from-gray-700 hover:to-gray-800'
                  }`}
                >
                  {showTrim ? '✂️ Exit Trim Mode' : '✂️ Trim Video'}
                </button>

                {trim && (trim.start !== null || trim.end !== null) && (
                  <div className="flex items-center gap-2 bg-gray-700/50 px-4 py-2 rounded-lg">
                    <span className="text-white text-sm">Trim:</span>
                    {trim.start !== null && (
                      <span className="text-green-400 text-sm">{formatTime(trim.start)}</span>
                    )}
                    <span className="text-white text-sm">-</span>
                    {trim.end !== null && (
                      <span className="text-red-400 text-sm">{formatTime(trim.end)}</span>
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
                  className={`w-full px-4 py-4 pr-10 rounded-2xl border-2 focus:outline-none transition-all duration-300 text-lg cursor-pointer appearance-none ${
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
                  className={`w-full px-4 py-4 pr-10 rounded-2xl border-2 focus:outline-none transition-all duration-300 text-lg cursor-pointer appearance-none ${
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

          {/* Download Queue */}
          {jobs.length > 0 && (
            <div className="relative mb-8">
              <label className={`block text-sm font-semibold mb-3 flex items-center gap-2 ${isDarkMode ? 'text-gray-200' : 'text-gray-800'}`}>
                <span className="text-blue-500">📥</span>
                Downloads
              </label>
              <div className="space-y-3">
                {jobs.map((job) => {
                  const { status: jobStatus } = job;
                  const pct = Math.round(job.progress?.percentage ?? 0);
                  const speed = formatSpeed(job.progress?.speed_bytes_per_sec);
                  const eta = job.progress?.eta_seconds != null ? formatTime(job.progress.eta_seconds) : null;
                  const isTerminal = ['done', 'failed', 'cancelled'].includes(jobStatus);
                  const isFailed = jobStatus === 'failed';
                  const isDone = jobStatus === 'done';
                  return (
                    <div
                      key={job.id}
                      className={`p-4 rounded-2xl border-2 backdrop-blur-sm ${
                        isFailed
                          ? (isDarkMode ? 'bg-red-900/30 border-red-500/50' : 'bg-red-50/80 border-red-300/50')
                          : isDone
                          ? (isDarkMode ? 'bg-green-900/30 border-green-500/50' : 'bg-green-50/80 border-green-300/50')
                          : (isDarkMode ? 'bg-gray-700/40 border-gray-600/50' : 'bg-white/60 border-gray-200/50')
                      }`}
                    >
                      <div className="flex items-center gap-3">
                        <div className="flex-1 min-w-0">
                          <p className={`text-sm font-semibold truncate ${isDarkMode ? 'text-gray-200' : 'text-gray-800'}`}>
                            {job.title || job.url}
                          </p>
                          <div className={`h-1.5 rounded mt-2 overflow-hidden ${isDarkMode ? 'bg-gray-700' : 'bg-gray-200'}`}>
                            <div
                              className={`h-1.5 rounded transition-all duration-300 ${
                                isFailed ? 'bg-red-500' : isDone ? 'bg-green-500' : 'bg-blue-500'
                              }`}
                              style={{ width: `${pct}%` }}
                            />
                          </div>
                          <div className={`flex items-center gap-3 mt-1 text-xs ${isDarkMode ? 'text-gray-400' : 'text-gray-500'}`}>
                            <span>{pct}%</span>
                            {speed && <span>{speed}</span>}
                            {eta && <span>ETA {eta}</span>}
                          </div>
                          {isFailed && job.error && (
                            <p className="text-xs text-red-400 mt-1 truncate">{job.error}</p>
                          )}
                        </div>
                        <span className={`text-xs font-semibold w-20 text-right capitalize ${isDarkMode ? 'text-gray-400' : 'text-gray-500'}`}>
                          {jobStatus}
                        </span>
                        {!isTerminal && (
                          <button
                            onClick={() => cancel(job.id)}
                            className="text-xs font-semibold text-red-400 hover:text-red-300 whitespace-nowrap"
                          >
                            Cancel
                          </button>
                        )}
                      </div>
                    </div>
                  );
                })}
              </div>
            </div>
          )}

          {/* Download Button */}
          <div className="relative">
            <button
              onClick={startDownload}
              disabled={!isValidUrl(url) || !outputFolder}
              className={`relative w-full py-6 px-8 rounded-2xl font-bold text-xl transition-all duration-300 transform overflow-hidden ${
                !isValidUrl(url) || !outputFolder
                  ? (isDarkMode ? 'bg-gray-700 text-gray-400 cursor-not-allowed' : 'bg-gray-300 text-gray-500 cursor-not-allowed')
                  : `bg-gradient-to-r from-red-500 to-pink-500 text-white hover:from-red-600 hover:to-pink-600 hover:scale-105 hover:shadow-2xl ${
                      isDarkMode ? 'shadow-red-500/25' : 'shadow-red-500/25'
                    } active:scale-95`
              }`}
            >
              {/* Animated background for active state */}
              {isValidUrl(url) && outputFolder && (
                <div className="absolute top-0 left-0 w-full h-full bg-gradient-to-r from-red-400 to-pink-400 opacity-0 hover:opacity-20 transition-opacity duration-300"></div>
              )}

              <div className="relative flex items-center justify-center gap-3">
                <span className="text-2xl">{trim ? '✂️' : '⬇️'}</span>
                <span>{trim ? 'Trim & Download' : 'Start Download'}</span>
              </div>
            </button>

            {/* Download requirements indicator */}
            {(!isValidUrl(url) || !outputFolder) && (
              <div className={`mt-4 p-4 rounded-xl border-2 border-dashed ${
                isDarkMode ? 'border-gray-600 bg-gray-800/30' : 'border-gray-300 bg-gray-50/30'
              }`}>
                <div className="flex flex-col gap-2 text-sm">
                  <div className={`font-semibold flex items-center gap-2 ${isDarkMode ? 'text-gray-300' : 'text-gray-700'}`}>
                    📋 Required to start download:
                  </div>
                  <div className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-xs">
                    <div className={`flex items-center gap-2 ${
                      isValidUrl(url) ? 'text-green-500' : (isDarkMode ? 'text-gray-400' : 'text-gray-500')
                    }`}>
                      {isValidUrl(url) ? '✅' : '⏳'}
                      Valid URL
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
