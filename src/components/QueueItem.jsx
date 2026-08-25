import { useEffect, useState } from 'react';
import Sparkline from './Sparkline';
import { IconPlay } from './icons';
import { formatEta, formatSpeed } from '../lib/format';

// The queue says what a job is doing in the product's own words rather than
// echoing the backend's enum.
const STATUS_LABELS = {
  queued: 'Queued',
  running: 'Downloading',
  downloading: 'Downloading',
  paused: 'Paused',
  done: 'Done',
  failed: 'Failed',
  cancelled: 'Cancelled',
};

const statusLabel = (status) =>
  STATUS_LABELS[status] || (status ? status.charAt(0).toUpperCase() + status.slice(1) : '');

const isLive = (status) => status === 'running' || status === 'downloading';

/**
 * One row of the queue: what is being downloaded, and how it is going.
 *
 * Laid out as a media column and a body column so the thumbnail and the speed
 * trace share one left edge down the whole list — the sparklines are a small
 * multiple, and they only compare if they start at the same x.
 *
 * A trimmed job gets no sparkline. yt-dlp routes `--download-sections` through
 * ffmpeg, which reports elapsed media time rather than bytes, so the speed is
 * always 0 and the ETA is always absent for those. A flat line on the floor
 * would read as "stalled" when the job is in fact working, so the row says in
 * words that no speed is being reported and shows the percentage, which is
 * real.
 */
export default function QueueItem({ job, samples, onCancel }) {
  const [thumbBroken, setThumbBroken] = useState(false);

  // The thumbnail arrives after the job is queued, and a retried job can be
  // given a different one. Either way the previous failure no longer applies.
  useEffect(() => { setThumbBroken(false); }, [job.thumbnail]);

  const { status } = job;
  const live = isLive(status);
  const isFailed = status === 'failed';
  const isDone = status === 'done';
  const isCancelled = status === 'cancelled';
  const isTerminal = isFailed || isDone || isCancelled;

  const trimmed = Boolean(job.trim);
  const pct = Math.round(job.progress?.percentage ?? 0);
  const speed = trimmed ? null : formatSpeed(job.progress?.speed_bytes_per_sec);
  const eta = trimmed ? null : formatEta(job.progress?.eta_seconds);

  const barColor = isFailed
    ? 'bg-danger'
    : isDone
      ? 'bg-ok'
      : isCancelled
        ? 'bg-hair-strong'
        : 'bg-accent';
  const statusColor = isFailed
    ? 'text-danger'
    : isDone
      ? 'text-ok'
      : isTerminal
        ? 'text-fg-muted'
        : 'text-fg';
  const fill = isDone ? 1 : Math.max(0, Math.min(1, pct / 100));

  const label = live && trimmed ? 'Trimming' : statusLabel(status);
  const showThumb = Boolean(job.thumbnail) && !thumbBroken;

  return (
    <div className="border-b border-hair px-4 py-3">
      <div className="flex gap-3">
        <div className="w-[5.25rem] shrink-0">
          {/* The 16:9 box is held whether or not an image ever arrives, so the
              row does not jump as thumbnails load in behind the queue. */}
          <div className="flex aspect-video items-center justify-center overflow-hidden rounded-field bg-sunken">
            {showThumb ? (
              <img
                src={job.thumbnail}
                alt=""
                loading="lazy"
                decoding="async"
                draggable={false}
                onError={() => setThumbBroken(true)}
                className="h-full w-full object-cover"
              />
            ) : (
              <IconPlay size={16} className="text-fg-muted" />
            )}
          </div>

          {/* Reserved from the moment the job goes live, so the row settles
              once rather than growing again when the second sample lands. */}
          {live && !trimmed && (
            <div className="mt-1.5 h-[22px]">
              <Sparkline samples={samples} />
            </div>
          )}
        </div>

        <div className="flex min-w-0 flex-1 flex-col">
          <div className="flex items-start gap-2">
            <p className="line-clamp-2 min-w-0 flex-1 text-ui font-medium" title={job.title || job.url}>
              {job.title || job.url}
            </p>
            <span className={`shrink-0 text-meta font-medium ${statusColor}`}>{label}</span>
          </div>

          {!isTerminal && (
            <div className="mt-auto flex items-center gap-3 pt-1.5 text-meta text-fg-muted">
              <span className="tnum">{pct}%</span>
              {trimmed && live && <span>speed not reported</span>}
              {speed && <span className="tnum">{speed}</span>}
              {eta && <span className="tnum">{eta}</span>}
              <button
                type="button"
                onClick={() => onCancel(job.id)}
                className="btn btn-sm btn-danger -my-1 ml-auto"
              >
                Cancel
              </button>
            </div>
          )}
        </div>
      </div>

      <div className="mt-2.5 h-[3px] overflow-hidden bg-sunken">
        <div
          className={`h-full origin-left ${barColor} transition-transform duration-200 ease-out`}
          style={{ transform: `scaleX(${fill})` }}
        />
      </div>

      {isFailed && job.error && <p className="mt-1.5 text-meta text-danger">{job.error}</p>}
    </div>
  );
}
