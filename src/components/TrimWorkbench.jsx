import { useState, useRef, useEffect, useCallback } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { formatTime, parseTimeToSeconds } from '../lib/time';
import './TrimWorkbench.css';

/**
 * Trim workbench built around a real <video> element.
 *
 * The element's own `duration` is the source of truth. The previous
 * implementation derived the scrub bound from yt-dlp metadata, which defaults
 * to 0 whenever a probe fails — collapsing `Math.max(1, duration)` to 1 and
 * leaving a two-position slider. Reading duration from a stream the browser has
 * actually decoded makes that failure mode unreachable.
 */
export default function TrimWorkbench({ url, onChange }) {
  const videoRef = useRef(null);
  const trackRef = useRef(null);

  const [source, setSource] = useState(null);      // { kind, url, title }
  const [phase, setPhase] = useState('idle');      // idle|probing|proxying|ready|error
  const [error, setError] = useState('');
  const [duration, setDuration] = useState(0);     // from the <video> element
  const [current, setCurrent] = useState(0);
  const [inPoint, setInPoint] = useState(null);
  const [outPoint, setOutPoint] = useState(null);
  const [dragging, setDragging] = useState(null);  // 'in' | 'out' | null
  const [startInput, setStartInput] = useState('');
  const [endInput, setEndInput] = useState('');

  const ready = phase === 'ready' && duration > 0;

  // --- source resolution: stream first, proxy on failure -------------------

  const loadProxy = useCallback(async () => {
    setPhase('proxying');
    try {
      const path = await invoke('fetch_preview_proxy', { url });
      const { convertFileSrc } = await import('@tauri-apps/api/core');
      setSource((s) => ({ ...s, kind: 'proxy', url: convertFileSrc(path) }));
      setPhase('idle');
    } catch (e) {
      setError(`Could not prepare a preview: ${e}`);
      setPhase('error');
    }
  }, [url]);

  useEffect(() => {
    if (!url) return;
    let cancelled = false;

    (async () => {
      setPhase('probing');
      setError('');
      try {
        const result = await invoke('resolve_preview', { url });
        if (cancelled) return;
        setSource(result);
        if (result.kind === 'needs_proxy') {
          await loadProxy();
        } else {
          setPhase('idle');
        }
      } catch (e) {
        if (cancelled) return;
        // Errors are shown, not swallowed into console.warn as before.
        setError(String(e));
        setPhase('error');
      }
    })();

    return () => { cancelled = true; };
  }, [url, loadProxy]);

  // --- selection ------------------------------------------------------------

  const clamp = (t) => Math.max(0, Math.min(t, duration));

  // Report an ordered range so an out-point set before the in-point still works.
  const emit = useCallback((a, b) => {
    if (a == null || b == null) { onChange?.(null); return; }
    const [start, end] = a <= b ? [a, b] : [b, a];
    onChange?.(start === end ? null : { start, end });
  }, [onChange]);

  const setIn = (t) => { const v = clamp(t); setInPoint(v); emit(v, outPoint); };
  const setOut = (t) => { const v = clamp(t); setOutPoint(v); emit(inPoint, v); };
  const clearSelection = () => { setInPoint(null); setOutPoint(null); onChange?.(null); };

  useEffect(() => { setStartInput(inPoint != null ? formatTime(inPoint) : ''); }, [inPoint]);
  useEffect(() => { setEndInput(outPoint != null ? formatTime(outPoint) : ''); }, [outPoint]);

  const applyInput = (raw, apply) => {
    const secs = parseTimeToSeconds(raw);
    if (Number.isNaN(secs)) { setError('Invalid time. Use SS, MM:SS or HH:MM:SS'); return; }
    setError('');
    apply(secs);
  };

  // --- drag handling --------------------------------------------------------

  const timeFromClientX = (clientX) => {
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0) return 0;
    return clamp(((clientX - rect.left) / rect.width) * duration);
  };

  useEffect(() => {
    if (!dragging) return;

    const onMove = (e) => {
      const t = timeFromClientX(e.clientX);
      if (dragging === 'in') setIn(t); else setOut(t);
      if (videoRef.current) videoRef.current.currentTime = t;
    };
    const onUp = () => setDragging(null);

    window.addEventListener('pointermove', onMove);
    window.addEventListener('pointerup', onUp);
    return () => {
      window.removeEventListener('pointermove', onMove);
      window.removeEventListener('pointerup', onUp);
    };
  });

  // --- keyboard -------------------------------------------------------------

  useEffect(() => {
    if (!ready) return;
    const onKey = (e) => {
      if (e.target.tagName === 'INPUT') return;
      const v = videoRef.current;
      if (!v) return;

      if (e.key === '[') { setIn(v.currentTime); e.preventDefault(); }
      else if (e.key === ']') { setOut(v.currentTime); e.preventDefault(); }
      else if (e.key === 'ArrowLeft')  { v.currentTime = clamp(v.currentTime - (e.shiftKey ? 0.1 : 1)); e.preventDefault(); }
      else if (e.key === 'ArrowRight') { v.currentTime = clamp(v.currentTime + (e.shiftKey ? 0.1 : 1)); e.preventDefault(); }
    };
    window.addEventListener('keydown', onKey);
    return () => window.removeEventListener('keydown', onKey);
  });

  // --- geometry: guarded against a zero denominator -------------------------

  const pct = (t) => (duration > 0 ? Math.max(0, Math.min(100, (t / duration) * 100)) : 0);
  const selLeft = inPoint != null && outPoint != null ? pct(Math.min(inPoint, outPoint)) : null;
  const selWidth = inPoint != null && outPoint != null ? Math.abs(pct(outPoint) - pct(inPoint)) : 0;
  const clipLength = inPoint != null && outPoint != null ? Math.abs(outPoint - inPoint) : null;

  return (
    <div className="bg-gray-900 rounded-xl overflow-hidden shadow-2xl">
      <div className="bg-gray-800 p-4 border-b border-gray-700">
        <h3 className="text-white font-semibold">Trim</h3>
        <p className="text-gray-400 text-sm truncate">{source?.title || 'Loading…'}</p>
      </div>

      <div className="relative bg-black min-h-[16rem] flex items-center justify-center">
        {phase === 'error' ? (
          <div className="text-center p-6">
            <p className="text-red-400 mb-3">{error}</p>
            <button
              onClick={loadProxy}
              className="bg-gray-700 hover:bg-gray-600 text-white px-4 py-2 rounded-lg text-sm"
            >
              Try downloading a preview instead
            </button>
          </div>
        ) : phase === 'proxying' ? (
          <div className="text-center p-6 text-gray-300">
            <div className="animate-spin rounded-full h-10 w-10 border-b-2 border-red-500 mx-auto mb-3" />
            <p>Preparing preview…</p>
            <p className="text-gray-500 text-sm mt-1">This video has no directly playable stream.</p>
          </div>
        ) : source?.url ? (
          <video
            ref={videoRef}
            src={source.url}
            controls
            className="w-full max-h-96"
            onLoadedMetadata={(e) => {
              // Ground truth. Never yt-dlp's metadata.
              setDuration(e.currentTarget.duration || 0);
              setPhase('ready');
            }}
            onTimeUpdate={(e) => setCurrent(e.currentTarget.currentTime)}
            onError={() => {
              if (source.kind !== 'proxy') loadProxy();
              else { setError('Preview could not be played.'); setPhase('error'); }
            }}
          />
        ) : (
          <div className="text-gray-400 p-6">Loading preview…</div>
        )}
      </div>

      <div className="bg-gray-800 p-4">
        <div
          ref={trackRef}
          className={`relative h-8 bg-gray-700 rounded-lg ${ready ? 'cursor-pointer' : 'opacity-50 cursor-not-allowed'}`}
          onPointerDown={(e) => {
            if (!ready) return;
            const t = timeFromClientX(e.clientX);
            if (videoRef.current) videoRef.current.currentTime = t;
          }}
        >
          {selLeft != null && (
            <div
              className="absolute top-0 h-8 bg-green-500/30 border-x-2 border-green-400 pointer-events-none"
              style={{ left: `${selLeft}%`, width: `${selWidth}%` }}
            />
          )}

          <div className="absolute top-0 h-8 w-0.5 bg-white pointer-events-none" style={{ left: `${pct(current)}%` }} />

          {ready && inPoint != null && (
            <div
              role="slider" aria-label="Trim start" tabIndex={0}
              aria-valuemin={0} aria-valuemax={duration} aria-valuenow={inPoint}
              onPointerDown={(e) => { e.stopPropagation(); setDragging('in'); }}
              className="absolute -top-1 h-10 w-3 -ml-1.5 bg-green-400 rounded cursor-ew-resize"
              style={{ left: `${pct(inPoint)}%` }}
            />
          )}
          {ready && outPoint != null && (
            <div
              role="slider" aria-label="Trim end" tabIndex={0}
              aria-valuemin={0} aria-valuemax={duration} aria-valuenow={outPoint}
              onPointerDown={(e) => { e.stopPropagation(); setDragging('out'); }}
              className="absolute -top-1 h-10 w-3 -ml-1.5 bg-red-400 rounded cursor-ew-resize"
              style={{ left: `${pct(outPoint)}%` }}
            />
          )}
        </div>

        <div className="flex justify-between text-sm text-gray-400 mt-2">
          <span>{formatTime(current)}</span>
          <span>{ready ? formatTime(duration) : '—:—'}</span>
        </div>

        <div className="flex flex-wrap items-center gap-2 mt-4">
          <button disabled={!ready} onClick={() => setIn(videoRef.current?.currentTime ?? 0)}
            className="bg-green-600 hover:bg-green-500 disabled:opacity-40 text-white px-4 py-2 rounded-lg text-sm">
            Set Start <kbd className="ml-1 opacity-70">[</kbd>
          </button>
          <button disabled={!ready} onClick={() => setOut(videoRef.current?.currentTime ?? 0)}
            className="bg-red-600 hover:bg-red-500 disabled:opacity-40 text-white px-4 py-2 rounded-lg text-sm">
            Set End <kbd className="ml-1 opacity-70">]</kbd>
          </button>
          <button disabled={!ready} onClick={clearSelection}
            className="bg-gray-600 hover:bg-gray-500 disabled:opacity-40 text-white px-4 py-2 rounded-lg text-sm">
            Clear
          </button>
          {clipLength != null && (
            <span className="text-blue-300 text-sm ml-auto">Clip length: {formatTime(clipLength)}</span>
          )}
        </div>

        <div className="grid grid-cols-1 md:grid-cols-2 gap-4 mt-4">
          <div>
            <label className="block text-xs text-gray-400 mb-1">Start (SS, MM:SS or HH:MM:SS)</label>
            <input
              type="text" value={startInput} disabled={!ready}
              onChange={(e) => setStartInput(e.target.value)}
              onBlur={() => applyInput(startInput, setIn)}
              onKeyDown={(e) => { if (e.key === 'Enter') applyInput(startInput, setIn); }}
              className="w-full bg-gray-700 text-white px-3 py-2 rounded-lg border border-gray-600 focus:border-green-500 outline-none disabled:opacity-40"
            />
          </div>
          <div>
            <label className="block text-xs text-gray-400 mb-1">End (SS, MM:SS or HH:MM:SS)</label>
            <input
              type="text" value={endInput} disabled={!ready}
              onChange={(e) => setEndInput(e.target.value)}
              onBlur={() => applyInput(endInput, setOut)}
              onKeyDown={(e) => { if (e.key === 'Enter') applyInput(endInput, setOut); }}
              className="w-full bg-gray-700 text-white px-3 py-2 rounded-lg border border-gray-600 focus:border-red-500 outline-none disabled:opacity-40"
            />
          </div>
        </div>

        {error && phase !== 'error' && <p className="text-red-400 text-xs mt-2">{error}</p>}
      </div>
    </div>
  );
}
