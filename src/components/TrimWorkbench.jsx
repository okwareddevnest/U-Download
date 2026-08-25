import { useState, useRef, useEffect, useLayoutEffect, useCallback } from 'react';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { formatTime, parseTimeToSeconds } from '../lib/time';
import './TrimWorkbench.css';

/**
 * Trim workbench built around a real <video> element.
 *
 * The element's own `duration` is the source of truth for playback. The
 * previous implementation derived the scrub bound from yt-dlp metadata, which
 * defaults to 0 whenever a probe fails — collapsing `Math.max(1, duration)` to
 * 1 and leaving a two-position slider. Reading duration from a stream the
 * browser has actually decoded makes that failure mode unreachable.
 *
 * Playback is not, however, the only way a duration can be known, and it is not
 * a precondition for expressing a trim. See `knownDuration` below.
 */
export default function TrimWorkbench({ url, onChange }) {
  const videoRef = useRef(null);
  const trackRef = useRef(null);

  const [source, setSource] = useState(null);      // { kind, url, title, duration }
  const [phase, setPhase] = useState('idle');      // idle|probing|proxying|ready|error
  const [error, setError] = useState('');
  const [duration, setDuration] = useState(0);     // from the <video> element
  const [current, setCurrent] = useState(0);
  const [inPoint, setInPoint] = useState(null);
  const [outPoint, setOutPoint] = useState(null);
  const [dragging, setDragging] = useState(null);  // 'in' | 'out' | null
  const [startInput, setStartInput] = useState('');
  const [endInput, setEndInput] = useState('');

  // The URL the workbench is currently showing, readable from inside an async
  // callback that started under a previous one. A ref rather than the closed-over
  // `url` because the point is to see the *latest* value, not the captured one.
  // Written in a layout effect so it is already current by the time the probe
  // effect below runs for the same render.
  const latestUrlRef = useRef(url);
  useLayoutEffect(() => { latestUrlRef.current = url; }, [url]);

  // --- source resolution: stream first, proxy on failure -------------------

  /**
   * Downloads a low-resolution local copy to scrub against.
   *
   * Every state write is gated on the URL still being the one this call was
   * made for. Without that gate a proxy download for URL A — which can run for
   * minutes — would, on resolving, overwrite the source and duration of URL B
   * that the user had since typed. The header would show B's title, the video
   * element would play A, and a trim drawn against A's timeline would be
   * applied to B: the exact off-target cut this whole feature exists to fix.
   *
   * `targetUrl` is an explicit parameter and not just the closed-over `url` so
   * that every caller — including the retry button, which must run for the
   * URL now on screen — states which video it means.
   */
  const loadProxy = useCallback(async (targetUrl) => {
    if (latestUrlRef.current !== targetUrl) return;
    setPhase('proxying');
    try {
      const path = await invoke('fetch_preview_proxy', { url: targetUrl });
      if (latestUrlRef.current !== targetUrl) return;
      // `has_audio: true` explicitly: the source being replaced may have been a
      // video-only stream, and the proxy yt-dlp just muxed is not silent.
      setSource((s) => ({ ...s, kind: 'proxy', url: convertFileSrc(path), has_audio: true }));
      setPhase('idle');
    } catch (e) {
      if (latestUrlRef.current !== targetUrl) return;
      setError(`Could not prepare a preview: ${e}`);
      setPhase('error');
    }
  }, []);

  useEffect(() => {
    if (!url) return;
    let cancelled = false;

    (async () => {
      setPhase('probing');
      setError('');
      setDuration(0);
      // Cleared together with the duration, and for the same reason. `source`
      // is the *other* place a duration comes from (`probedDuration` below), so
      // leaving the previous video's source in place while a new URL is being
      // probed kept `knownDuration` pointing at the old length — for the one to
      // three seconds a probe takes, typed timestamps were clamped against a
      // video that is no longer on screen.
      setSource(null);
      try {
        const result = await invoke('resolve_preview', { url });
        if (cancelled) return;
        setSource(result);
        if (result.kind === 'needs_proxy') {
          await loadProxy(url);
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

  // --- what is known, and what that permits ---------------------------------

  // A decoded, seekable video. Only this justifies the scrubber and the drag
  // handles, which need frames to land on.
  const playable = phase === 'ready' && Number.isFinite(duration) && duration > 0;

  // A duration from any source. `resolve_preview` reports yt-dlp's metadata
  // duration even when it can offer no playable stream, and that is enough to
  // clamp a typed timestamp against.
  const probedDuration = Number(source?.duration);
  const knownDuration = playable
    ? duration
    : (Number.isFinite(probedDuration) && probedDuration > 0 ? probedDuration : null);

  // Typing timestamps must not require a preview. When both resolution and
  // proxying fail there was previously no way to express a trim at all. A user
  // who knows their timestamps is not blocked by a preview failure.
  //
  // What actually gates the fields is having a bound to clamp against, not the
  // phase: they are enabled as soon as any duration is known, and while a probe
  // is in flight neither source exists (the effect clears both), so they stay
  // disabled until the probe reports — which is the point, since clamping
  // against the previous video's length is worse than refusing input.
  const canEditTimes = knownDuration != null || phase === 'error';

  // --- selection ------------------------------------------------------------

  // Clamped against whatever bound is known. With no bound at all the value is
  // accepted as typed rather than refused: yt-dlp is the final authority on
  // whether the range is valid, and an unclamped range it rejects is a better
  // outcome than an input the user cannot use.
  const clamp = (t) => (knownDuration != null
    ? Math.max(0, Math.min(t, knownDuration))
    : Math.max(0, t));

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
    if (!rect || rect.width === 0 || !playable) return 0;
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
    if (!playable) return;
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

  // --- geometry: guarded against a zero or non-finite denominator -----------

  const pct = (t) => (playable ? Math.max(0, Math.min(100, (t / duration) * 100)) : 0);
  const selLeft = inPoint != null && outPoint != null ? pct(Math.min(inPoint, outPoint)) : null;
  const selWidth = inPoint != null && outPoint != null ? Math.abs(pct(outPoint) - pct(inPoint)) : 0;
  const clipLength = inPoint != null && outPoint != null ? Math.abs(outPoint - inPoint) : null;

  return (
    <div className="bg-gray-900 rounded-xl overflow-hidden shadow-2xl">
      <div className="bg-gray-800 p-4 border-b border-gray-700">
        <h3 className="text-white font-semibold">Trim</h3>
        {/* Named states rather than a blank line: clearing `source` on a URL
            switch is deliberate, and an empty title would read as a failure. */}
        <p className="text-gray-400 text-sm truncate">
          {source?.title || (phase === 'probing' ? 'Reading video info…' : 'Loading…')}
        </p>
      </div>

      <div className="relative bg-black min-h-[16rem] flex items-center justify-center">
        {phase === 'error' ? (
          <div className="text-center p-6">
            <p className="text-red-400 mb-3">{error}</p>
            <button
              onClick={() => loadProxy(url)}
              className="bg-gray-700 hover:bg-gray-600 text-white px-4 py-2 rounded-lg text-sm"
            >
              Try downloading a preview instead
            </button>
            <p className="text-gray-500 text-xs mt-3">
              You can still type start and end times below.
            </p>
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
              //
              // A live stream reports Infinity here. It is truthy, so it used to
              // pass straight through `duration || 0` and poison every derived
              // value — `pct()` returned 0 for every point and the readouts
              // rendered "Infinity:NaN:NaN".
              const d = e.currentTarget.duration;
              setDuration(Number.isFinite(d) && d > 0 ? d : 0);
              setPhase('ready');
            }}
            onTimeUpdate={(e) => setCurrent(e.currentTarget.currentTime)}
            onError={() => {
              // The video-only stream `resolve_preview` may have handed us is
              // very likely playable but not guaranteed to be for every itag,
              // so this fallback is the safety net that keeps such a URL from
              // dead-ending: any stream that will not play becomes a proxy
              // download, exactly as a `needs_proxy` result would have.
              if (source.kind !== 'proxy') loadProxy(url);
              else { setError('Preview could not be played.'); setPhase('error'); }
            }}
          />
        ) : (
          <div className="text-gray-400 p-6">Loading preview…</div>
        )}
      </div>

      {/* A muted preview is a normal outcome, not a fault: modern YouTube serves
          picture and sound separately, and a video-only stream is what lets the
          preview appear at once instead of after a full download. Said plainly,
          in the same muted grey as the other captions — not as an error. */}
      {phase === 'ready' && source?.kind === 'stream' && source?.has_audio === false && (
        <p className="bg-gray-800 px-4 py-2 text-xs text-gray-400 border-b border-gray-700">
          Preview has no sound. The download will include audio.
        </p>
      )}

      <div className="bg-gray-800 p-4">
        <div
          ref={trackRef}
          className={`relative h-8 bg-gray-700 rounded-lg ${playable ? 'cursor-pointer' : 'opacity-50 cursor-not-allowed'}`}
          onPointerDown={(e) => {
            if (!playable) return;
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

          {playable && inPoint != null && (
            <div
              role="slider" aria-label="Trim start" tabIndex={0}
              aria-valuemin={0} aria-valuemax={duration} aria-valuenow={inPoint}
              onPointerDown={(e) => { e.stopPropagation(); setDragging('in'); }}
              className="absolute -top-1 h-10 w-3 -ml-1.5 bg-green-400 rounded cursor-ew-resize"
              style={{ left: `${pct(inPoint)}%` }}
            />
          )}
          {playable && outPoint != null && (
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
          <span>{knownDuration != null ? formatTime(knownDuration) : '—:—'}</span>
        </div>

        <div className="flex flex-wrap items-center gap-2 mt-4">
          <button disabled={!playable} onClick={() => setIn(videoRef.current?.currentTime ?? 0)}
            className="bg-green-600 hover:bg-green-500 disabled:opacity-40 text-white px-4 py-2 rounded-lg text-sm">
            Set Start <kbd className="ml-1 opacity-70">[</kbd>
          </button>
          <button disabled={!playable} onClick={() => setOut(videoRef.current?.currentTime ?? 0)}
            className="bg-red-600 hover:bg-red-500 disabled:opacity-40 text-white px-4 py-2 rounded-lg text-sm">
            Set End <kbd className="ml-1 opacity-70">]</kbd>
          </button>
          {/* Clearing follows whatever can set a value, so a typed-only
              selection is never left with no way to undo it. */}
          <button disabled={!playable && !canEditTimes} onClick={clearSelection}
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
              type="text" value={startInput} disabled={!canEditTimes}
              onChange={(e) => setStartInput(e.target.value)}
              onBlur={() => applyInput(startInput, setIn)}
              onKeyDown={(e) => { if (e.key === 'Enter') applyInput(startInput, setIn); }}
              className="w-full bg-gray-700 text-white px-3 py-2 rounded-lg border border-gray-600 focus:border-green-500 outline-none disabled:opacity-40"
            />
          </div>
          <div>
            <label className="block text-xs text-gray-400 mb-1">End (SS, MM:SS or HH:MM:SS)</label>
            <input
              type="text" value={endInput} disabled={!canEditTimes}
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
