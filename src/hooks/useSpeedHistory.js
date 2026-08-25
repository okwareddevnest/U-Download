import { useEffect, useRef, useState } from 'react';
import { appendSample } from '../lib/speedSeries';

const isLive = (status) => status === 'running' || status === 'downloading';

/**
 * Collects a per-job speed history from successive job renders.
 *
 * Two details carry the whole hook:
 *
 * 1. Any one job's `job-updated` event re-renders the entire list, so folding a
 *    sample in for every live job on every render would multiply a two-download
 *    queue's series by two and make an idle job look like a busy one. Each job's
 *    last reported progress is remembered and a sample is taken only when that
 *    job itself reports something new.
 * 2. A job that has left the queue or reached a terminal state keeps no series.
 *    Nothing draws a graph for it, and holding the samples would be a slow leak
 *    across a long session.
 */
export function useSpeedHistory(jobs) {
  const [history, setHistory] = useState({});
  // job id -> the last progress report already folded into that job's series.
  const seenRef = useRef({});

  useEffect(() => {
    const seen = seenRef.current;
    const fresh = [];
    const live = new Set();

    jobs.forEach((job) => {
      if (!isLive(job.status)) return;
      live.add(job.id);

      const p = job.progress;
      if (!p) return;

      const report = `${p.bytes_downloaded}/${p.percentage}/${p.speed_bytes_per_sec}`;
      if (seen[job.id] === report) return;
      seen[job.id] = report;
      fresh.push([job.id, p.speed_bytes_per_sec]);
    });

    Object.keys(seen).forEach((id) => {
      if (!live.has(id)) delete seen[id];
    });

    setHistory((prev) => {
      const stale = Object.keys(prev).filter((id) => !live.has(id));
      if (fresh.length === 0 && stale.length === 0) return prev;

      const next = { ...prev };
      stale.forEach((id) => delete next[id]);
      fresh.forEach(([id, speed]) => {
        next[id] = appendSample(next[id], speed);
      });
      return next;
    });
  }, [jobs]);

  return history;
}
