import { useState, useRef, useEffect, useLayoutEffect, useCallback } from 'react';
import { invoke, convertFileSrc } from '@tauri-apps/api/core';
import { formatTime, parseTimeToSeconds } from '../lib/time';
import { tileAt, tileStyle } from '../lib/storyboard';
import { IconPlay, IconPause, IconSound, IconSoundOff } from './icons';

// Rendered width of one storyboard tile, in CSS pixels. 160 is the native width
// of YouTube's `sb1` sheets, so the common case is painted 1:1; a coarser or
// finer track is scaled to the same box so the hover frame is one size.
const TILE_DISPLAY_WIDTH = 160;

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
export default function TrimWorkbench({ url, onChange, onClose }) {
  const videoRef = useRef(null);
  const audioRef = useRef(null);
  const trackRef = useRef(null);

  const [source, setSource] = useState(null);      // { kind, url, title, duration, has_audio, audio_url }
  const [phase, setPhase] = useState('idle');      // idle|probing|proxying|ready|error
  const [error, setError] = useState('');
  const [duration, setDuration] = useState(0);     // from the <video> element
  const [current, setCurrent] = useState(0);
  const [inPoint, setInPoint] = useState(null);
  const [outPoint, setOutPoint] = useState(null);
  const [dragging, setDragging] = useState(null);  // 'in' | 'out' | null
  const [startInput, setStartInput] = useState('');
  const [endInput, setEndInput] = useState('');
  const [isPlaying, setIsPlaying] = useState(false);
  const [muted, setMuted] = useState(false);
  const [audioBroken, setAudioBroken] = useState(false);
  // Why the picture is not moving. `buffering` is the element asking for data
  // it does not have; `stalled` is that request getting nowhere. Both were
  // previously invisible, which is what made a slow 2.9 GB stream look like a
  // dead play button rather than like a slow stream.
  const [buffering, setBuffering] = useState(false);
  const [stalled, setStalled] = useState(false);
  // A rejected play() request, in words. The empty `.catch(() => {})` this
  // replaces is the reason the reported failure produced no diagnostic at all.
  const [playbackNote, setPlaybackNote] = useState('');
  // Pointer position over the scrub track, as a fraction of its width plus the
  // timestamp it lands on. Stored as a ratio, not an x, so the hover frame
  // stays put when the window is resized mid-hover.
  const [hover, setHover] = useState(null);

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
      // video-only stream, and the proxy yt-dlp just muxed is not silent. The
      // separate audio track goes with it — a muxed file carries its own sound.
      setSource((s) => ({ ...s, kind: 'proxy', url: convertFileSrc(path), has_audio: true, audio_url: null }));
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
      setIsPlaying(false);
      setPlaybackNote('');
      setBuffering(false);
      setStalled(false);
      setHover(null);
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

  // --- why nothing is moving --------------------------------------------------

  // A preview stream can be perfectly healthy and still show a black rectangle
  // for a long time: a nine-hour recording is a multi-gigabyte file, and the
  // webview must pull its index before it can paint a single frame. Silence
  // there is indistinguishable from a broken player, so the element's own
  // account of what it is waiting for is put on screen.
  useEffect(() => {
    const v = videoRef.current;
    if (!v) return;

    const startWaiting = () => setBuffering(true);
    const stopWaiting = () => { setBuffering(false); setStalled(false); };
    // `stalled` means the request has produced no data for three seconds. It is
    // said differently from ordinary buffering because the remedy differs: one
    // resolves by waiting, the other usually does not.
    const onStalled = () => setStalled(true);
    const onProgress = () => setStalled(false);

    v.addEventListener('waiting', startWaiting);
    v.addEventListener('seeking', startWaiting);
    v.addEventListener('canplay', stopWaiting);
    v.addEventListener('playing', stopWaiting);
    v.addEventListener('seeked', stopWaiting);
    v.addEventListener('stalled', onStalled);
    v.addEventListener('progress', onProgress);

    return () => {
      v.removeEventListener('waiting', startWaiting);
      v.removeEventListener('seeking', startWaiting);
      v.removeEventListener('canplay', stopWaiting);
      v.removeEventListener('playing', stopWaiting);
      v.removeEventListener('seeked', stopWaiting);
      v.removeEventListener('stalled', onStalled);
      v.removeEventListener('progress', onProgress);
    };
  }, [source?.url]);

  // --- the separate audio track ---------------------------------------------

  // A modern YouTube pick is often video-only. When the backend can name an
  // audio-only companion for it, the preview plays both and stays in step
  // rather than running silent until a proxy finishes downloading.
  const audioUrl = source?.has_audio === false ? (source?.audio_url || null) : null;
  const audioActive = Boolean(audioUrl) && !audioBroken;

  // A new companion track is innocent until it fails on its own account.
  useEffect(() => { setAudioBroken(false); }, [audioUrl]);

  useEffect(() => {
    if (!audioActive) return;
    const v = videoRef.current;
    const a = audioRef.current;
    if (!v || !a) return;

    // Two independent media elements will drift — different buffers, different
    // decode clocks. 150ms is roughly where a viewer starts to notice lip sync
    // slipping, so that is the correction threshold rather than an exact match
    // that would re-seek constantly and stutter the sound.
    const DRIFT_LIMIT = 0.15;

    const resync = () => {
      if (Math.abs(a.currentTime - v.currentTime) > DRIFT_LIMIT) {
        try { a.currentTime = v.currentTime; } catch { /* seek refused; the drift check retries */ }
      }
    };

    const onPlay = () => {
      a.playbackRate = v.playbackRate;
      resync();
      // Autoplay policy or a decode failure rejects here. Either way the
      // picture keeps playing; sound is the thing that is optional.
      a.play().catch(() => {});
    };
    const onPause = () => { a.pause(); };
    const onSeeked = () => { try { a.currentTime = v.currentTime; } catch { /* ignored */ } };
    const onRate = () => { a.playbackRate = v.playbackRate; };
    const onVolume = () => { a.muted = v.muted; a.volume = v.volume; };
    // Video buffering must not leave the audio running on ahead.
    const onWaiting = () => { a.pause(); };
    const onPlaying = () => { resync(); if (!v.paused) a.play().catch(() => {}); };
    const onEnded = () => { a.pause(); };
    // A failed audio track is dropped in silence: the preview is still a
    // preview without it, and an error here must never take the picture down.
    const onAudioError = () => { try { a.pause(); } catch { /* ignored */ } setAudioBroken(true); };

    a.muted = v.muted;
    a.volume = v.volume;
    a.playbackRate = v.playbackRate;
    if (!v.paused) onPlay();

    v.addEventListener('play', onPlay);
    v.addEventListener('pause', onPause);
    v.addEventListener('seeked', onSeeked);
    v.addEventListener('ratechange', onRate);
    v.addEventListener('volumechange', onVolume);
    v.addEventListener('waiting', onWaiting);
    v.addEventListener('playing', onPlaying);
    v.addEventListener('ended', onEnded);
    a.addEventListener('error', onAudioError);

    // Seeks and stalls are handled above; this catches the slow accumulating
    // drift that no single event announces.
    const driftTimer = setInterval(() => { if (!v.paused) resync(); }, 1000);

    return () => {
      clearInterval(driftTimer);
      v.removeEventListener('play', onPlay);
      v.removeEventListener('pause', onPause);
      v.removeEventListener('seeked', onSeeked);
      v.removeEventListener('ratechange', onRate);
      v.removeEventListener('volumechange', onVolume);
      v.removeEventListener('waiting', onWaiting);
      v.removeEventListener('playing', onPlaying);
      v.removeEventListener('ended', onEnded);
      a.removeEventListener('error', onAudioError);
      try { a.pause(); } catch { /* ignored */ }
    };
  }, [audioActive, audioUrl, source?.url, phase]);

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

  // --- transport ------------------------------------------------------------

  /**
   * Play/pause, with the reason stated when play is refused.
   *
   * `play()` returns a promise that rejects for reasons the user can act on —
   * an undecodable stream, a blocked autoplay policy — and the empty `.catch`
   * this replaces threw all of them away. That is the same class of bug as the
   * `console.warn` the dead slider hid inside: a failure the product knows
   * about and does not say.
   */
  const togglePlay = () => {
    const v = videoRef.current;
    if (!v) return;
    if (!v.paused) { v.pause(); return; }

    setPlaybackNote('');
    const started = v.play();
    // Older engines return undefined here; only a promise can reject.
    if (!started || typeof started.catch !== 'function') return;

    started.catch((err) => {
      const name = err?.name || '';
      if (name === 'AbortError') {
        // The request was superseded by a pause or a reload — what happens when
        // the button is pressed twice, or the playhead moved mid-start. Nothing
        // failed, so nothing is reported. Handled explicitly, not swallowed.
        return;
      }
      if (name === 'NotSupportedError') {
        // The stream arrived but this engine cannot decode it. Same remedy as
        // the element's own error event: fetch a small muxed copy instead.
        setPlaybackNote('This preview stream cannot be played here. Fetching a small copy instead.');
        if (source?.kind !== 'proxy') loadProxy(url);
        return;
      }
      if (name === 'NotAllowedError') {
        setPlaybackNote('The system blocked playback. Press play again, or mute the preview first.');
        return;
      }
      setPlaybackNote(`Could not start playback: ${err?.message || err}`);
    });
  };

  const toggleMute = () => {
    const v = videoRef.current;
    const next = !muted;
    setMuted(next);
    // Written straight onto the element so `volumechange` fires and the
    // companion audio track follows through the sync effect above.
    if (v) v.muted = next;
  };

  // --- storyboard hover frames ------------------------------------------------

  // Absent for most sources: only some extractors publish sprite sheets, and
  // everything below reduces to "no hover frame" when they do not.
  const storyboard = source?.storyboard || null;

  /**
   * Where the pointer is over the track, as a ratio and a timestamp.
   *
   * Deliberately separate from `timeFromClientX`, which drives seeking and
   * therefore requires a decoded, seekable element. Showing a frame does not:
   * a duration from the probe is enough, so hover frames work even while the
   * stream itself is still loading — which is precisely when they are most
   * useful. The scrub bar's own behaviour is untouched.
   */
  const hoverAt = (clientX) => {
    const rect = trackRef.current?.getBoundingClientRect();
    if (!rect || rect.width === 0 || knownDuration == null) return null;
    const ratio = Math.max(0, Math.min(1, (clientX - rect.left) / rect.width));
    return { ratio, time: ratio * knownDuration };
  };

  const hoverTile = hover ? tileAt(storyboard, hover.time) : null;

  // Sheets are fetched one at a time, when first hovered, and never twice: the
  // `sb1` track of the reported nine-hour video is 135 sheets at ~56 KB, so
  // loading it up front would be 7.5 MB and 135 requests for frames the user
  // will mostly never look at. `pendingSheets` remembers what is in flight and
  // what has failed; `readySheets` is state because a decoded sheet is a reason
  // to paint. Once decoded, later hovers into the same sheet are a repaint.
  const pendingSheets = useRef(new Map());
  const [readySheets, setReadySheets] = useState(() => new Set());

  // A different video means different sheet URLs. The browser's own HTTP cache
  // still holds anything fetched before, so returning to a video is cheap.
  useEffect(() => {
    pendingSheets.current = new Map();
    setReadySheets(new Set());
  }, [url]);

  const hoverSheetUrl = hoverTile?.url || null;
  useEffect(() => {
    if (!hoverSheetUrl) return;
    if (pendingSheets.current.has(hoverSheetUrl)) return;

    pendingSheets.current.set(hoverSheetUrl, 'loading');
    const img = new Image();
    img.onload = () => {
      pendingSheets.current.set(hoverSheetUrl, 'ready');
      setReadySheets((prev) => new Set(prev).add(hoverSheetUrl));
    };
    // A sheet that 404s or whose signature has expired is remembered as failed
    // and never retried. Nothing is said about it: the hover frame is an
    // enhancement to scrubbing, and its absence must read as "this source has
    // no thumbnails", not as a fault.
    img.onerror = () => { pendingSheets.current.set(hoverSheetUrl, 'error'); };
    img.src = hoverSheetUrl;
    // No cleanup on purpose. This effect re-runs on every sheet the pointer
    // crosses, and detaching the handlers there would leave the entry stuck at
    // "loading" — the sheet would be fetched and then never recorded, so it
    // could never be painted and would never be retried.
  }, [hoverSheetUrl]);

  const hoverStyle = hoverSheetUrl && readySheets.has(hoverSheetUrl)
    ? tileStyle(storyboard, hoverTile, TILE_DISPLAY_WIDTH)
    : null;

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
      // A handle is being dragged to a moment, which is exactly when the frame
      // at that moment is wanted. The pointer has left the track's own move
      // events by now — it is captured on the window — so hover is updated here.
      setHover(hoverAt(e.clientX));
    };
    const onUp = () => { setDragging(null); setHover(null); };

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

  // Named states rather than a blank line: clearing `source` on a URL switch is
  // deliberate, and an empty title would read as a failure.
  const heading = source?.title
    || (phase === 'probing' ? 'Reading video information'
      : phase === 'proxying' ? 'Preparing preview'
      : phase === 'error' ? 'Preview unavailable'
      : 'Loading');

  return (
    <div className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-3 px-5 pb-2">
        <span className="label-region shrink-0">Preview</span>
        <p className="min-w-0 flex-1 truncate text-meta text-fg-muted" title={source?.title || ''}>
          {heading}
        </p>
        {onClose && (
          <button type="button" onClick={onClose} className="btn btn-sm btn-quiet shrink-0">
            Hide
          </button>
        )}
      </div>

      {/* The stage. Full-bleed between two hairlines rather than boxed in a
          card: the picture is the content, and a frame around it would only
          add a second edge next to the window's own. */}
      <div className="on-stage relative flex min-h-[6rem] flex-1 items-center justify-center overflow-hidden border-y border-hair bg-stage">
        {phase === 'error' ? (
          <div className="max-w-sm px-6 py-5 text-center">
            <p className="text-body text-danger">{error}</p>
            <button
              type="button"
              onClick={() => loadProxy(url)}
              className="btn btn-secondary mt-3"
            >
              Download a preview instead
            </button>
            <p className="mt-3 text-meta text-white/55">
              Start and end times can still be typed below.
            </p>
          </div>
        ) : phase === 'proxying' ? (
          <div className="w-full max-w-xs px-6 text-center">
            <div className="rail" />
            <p className="mt-3 text-body text-white/85">Preparing preview</p>
            <p className="mt-1 text-meta text-white/55">
              This video has no directly playable stream, so a small copy is being fetched.
            </p>
          </div>
        ) : source?.url ? (
          <>
          <video
            ref={videoRef}
            src={source.url}
            playsInline
            muted={muted}
            className="h-full w-full object-contain"
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
            onPlay={() => setIsPlaying(true)}
            onPause={() => setIsPlaying(false)}
            onEnded={() => setIsPlaying(false)}
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

          {/* What the stage is doing while it shows nothing. A black rectangle
              with no caption is the reported symptom, and on a long recording
              it can persist for a while entirely legitimately: the webview has
              to fetch the file's index before it can paint a frame. The same
              2px rail used elsewhere for indeterminate work, laid across the
              top edge of the picture rather than parked in the middle of it. */}
          {(phase !== 'ready' || buffering || stalled) && (
            <div className="pointer-events-none absolute inset-x-0 top-0">
              <div className="rail" />
            </div>
          )}
          {/* Keyed on the phase and not on `playable`, which is also false for
              a live stream whose duration is Infinity — that video plays fine
              and must not sit under a permanent "loading" caption. */}
          {phase !== 'ready' ? (
            <p className="pointer-events-none absolute inset-x-0 top-1/2 -mt-3 px-6 text-center text-body text-white/85">
              Loading video
            </p>
          ) : (buffering || stalled) && (
            <p className="pointer-events-none absolute bottom-2 left-3 right-3 truncate text-meta text-white/70">
              {stalled
                ? 'Still loading. This stream is slow to seek.'
                : 'Buffering'}
            </p>
          )}
          </>
        ) : (
          /* Honest about the wait. Reading a URL's information is a full
             extractor run on the site, measured here at half a minute or more
             on a first load, and a bare "Reading video information" under a
             moving rail reads as a hang long before it finishes. The second
             line is not reassurance for its own sake: the result is cached, so
             the wait genuinely is a once-per-link cost, and knowing that is what
             makes waiting it out reasonable rather than something to escape by
             closing the panel. No countdown and no percentage — nothing here
             knows how long the site will take. */
          <div className="w-full max-w-xs px-6 text-center">
            <div className="rail" />
            <p className="mt-3 text-body text-white/85">Reading video information</p>
            <p className="mt-1 text-meta text-white/55">
              The first read of a link can take up to a minute. Opening the same link
              again is immediate.
            </p>
          </div>
        )}

        {/* Sound for a video-only pick. Hidden by design: the transport above
            is the only control surface, and this element follows the video. */}
        {audioActive && (
          <audio ref={audioRef} src={audioUrl} preload="auto" className="hidden" />
        )}
      </div>

      {/* Transport. The track doubles as the trim timeline — one ruler for
          both jobs, so a cut point is placed exactly where the playhead is. */}
      <div className="flex items-center gap-3 px-5 py-2.5">
        <button
          type="button"
          onClick={togglePlay}
          disabled={!playable}
          className="icon-btn shrink-0"
          aria-label={isPlaying ? 'Pause' : 'Play'}
          title={isPlaying ? 'Pause' : 'Play'}
        >
          {isPlaying ? <IconPause size={18} /> : <IconPlay size={18} />}
        </button>

        <div
          ref={trackRef}
          className={`relative h-6 flex-1 rounded-[3px] bg-sunken ${playable ? 'cursor-pointer' : 'opacity-60'}`}
          onPointerDown={(e) => {
            if (!playable) return;
            const t = timeFromClientX(e.clientX);
            if (videoRef.current) videoRef.current.currentTime = t;
          }}
          onPointerMove={(e) => setHover(hoverAt(e.clientX))}
          onPointerLeave={() => { if (!dragging) setHover(null); }}
        >
          {/* The storyboard frame for the moment under the pointer. Present
              only once its sheet has decoded, so a slow or missing sheet shows
              nothing rather than a torn or empty box. `clamp` keeps the frame
              inside the track at both ends without measuring anything. */}
          {hoverStyle && (
            <div
              className="pointer-events-none absolute bottom-full z-10 mb-2 -translate-x-1/2"
              style={{ left: `clamp(${TILE_DISPLAY_WIDTH / 2}px, ${hover.ratio * 100}%, calc(100% - ${TILE_DISPLAY_WIDTH / 2}px))` }}
            >
              <div className="relative border border-hair-strong bg-stage" style={hoverStyle}>
                <span className="tnum absolute inset-x-0 bottom-0 bg-black/70 py-px text-center text-[11px] leading-4 text-white">
                  {formatTime(hover.time)}
                </span>
              </div>
            </div>
          )}
          {hoverStyle && (
            <div
              className="pointer-events-none absolute inset-y-0 -ml-px w-px bg-fg-muted"
              style={{ left: `${hover.ratio * 100}%` }}
            />
          )}

          {selLeft != null && (
            <div
              className="pointer-events-none absolute inset-y-0 border-x border-accent bg-accent-soft"
              style={{ left: `${selLeft}%`, width: `${selWidth}%` }}
            />
          )}

          <div
            className="pointer-events-none absolute inset-y-0 -ml-px w-0.5 bg-fg"
            style={{ left: `${pct(current)}%` }}
          />

          {playable && inPoint != null && (
            <div
              role="slider" aria-label="Trim start" tabIndex={0}
              aria-valuemin={0} aria-valuemax={duration} aria-valuenow={inPoint}
              onPointerDown={(e) => { e.stopPropagation(); setDragging('in'); }}
              className="absolute -inset-y-1 -ml-2 w-4 cursor-ew-resize touch-none"
              style={{ left: `${pct(inPoint)}%` }}
            >
              <span className="absolute inset-y-0 left-1/2 -ml-px w-0.5 bg-accent" />
              {/* Cap at the top marks the in-point; the out handle's cap sits
                  at the bottom. Two handles in one accent, told apart by
                  shape rather than by a second colour. */}
              <span className="absolute left-1/2 top-0 -ml-1 h-1.5 w-2 bg-accent" />
            </div>
          )}
          {playable && outPoint != null && (
            <div
              role="slider" aria-label="Trim end" tabIndex={0}
              aria-valuemin={0} aria-valuemax={duration} aria-valuenow={outPoint}
              onPointerDown={(e) => { e.stopPropagation(); setDragging('out'); }}
              className="absolute -inset-y-1 -ml-2 w-4 cursor-ew-resize touch-none"
              style={{ left: `${pct(outPoint)}%` }}
            >
              <span className="absolute inset-y-0 left-1/2 -ml-px w-0.5 bg-accent" />
              <span className="absolute bottom-0 left-1/2 -ml-1 h-1.5 w-2 bg-accent" />
            </div>
          )}
        </div>

        <button
          type="button"
          onClick={toggleMute}
          disabled={!playable}
          className="icon-btn shrink-0"
          aria-label={muted ? 'Unmute preview' : 'Mute preview'}
          title={muted ? 'Unmute preview' : 'Mute preview'}
        >
          {muted ? <IconSoundOff size={18} /> : <IconSound size={18} />}
        </button>

        <span className="tnum shrink-0 text-meta text-fg-muted">
          {formatTime(current)} / {knownDuration != null ? formatTime(knownDuration) : '--:--'}
        </span>
      </div>

      {/* Why the play button did nothing. Sits directly under the transport it
          belongs to, not at the foot of the panel with the input errors. */}
      {playbackNote && (
        <p className="px-5 pb-1 text-meta text-danger">{playbackNote}</p>
      )}

      {/* A muted preview is a normal outcome, not a fault: modern YouTube serves
          picture and sound separately. It is only said when no companion audio
          track could be played — with one, the preview has sound and the note
          would be a lie. */}
      {phase === 'ready' && source?.kind === 'stream' && source?.has_audio === false && !audioActive && (
        <p className="px-5 pb-1 text-meta text-fg-muted">
          This preview stream carries no sound. The download will include audio.
        </p>
      )}

      <div className="flex flex-wrap items-center gap-x-4 gap-y-2 px-5 pb-4 pt-1">
        <div className="flex items-center gap-2">
          <label htmlFor="trim-in" className="label-region">In</label>
          <input
            id="trim-in"
            type="text" value={startInput} disabled={!canEditTimes}
            placeholder="0:00"
            aria-label="Trim start time"
            onChange={(e) => setStartInput(e.target.value)}
            onBlur={() => applyInput(startInput, setIn)}
            onKeyDown={(e) => { if (e.key === 'Enter') applyInput(startInput, setIn); }}
            className="field tnum w-[5.5rem] text-center"
          />
          <button
            type="button" disabled={!playable}
            onClick={() => setIn(videoRef.current?.currentTime ?? 0)}
            className="btn btn-sm btn-secondary"
            title="Set the start point at the playhead"
          >
            Mark<kbd className="tnum font-sans text-fg-muted">[</kbd>
          </button>
        </div>

        <div className="flex items-center gap-2">
          <label htmlFor="trim-out" className="label-region">Out</label>
          <input
            id="trim-out"
            type="text" value={endInput} disabled={!canEditTimes}
            placeholder="0:00"
            aria-label="Trim end time"
            onChange={(e) => setEndInput(e.target.value)}
            onBlur={() => applyInput(endInput, setOut)}
            onKeyDown={(e) => { if (e.key === 'Enter') applyInput(endInput, setOut); }}
            className="field tnum w-[5.5rem] text-center"
          />
          <button
            type="button" disabled={!playable}
            onClick={() => setOut(videoRef.current?.currentTime ?? 0)}
            className="btn btn-sm btn-secondary"
            title="Set the end point at the playhead"
          >
            Mark<kbd className="tnum font-sans text-fg-muted">]</kbd>
          </button>
        </div>

        <div className="ml-auto flex items-center gap-3">
          {clipLength != null && (
            <span className="tnum text-meta text-fg-muted">
              Clip <span className="font-medium text-fg">{formatTime(clipLength)}</span>
            </span>
          )}
          {/* Clearing follows whatever can set a value, so a typed-only
              selection is never left with no way to undo it. */}
          <button
            type="button" disabled={!playable && !canEditTimes}
            onClick={clearSelection}
            className="btn btn-sm btn-quiet"
          >
            Clear
          </button>
        </div>

        {error && phase !== 'error' && (
          <p className="w-full text-meta text-danger">{error}</p>
        )}

        {playable && (
          <p className="w-full text-meta text-fg-muted">
            Arrow keys nudge the playhead a second, Shift for a tenth.
          </p>
        )}
      </div>
    </div>
  );
}
