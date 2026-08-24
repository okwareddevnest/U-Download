import { useState, useEffect, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';

/**
 * Keeps a map of jobs keyed by id, updated from job-scoped events.
 *
 * Routing by job_id is what makes concurrent downloads possible: the previous
 * implementation wrote every progress event into one set of global useState
 * variables, so a second download would overwrite the first one's display.
 */
export function useJobs({ onDone } = {}) {
  const [jobsById, setJobsById] = useState({});
  const [error, setError] = useState(null);
  const unlistenRefs = useRef([]);
  const onDoneRef = useRef(onDone);
  onDoneRef.current = onDone;

  useEffect(() => {
    let cancelled = false;

    (async () => {
      try {
        const initial = await invoke('list_jobs');
        if (!cancelled) {
          setJobsById(Object.fromEntries(initial.map((j) => [j.id, j])));
          setError(null);
        }
      } catch (e) {
        console.error('Failed to load jobs:', e);
        if (!cancelled) {
          setError(e);
        }
      }

      const unlistenUpdated = await listen('job-updated', (event) => {
        if (cancelled) return;
        const job = event.payload;
        setJobsById((prev) => ({ ...prev, [job.id]: job }));
      });

      if (cancelled) {
        unlistenUpdated();
        return;
      }

      const unlistenFailed = await listen('job-failed', (event) => {
        if (cancelled) return;
        const { job_id, error } = event.payload;
        setJobsById((prev) =>
          prev[job_id]
            ? { ...prev, [job_id]: { ...prev[job_id], status: 'failed', error } }
            : prev
        );
      });

      if (cancelled) {
        unlistenUpdated();
        unlistenFailed();
        return;
      }

      const unlistenDone = await listen('job-done', (event) => {
        if (cancelled) return;
        const { job_id, output_path, title } = event.payload;
        setJobsById((prev) =>
          prev[job_id]
            ? {
                ...prev,
                [job_id]: { ...prev[job_id], status: 'done', output_path, title },
              }
            : prev
        );
        onDoneRef.current?.({ job_id, output_path, title });
      });

      if (cancelled) {
        unlistenUpdated();
        unlistenFailed();
        unlistenDone();
        return;
      }

      unlistenRefs.current = [unlistenUpdated, unlistenFailed, unlistenDone];
    })();

    return () => {
      cancelled = true;
      unlistenRefs.current.forEach((fn) => fn && fn());
    };
  }, []);

  const enqueue = useCallback(async ({ url, format, trim, outputFolder, concurrency }) => {
    const id = await invoke('enqueue_job', {
      url,
      format,
      trim: trim ?? null,
      outputFolder,
      concurrency: concurrency ?? 2,
    });
    return id;
  }, []);

  const cancel = useCallback((jobId) => invoke('cancel_job', { jobId }), []);
  const pause = useCallback((jobId) => invoke('pause_job', { jobId }), []);
  const resume = useCallback((jobId) => invoke('resume_job', { jobId }), []);

  const jobs = Object.values(jobsById).sort((a, b) => a.created_at - b.created_at);

  return { jobs, error, enqueue, cancel, pause, resume };
}
