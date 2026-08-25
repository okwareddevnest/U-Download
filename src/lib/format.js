/**
 * Number-to-words helpers for the queue readouts.
 *
 * Kept apart from `time.js`: that file speaks timecodes ("1:05:20"), which is
 * the right language for a scrub bar and the wrong one for a countdown. Nobody
 * reads "2:14" and thinks "a bit over two minutes to wait".
 */

/**
 * Throughput as a person would say it. Returns null rather than "0 KB/s" when
 * nothing is being reported, so callers can omit the readout instead of
 * printing a zero that looks like a measurement.
 */
export function formatSpeed(bytesPerSec) {
  const v = Number(bytesPerSec);
  if (!Number.isFinite(v) || v <= 0) return null;
  if (v < 1024) return `${Math.round(v)} B/s`;
  const kb = v / 1024;
  if (kb < 1024) return `${Math.round(kb)} KB/s`;
  return `${(kb / 1024).toFixed(1)} MB/s`;
}

/**
 * Time remaining in words: "48s left", "2m 14s left", "1h 3m left".
 *
 * Null for anything that is not a real countdown — no estimate yet, a negative
 * or non-finite value, or zero, which arrives in the last instant of a
 * download and would render as the nonsense "0s left".
 */
export function formatEta(seconds) {
  const value = Number(seconds);
  if (!Number.isFinite(value) || value <= 0) return null;

  const total = Math.round(value);
  if (total < 60) return `${total}s left`;

  const h = Math.floor(total / 3600);
  const m = Math.floor((total % 3600) / 60);
  const s = total % 60;

  if (h > 0) return m > 0 ? `${h}h ${m}m left` : `${h}h left`;
  return s > 0 ? `${m}m ${s}s left` : `${m}m left`;
}
