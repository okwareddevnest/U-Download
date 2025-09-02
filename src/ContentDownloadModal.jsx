import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const ContentDownloadModal = ({ isOpen, onClose, onComplete }) => {
  const [contentStatus, setContentStatus] = useState(null);
  const [downloadProgress, setDownloadProgress] = useState(null);
  const [isDownloading, setIsDownloading] = useState(false);
  const [error, setError] = useState(null);
  const [userChoice, setUserChoice] = useState(null); // 'download', 'later', 'offline'

  useEffect(() => {
    if (isOpen) {
      checkContentStatus();
    }
  }, [isOpen]);

  useEffect(() => {
    let progressUnlisten, completeUnlisten, errorUnlisten;

    const setupEventListeners = async () => {
      // Listen for download progress
      progressUnlisten = await listen('content-download-progress', (event) => {
        setDownloadProgress(event.payload);
      });

      // Listen for download completion
      completeUnlisten = await listen('content-download-complete', (event) => {
        setIsDownloading(false);
        setDownloadProgress(null);
        onComplete?.();
        onClose?.();
      });

      // Listen for download errors
      errorUnlisten = await listen('content-download-error', (event) => {
        setIsDownloading(false);
        setError(event.payload.error_message || 'Download failed');
      });
    };

    if (isOpen) {
      setupEventListeners();
    }

    return () => {
      if (progressUnlisten) progressUnlisten();
      if (completeUnlisten) completeUnlisten();
      if (errorUnlisten) errorUnlisten();
    };
  }, [isOpen, onComplete, onClose]);

  const checkContentStatus = async () => {
    try {
      const status = await invoke('check_content_status');
      setContentStatus(status);
    } catch (err) {
      console.error('Failed to check content status:', err);
      setError('Failed to check content status: ' + err);
    }
  };

  const startDownload = async () => {
    if (!contentStatus?.compatible_packs?.[0]) return;
    
    const packId = contentStatus.compatible_packs[0].id;
    
    try {
      setIsDownloading(true);
      setError(null);
      await invoke('download_content_pack', { packId });
    } catch (err) {
      console.error('Failed to start download:', err);
      setError('Failed to start download: ' + err);
      setIsDownloading(false);
    }
  };

  const handleDownloadNow = () => {
    setUserChoice('download');
    startDownload();
  };

  const handleDownloadLater = () => {
    setUserChoice('later');
    onClose?.();
  };

  const handleOfflineMode = () => {
    setUserChoice('offline');
    onClose?.();
  };

  const formatBytes = (bytes) => {
    if (!bytes) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  };

  const formatPhase = (phase) => {
    const phases = {
      'preparing': 'Preparing download...',
      'downloading': 'Downloading content...',
      'verifying': 'Verifying checksums...',
      'signaturecheck': 'Verifying signatures...',
      'extracting': 'Extracting files...',
      'installing': 'Installing content...',
      'cleanup': 'Cleaning up...',
      'complete': 'Complete!'
    };
    return phases[phase] || phase;
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-2xl max-w-md w-full mx-4 p-6">
        {/* Header */}
        <div className="flex items-center mb-4">
          <div className="w-12 h-12 bg-blue-500 rounded-full flex items-center justify-center mr-3">
            <svg className="w-6 h-6 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M7 16a4 4 0 01-.88-7.903A5 5 0 1115.9 6L16 6a5 5 0 011 9.9M9 19l3 3m0 0l3-3m-3 3V10" />
            </svg>
          </div>
          <div>
            <h2 className="text-xl font-bold text-gray-900 dark:text-white">
              Core Content Required
            </h2>
            <p className="text-sm text-gray-600 dark:text-gray-400">
              First-time setup
            </p>
          </div>
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-100 dark:bg-red-900 border border-red-400 dark:border-red-600 text-red-700 dark:text-red-300 rounded">
            {error}
          </div>
        )}

        {/* Content */}
        {!isDownloading && !downloadProgress && (
          <div className="space-y-4">
            <p className="text-gray-700 dark:text-gray-300">
              U-Download requires essential binaries (yt-dlp, aria2c, FFmpeg) for full functionality. 
              These have been moved to a separate download to keep the installer small.
            </p>

            {contentStatus && (
              <div className="bg-gray-100 dark:bg-gray-700 rounded-lg p-4">
                <h3 className="font-semibold text-gray-900 dark:text-white mb-2">
                  Download Details
                </h3>
                <div className="space-y-2 text-sm">
                  <div className="flex justify-between">
                    <span className="text-gray-600 dark:text-gray-400">Content Pack:</span>
                    <span className="font-medium text-gray-900 dark:text-white">
                      {contentStatus.compatible_packs?.[0]?.name || 'Core Binaries'}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-600 dark:text-gray-400">Platform:</span>
                    <span className="font-medium text-gray-900 dark:text-white">
                      {contentStatus.current_platform}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-600 dark:text-gray-400">Download Size:</span>
                    <span className="font-medium text-gray-900 dark:text-white">
                      {contentStatus.compatible_packs?.[0]?.platforms?.find(p => 
                        p.id === contentStatus.current_platform)?.compressed_size && 
                       formatBytes(contentStatus.compatible_packs[0].platforms.find(p => 
                        p.id === contentStatus.current_platform).compressed_size)}
                    </span>
                  </div>
                  <div className="flex justify-between">
                    <span className="text-gray-600 dark:text-gray-400">Installed Size:</span>
                    <span className="font-medium text-gray-900 dark:text-white">
                      {formatBytes(contentStatus.compatible_packs?.[0]?.total_size)}
                    </span>
                  </div>
                </div>
              </div>
            )}

            <div className="bg-blue-50 dark:bg-blue-900 rounded-lg p-4">
              <h4 className="font-semibold text-blue-900 dark:text-blue-300 mb-1">
                What's Included
              </h4>
              <ul className="text-sm text-blue-800 dark:text-blue-300 space-y-1">
                <li>• <strong>yt-dlp</strong> - YouTube downloader engine</li>
                <li>• <strong>aria2c</strong> - High-speed download accelerator</li>
                <li>• <strong>FFmpeg</strong> - Video processing and trimming</li>
              </ul>
            </div>

            <div className="flex flex-col space-y-2">
              <button
                onClick={handleDownloadNow}
                className="w-full bg-blue-500 hover:bg-blue-600 text-white font-semibold py-2 px-4 rounded-lg transition duration-200"
              >
                Download Now
              </button>
              
              <div className="flex space-x-2">
                <button
                  onClick={handleDownloadLater}
                  className="flex-1 bg-gray-200 dark:bg-gray-600 hover:bg-gray-300 dark:hover:bg-gray-500 text-gray-800 dark:text-gray-200 font-semibold py-2 px-4 rounded-lg transition duration-200"
                >
                  Download Later
                </button>
                
                <button
                  onClick={handleOfflineMode}
                  className="flex-1 bg-gray-200 dark:bg-gray-600 hover:bg-gray-300 dark:hover:bg-gray-500 text-gray-800 dark:text-gray-200 font-semibold py-2 px-4 rounded-lg transition duration-200"
                >
                  Offline Mode
                </button>
              </div>
            </div>

            <p className="text-xs text-gray-500 dark:text-gray-400 text-center">
              Downloads are resumable and can be cancelled at any time
            </p>
          </div>
        )}

        {/* Download Progress */}
        {(isDownloading || downloadProgress) && (
          <div className="space-y-4">
            <div>
              <div className="flex justify-between items-center mb-2">
                <span className="text-sm font-medium text-gray-700 dark:text-gray-300">
                  {downloadProgress?.phase ? formatPhase(downloadProgress.phase) : 'Starting...'}
                </span>
                <span className="text-sm text-gray-500 dark:text-gray-400">
                  {downloadProgress?.percentage?.toFixed(1) || 0}%
                </span>
              </div>
              
              <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2">
                <div 
                  className="bg-blue-500 h-2 rounded-full transition-all duration-300"
                  style={{ width: `${downloadProgress?.percentage || 0}%` }}
                />
              </div>
            </div>

            {downloadProgress && (
              <div className="space-y-2 text-sm">
                <div className="flex justify-between">
                  <span className="text-gray-600 dark:text-gray-400">Speed:</span>
                  <span className="font-medium text-gray-900 dark:text-white">
                    {downloadProgress.speed_formatted}
                  </span>
                </div>
                
                <div className="flex justify-between">
                  <span className="text-gray-600 dark:text-gray-400">Downloaded:</span>
                  <span className="font-medium text-gray-900 dark:text-white">
                    {formatBytes(downloadProgress.bytes_downloaded)} / {formatBytes(downloadProgress.total_bytes)}
                  </span>
                </div>
                
                {downloadProgress.eta && downloadProgress.eta !== 'Calculating...' && (
                  <div className="flex justify-between">
                    <span className="text-gray-600 dark:text-gray-400">ETA:</span>
                    <span className="font-medium text-gray-900 dark:text-white">
                      {downloadProgress.eta}
                    </span>
                  </div>
                )}
              </div>
            )}

            <div className="flex space-x-2">
              <button
                onClick={() => invoke('pause_content_download', { packId: downloadProgress?.pack_id })}
                disabled={downloadProgress?.status !== 'active'}
                className="flex-1 bg-yellow-500 hover:bg-yellow-600 disabled:bg-gray-400 text-white font-semibold py-2 px-4 rounded-lg transition duration-200"
              >
                Pause
              </button>
              
              <button
                onClick={() => invoke('cancel_content_download', { packId: downloadProgress?.pack_id })}
                className="flex-1 bg-red-500 hover:bg-red-600 text-white font-semibold py-2 px-4 rounded-lg transition duration-200"
              >
                Cancel
              </button>
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default ContentDownloadModal;