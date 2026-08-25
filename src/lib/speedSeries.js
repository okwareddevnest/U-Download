/**
 * The throughput series behind each queue row's sparkline.
 *
 * No such history exists in the backend and none is being added: a download's
 * speed trace is worth exactly as much as the window that is looking at it, so
 * it is accumulated here from the progress events React already re-renders on,
 * and it dies with the job.
 */

/**
 * Samples retained per job. Progress arrives roughly once a second, so this is
 * about the last minute of throughput — enough to read a stall or a ramp, and a
 * hard ceiling so an overnight download cannot grow the series without bound.
 */
export const SPEED_WINDOW = 60;

/**
 * Appends one sample, dropping the oldest once the window is full.
 *
 * Returns a new array — the series is rendered from React state, so mutating
 * in place would leave the sparkline showing the previous frame.
 */
export function appendSample(series, value, limit = SPEED_WINDOW) {
  const v = Number(value);
  const sample = Number.isFinite(v) && v > 0 ? v : 0;
  const next = series ? [...series, sample] : [sample];
  return next.length > limit ? next.slice(next.length - limit) : next;
}
