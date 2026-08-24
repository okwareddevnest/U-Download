/** Parses "SS", "MM:SS", or "HH:MM:SS" into seconds. Returns NaN if invalid. */
export function parseTimeToSeconds(value) {
  const s = String(value || '').trim();
  if (!s) return NaN;

  const parts = s.split(':').map((p) => p.trim());
  if (parts.some((p) => p === '' || isNaN(Number(p)))) return NaN;

  if (parts.length === 1) return Math.floor(Number(parts[0]));
  if (parts.length === 2) {
    const [m, sec] = parts.map((p) => Math.floor(Number(p)));
    return m * 60 + sec;
  }
  if (parts.length === 3) {
    const [h, m, sec] = parts.map((p) => Math.floor(Number(p)));
    return h * 3600 + m * 60 + sec;
  }
  return NaN;
}

/**
 * Formats seconds as "M:SS", or "H:MM:SS" once past an hour. The hours case
 * matters: long videos are exactly where trimming is most used.
 */
export function formatTime(seconds) {
  const t = Math.max(0, Math.floor(seconds || 0));
  const h = Math.floor(t / 3600);
  const m = Math.floor((t % 3600) / 60);
  const s = t % 60;

  if (h > 0) {
    return `${h}:${String(m).padStart(2, '0')}:${String(s).padStart(2, '0')}`;
  }
  return `${m}:${String(s).padStart(2, '0')}`;
}
