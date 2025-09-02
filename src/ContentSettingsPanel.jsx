import React, { useState, useEffect } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

const ContentSettingsPanel = ({ isOpen, onClose }) => {
  const [contentStatus, setContentStatus] = useState(null);
  const [downloadProgress, setDownloadProgress] = useState({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState(null);

  useEffect(() => {
    if (isOpen) {
      loadContentStatus();
    }
  }, [isOpen]);

  useEffect(() => {
    if (!isOpen) return;

    let progressUnlisten, completeUnlisten, errorUnlisten;

    const setupEventListeners = async () => {
      progressUnlisten = await listen('content-download-progress', (event) => {
        setDownloadProgress(prev => ({
          ...prev,
          [event.payload.pack_id]: event.payload
        }));
      });

      completeUnlisten = await listen('content-download-complete', (event) => {
        setDownloadProgress(prev => {
          const updated = { ...prev };
          delete updated[event.payload.pack_id];
          return updated;
        });
        loadContentStatus(); // Refresh status
      });

      errorUnlisten = await listen('content-download-error', (event) => {
        setError(`Download failed: ${event.payload.error_message}`);
      });
    };

    setupEventListeners();

    return () => {
      if (progressUnlisten) progressUnlisten();
      if (completeUnlisten) completeUnlisten();
      if (errorUnlisten) errorUnlisten();
    };
  }, [isOpen]);

  const loadContentStatus = async () => {
    setLoading(true);
    setError(null);
    
    try {
      const status = await invoke('check_content_status');
      setContentStatus(status);
    } catch (err) {
      setError('Failed to load content status: ' + err);
    } finally {
      setLoading(false);
    }
  };

  const handleDownloadPack = async (packId) => {
    try {
      await invoke('download_content_pack', { packId });
    } catch (err) {
      setError('Failed to start download: ' + err);
    }
  };

  const formatBytes = (bytes) => {
    if (!bytes) return '0 B';
    const k = 1024;
    const sizes = ['B', 'KB', 'MB', 'GB'];
    const i = Math.floor(Math.log(bytes) / Math.log(k));
    return parseFloat((bytes / Math.pow(k, i)).toFixed(1)) + ' ' + sizes[i];
  };

  const getPackStatus = (packId) => {
    if (!contentStatus?.installation_status) return 'unknown';
    return contentStatus.installation_status[packId] || 'not_installed';
  };

  const getStatusColor = (status) => {
    switch (status) {
      case 'installed': return 'text-green-600 dark:text-green-400';
      case 'not_installed': return 'text-red-600 dark:text-red-400';
      case 'downloading': return 'text-blue-600 dark:text-blue-400';
      case 'corrupted': return 'text-yellow-600 dark:text-yellow-400';
      default: return 'text-gray-600 dark:text-gray-400';
    }
  };

  const getStatusText = (status) => {
    switch (status) {
      case 'installed': return '✅ Installed';
      case 'not_installed': return '❌ Not Installed';
      case 'downloading': return '⬇️ Downloading';
      case 'corrupted': return '⚠️ Corrupted';
      default: return '❓ Unknown';
    }
  };

  if (!isOpen) return null;

  return (
    <div className="fixed inset-0 bg-black bg-opacity-50 flex items-center justify-center z-50">
      <div className="bg-white dark:bg-gray-800 rounded-lg shadow-2xl max-w-4xl w-full mx-4 p-6 max-h-[90vh] overflow-y-auto">
        {/* Header */}
        <div className="flex items-center justify-between mb-6">
          <div className="flex items-center">
            <div className="w-10 h-10 bg-blue-500 rounded-full flex items-center justify-center mr-3">
              <svg className="w-5 h-5 text-white" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M19 7l-.867 12.142A2 2 0 0116.138 21H7.862a2 2 0 01-1.995-1.858L5 7m5 4v6m4-6v6m1-10V4a1 1 0 00-1-1h-4a1 1 0 00-1 1v3M4 7h16" />
              </svg>
            </div>
            <div>
              <h2 className="text-xl font-bold text-gray-900 dark:text-white">
                Content Management
              </h2>
              <p className="text-sm text-gray-600 dark:text-gray-400">
                Manage downloaded content packs
              </p>
            </div>
          </div>
          <button
            onClick={onClose}
            className="text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
          >
            <svg className="w-6 h-6" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path strokeLinecap="round" strokeLinejoin="round" strokeWidth={2} d="M6 18L18 6M6 6l12 12" />
            </svg>
          </button>
        </div>

        {error && (
          <div className="mb-4 p-3 bg-red-100 dark:bg-red-900 border border-red-400 dark:border-red-600 text-red-700 dark:text-red-300 rounded">
            {error}
          </div>
        )}

        {loading ? (
          <div className="flex items-center justify-center py-8">
            <div className="animate-spin rounded-full h-8 w-8 border-b-2 border-blue-500"></div>
            <span className="ml-3 text-gray-600 dark:text-gray-400">Loading content status...</span>
          </div>
        ) : (
          <div className="space-y-6">
            {/* System Info */}
            <div className="bg-gray-100 dark:bg-gray-700 rounded-lg p-4">
              <h3 className="font-semibold text-gray-900 dark:text-white mb-2">System Information</h3>
              <div className="grid grid-cols-2 gap-4 text-sm">
                <div>
                  <span className="text-gray-600 dark:text-gray-400">Platform:</span>
                  <span className="ml-2 font-medium text-gray-900 dark:text-white">
                    {contentStatus?.current_platform || 'Unknown'}
                  </span>
                </div>
                <div>
                  <span className="text-gray-600 dark:text-gray-400">App Version:</span>
                  <span className="ml-2 font-medium text-gray-900 dark:text-white">
                    {contentStatus?.app_version || 'Unknown'}
                  </span>
                </div>
              </div>
            </div>

            {/* Content Packs */}
            <div>
              <div className="flex items-center justify-between mb-4">
                <h3 className="text-lg font-semibold text-gray-900 dark:text-white">Content Packs</h3>
                <button
                  onClick={loadContentStatus}
                  className="px-3 py-1 bg-blue-500 hover:bg-blue-600 text-white text-sm rounded transition duration-200"
                >
                  Refresh
                </button>
              </div>

              {contentStatus?.compatible_packs?.length > 0 ? (
                <div className="space-y-4">
                  {contentStatus.compatible_packs.map((pack) => {
                    const status = getPackStatus(pack.id);
                    const progress = downloadProgress[pack.id];
                    const platform = pack.platforms?.find(p => p.id === contentStatus.current_platform);

                    return (
                      <div key={pack.id} className="border border-gray-200 dark:border-gray-600 rounded-lg p-4">
                        <div className="flex items-start justify-between mb-3">
                          <div>
                            <h4 className="font-semibold text-gray-900 dark:text-white">{pack.name}</h4>
                            <p className="text-sm text-gray-600 dark:text-gray-400 mt-1">{pack.description}</p>
                            {pack.required && (
                              <span className="inline-block bg-red-100 dark:bg-red-900 text-red-800 dark:text-red-300 text-xs px-2 py-1 rounded mt-2">
                                Required
                              </span>
                            )}
                          </div>
                          <div className="text-right">
                            <div className={`text-sm font-medium ${getStatusColor(status)}`}>
                              {getStatusText(status)}
                            </div>
                            <div className="text-xs text-gray-500 dark:text-gray-400 mt-1">
                              v{pack.version}
                            </div>
                          </div>
                        </div>

                        <div className="grid grid-cols-2 gap-4 text-sm mb-4">
                          <div>
                            <span className="text-gray-600 dark:text-gray-400">Download Size:</span>
                            <span className="ml-2 font-medium text-gray-900 dark:text-white">
                              {platform ? formatBytes(platform.compressed_size) : 'N/A'}
                            </span>
                          </div>
                          <div>
                            <span className="text-gray-600 dark:text-gray-400">Installed Size:</span>
                            <span className="ml-2 font-medium text-gray-900 dark:text-white">
                              {formatBytes(pack.total_size)}
                            </span>
                          </div>
                        </div>

                        {progress && (
                          <div className="mb-4">
                            <div className="flex justify-between text-sm mb-1">
                              <span className="text-gray-600 dark:text-gray-400">
                                {progress.phase || 'Downloading'}...
                              </span>
                              <span className="text-gray-900 dark:text-white">
                                {progress.percentage?.toFixed(1)}%
                              </span>
                            </div>
                            <div className="w-full bg-gray-200 dark:bg-gray-700 rounded-full h-2">
                              <div 
                                className="bg-blue-500 h-2 rounded-full transition-all duration-300"
                                style={{ width: `${progress.percentage || 0}%` }}
                              />
                            </div>
                            {progress.speed_formatted && (
                              <div className="flex justify-between text-xs text-gray-500 dark:text-gray-400 mt-1">
                                <span>{progress.speed_formatted}</span>
                                <span>{progress.eta}</span>
                              </div>
                            )}
                          </div>
                        )}

                        <div className="flex space-x-2">
                          {status === 'not_installed' && !progress && (
                            <button
                              onClick={() => handleDownloadPack(pack.id)}
                              className="px-3 py-2 bg-green-500 hover:bg-green-600 text-white text-sm rounded transition duration-200"
                            >
                              Download
                            </button>
                          )}
                          
                          {progress && (
                            <>
                              <button
                                onClick={() => invoke('pause_content_download', { packId: pack.id })}
                                disabled={progress.status !== 'active'}
                                className="px-3 py-2 bg-yellow-500 hover:bg-yellow-600 disabled:bg-gray-400 text-white text-sm rounded transition duration-200"
                              >
                                Pause
                              </button>
                              <button
                                onClick={() => invoke('cancel_content_download', { packId: pack.id })}
                                className="px-3 py-2 bg-red-500 hover:bg-red-600 text-white text-sm rounded transition duration-200"
                              >
                                Cancel
                              </button>
                            </>
                          )}
                          
                          {status === 'installed' && (
                            <button
                              onClick={() => handleDownloadPack(pack.id)}
                              className="px-3 py-2 bg-blue-500 hover:bg-blue-600 text-white text-sm rounded transition duration-200"
                            >
                              Re-download
                            </button>
                          )}
                        </div>
                      </div>
                    );
                  })}
                </div>
              ) : (
                <div className="text-center py-8 text-gray-500 dark:text-gray-400">
                  No compatible content packs found for your platform.
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
};

export default ContentSettingsPanel;