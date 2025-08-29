import { useState, useRef, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import './VideoPreview.css';

const VideoPreview = ({ url, onTimeSelect, isVisible }) => {
  const [currentTime, setCurrentTime] = useState(0);
  const [duration, setDuration] = useState(0);
  const [isLoading, setIsLoading] = useState(false);
  const [videoData, setVideoData] = useState(null);
  const [startTime, setStartTime] = useState(null);
  const [endTime, setEndTime] = useState(null);
  const [startInput, setStartInput] = useState('');
  const [endInput, setEndInput] = useState('');
  const [startError, setStartError] = useState('');
  const [endError, setEndError] = useState('');

  useEffect(() => {
    if (url && isVisible) {
      loadVideoData();
    }
  }, [url, isVisible]);

  const loadVideoData = async () => {
    if (!url) return;

    setIsLoading(true);
    try {
      const metadata = await invoke('get_video_metadata', { url });
      setVideoData(metadata);
      setDuration(metadata.duration);
    } catch (error) {
      console.error('Failed to load video metadata:', error);

      // Show user-friendly error message
      if (onTimeSelect) {
        // You could emit an error event here if needed
        console.warn('Video metadata loading failed, trim functionality may be limited');
      }

      // Fallback to basic duration
      setDuration(0);
      setVideoData(null);
    } finally {
      setIsLoading(false);
    }
  };

  // Format seconds -> mm:ss (supports long minutes)
  const formatTime = (time) => {
    const t = Math.max(0, Math.floor(time || 0));
    const minutes = Math.floor(t / 60);
    const seconds = t % 60;
    return `${minutes}:${seconds.toString().padStart(2, '0')}`;
  };

  // Parse HH:MM:SS | MM:SS | SS -> seconds (int)
  const parseTimeToSeconds = (value) => {
    const s = String(value || '').trim();
    if (!s) return NaN;
    const parts = s.split(':').map((p) => p.trim());
    if (parts.some((p) => p === '' || isNaN(Number(p)))) return NaN;
    let total = 0;
    if (parts.length === 1) {
      total = Math.floor(Number(parts[0]));
    } else if (parts.length === 2) {
      const [m, sec] = parts.map((p) => Math.floor(Number(p)));
      total = m * 60 + sec;
    } else if (parts.length === 3) {
      const [h, m, sec] = parts.map((p) => Math.floor(Number(p)));
      total = h * 3600 + m * 60 + sec;
    } else {
      return NaN;
    }
    return total;
  };

  const handleSeek = (e) => {
    const newTime = Math.max(0, Math.min(Number(e.target.value), Math.max(1, Math.floor(duration || 0))));
    setCurrentTime(newTime);
  };

  const setStartTimeAtCurrent = () => {
    setStartTime(currentTime);
    if (onTimeSelect) {
      onTimeSelect('start', currentTime);
    }
  };

  const setEndTimeAtCurrent = () => {
    setEndTime(currentTime);
    if (onTimeSelect) {
      onTimeSelect('end', currentTime);
    }
  };

  const clearTrimSelection = () => {
    setStartTime(null);
    setEndTime(null);
    if (onTimeSelect) {
      onTimeSelect('clear');
    }
  };

  // Sync manual input fields when times are set via buttons or external state
  useEffect(() => {
    setStartInput(startTime != null ? formatTime(startTime) : '');
  }, [startTime]);

  useEffect(() => {
    setEndInput(endTime != null ? formatTime(endTime) : '');
  }, [endTime]);

  const applyStartInput = () => {
    setStartError('');
    const secs = parseTimeToSeconds(startInput);
    if (isNaN(secs)) {
      setStartError('Invalid time. Use SS, MM:SS or HH:MM:SS');
      return;
    }
    const clamped = Math.max(0, Math.min(secs, Math.floor(duration || 0)));
    setStartTime(clamped);
    if (onTimeSelect) onTimeSelect('start', clamped);
  };

  const applyEndInput = () => {
    setEndError('');
    const secs = parseTimeToSeconds(endInput);
    if (isNaN(secs)) {
      setEndError('Invalid time. Use SS, MM:SS or HH:MM:SS');
      return;
    }
    const clamped = Math.max(0, Math.min(secs, Math.floor(duration || 0)));
    setEndTime(clamped);
    if (onTimeSelect) onTimeSelect('end', clamped);
  };

  if (!isVisible) return null;

  return (
    <div className="bg-gray-900 rounded-xl overflow-hidden shadow-2xl">
      {/* Header */}
      <div className="bg-gray-800 p-4 border-b border-gray-700">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="w-10 h-10 bg-red-600 rounded-lg flex items-center justify-center">
              <span className="text-white font-bold text-lg">▶</span>
            </div>
            <div>
              <h3 className="text-white font-semibold">Video Preview</h3>
              <p className="text-gray-400 text-sm">
                {videoData ? videoData.title : 'Loading video info...'}
              </p>
            </div>
          </div>
          {videoData && (
            <div className="text-right">
              <p className="text-white text-sm">{formatTime(duration)}</p>
              <p className="text-gray-400 text-xs">Duration</p>
            </div>
          )}
        </div>
      </div>

      {/* Video Thumbnail/Preview */}
      <div className="relative bg-black">
        {isLoading ? (
          <div className="w-full h-64 flex items-center justify-center bg-gray-900">
            <div className="animate-spin rounded-full h-12 w-12 border-b-2 border-red-500"></div>
          </div>
        ) : videoData?.thumbnail_url ? (
          <div className="relative">
            <img
              src={videoData.thumbnail_url}
              alt="Video thumbnail"
              className="w-full h-64 object-cover"
              onError={(e) => {
                e.target.style.display = 'none';
                e.target.nextSibling.style.display = 'flex';
              }}
            />
            <div className="absolute inset-0 bg-black/50 flex items-center justify-center hidden">
              <div className="text-white text-center">
                <div className="w-16 h-16 bg-gray-700 rounded-full flex items-center justify-center mx-auto mb-2">
                  <svg className="w-8 h-8" fill="currentColor" viewBox="0 0 24 24">
                    <path d="M8 5v14l11-7z"/>
                  </svg>
                </div>
                <p>Preview not available</p>
              </div>
            </div>
          </div>
        ) : (
          <div className="w-full h-64 flex items-center justify-center bg-gray-900">
            <div className="text-white text-center">
              <div className="w-16 h-16 bg-gray-700 rounded-full flex items-center justify-center mx-auto mb-2">
                <svg className="w-8 h-8" fill="currentColor" viewBox="0 0 24 24">
                  <path d="M8 5v14l11-7z"/>
                </svg>
              </div>
              <p>Video preview</p>
              <p className="text-sm text-gray-400 mt-1">Select time range below</p>
            </div>
          </div>
        )}
      </div>

      {/* Controls */}
      <div className="bg-gray-800 p-4">
        {/* Progress Bar (per-second precision) */}
        <div className="mb-4">
          <div className="relative">
            <input
              type="range"
              min="0"
              max={Math.max(1, Math.floor(duration || 0))}
              step={1}
              value={Math.max(0, Math.min(Math.floor(currentTime || 0), Math.max(1, Math.floor(duration || 0))))}
              onChange={handleSeek}
              className="w-full h-2 bg-gray-700 rounded-lg appearance-none cursor-pointer slider"
            />

            {/* Trim markers */}
            {startTime !== null && (
              <div
                className="absolute top-0 h-2 bg-green-500 rounded-l-lg pointer-events-none"
                style={{
                  left: `${(startTime / duration) * 100}%`,
                  width: endTime ? `${((endTime - startTime) / duration) * 100}%` : `${(1 - startTime / duration) * 100}%`
                }}
              />
            )}
          </div>

          {/* Time Display */}
          <div className="flex justify-between items-center mt-2 text-sm text-gray-400">
            <span>{formatTime(currentTime)}</span>
            <span>{formatTime(duration)}</span>
          </div>
        </div>

        {/* Timeline Controls */}
        <div className="flex items-center justify-center gap-4 mb-4">
          <div className="flex items-center gap-2">
            <span className="text-white text-sm font-medium">Current:</span>
            <div className="bg-gray-700 px-3 py-1 rounded-lg">
              <span className="text-white font-mono text-sm">{formatTime(currentTime)}</span>
            </div>
          </div>

          {/* Trim Controls */}
          <div className="flex items-center gap-2">
            <button
              onClick={setStartTimeAtCurrent}
              className="bg-green-600 hover:bg-green-500 text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors flex items-center gap-2"
            >
              <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24">
                <path d="M8 5v14l11-7z"/>
              </svg>
              Set Start
            </button>
            <button
              onClick={setEndTimeAtCurrent}
              className="bg-red-600 hover:bg-red-500 text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors flex items-center gap-2"
            >
              <svg className="w-4 h-4" fill="currentColor" viewBox="0 0 24 24">
                <path d="M6 19h4V5H6v14zm8-14v14h4V5h-4z"/>
              </svg>
              Set End
            </button>
            <button
              onClick={clearTrimSelection}
              className="bg-gray-600 hover:bg-gray-500 text-white px-4 py-2 rounded-lg text-sm font-medium transition-colors"
            >
              Clear
            </button>
          </div>
        </div>

        {/* Manual Start/End Inputs */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mb-4">
          <div>
            <label className="block text-xs text-gray-400 mb-1">Start (SS, MM:SS or HH:MM:SS)</label>
            <div className="flex gap-2">
              <input
                type="text"
                value={startInput}
                onChange={(e) => setStartInput(e.target.value)}
                onBlur={applyStartInput}
                onKeyDown={(e) => { if (e.key === 'Enter') applyStartInput(); }}
                placeholder="0:00"
                className="flex-1 bg-gray-700 text-white px-3 py-2 rounded-lg outline-none border border-gray-600 focus:border-green-500"
              />
              <button
                onClick={applyStartInput}
                className="px-3 py-2 rounded-lg bg-green-600 hover:bg-green-500 text-white text-sm font-medium"
              >
                Apply
              </button>
            </div>
            {startError && <p className="text-red-400 text-xs mt-1">{startError}</p>}
          </div>
          <div>
            <label className="block text-xs text-gray-400 mb-1">End (SS, MM:SS or HH:MM:SS)</label>
            <div className="flex gap-2">
              <input
                type="text"
                value={endInput}
                onChange={(e) => setEndInput(e.target.value)}
                onBlur={applyEndInput}
                onKeyDown={(e) => { if (e.key === 'Enter') applyEndInput(); }}
                placeholder={formatTime(duration)}
                className="flex-1 bg-gray-700 text-white px-3 py-2 rounded-lg outline-none border border-gray-600 focus:border-red-500"
              />
              <button
                onClick={applyEndInput}
                className="px-3 py-2 rounded-lg bg-red-600 hover:bg-red-500 text-white text-sm font-medium"
              >
                Apply
              </button>
            </div>
            {endError && <p className="text-red-400 text-xs mt-1">{endError}</p>}
          </div>
        </div>

        {/* Trim Info */}
        {(startTime !== null || endTime !== null) && (
          <div className="mt-4 p-3 bg-gray-700 rounded-lg">
            <h4 className="text-white font-medium mb-2">Trim Selection</h4>
            <div className="flex items-center gap-4 text-sm text-gray-300">
              {startTime !== null && (
                <div>
                  <span className="text-green-400">Start:</span> {formatTime(startTime)}
                </div>
              )}
              {endTime !== null && (
                <div>
                  <span className="text-red-400">End:</span> {formatTime(endTime)}
                </div>
              )}
              {startTime !== null && endTime !== null && (
                <div>
                  <span className="text-blue-400">Duration:</span> {formatTime(endTime - startTime)}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default VideoPreview;
