import { describe, it, expect } from 'vitest';
import { parseTimeToSeconds, formatTime } from './time';

describe('parseTimeToSeconds', () => {
  it('parses bare seconds', () => {
    expect(parseTimeToSeconds('45')).toBe(45);
  });

  it('parses MM:SS', () => {
    expect(parseTimeToSeconds('2:30')).toBe(150);
  });

  it('parses HH:MM:SS', () => {
    expect(parseTimeToSeconds('1:02:03')).toBe(3723);
  });

  it('tolerates surrounding whitespace', () => {
    expect(parseTimeToSeconds('  2:30  ')).toBe(150);
  });

  it('returns NaN for non-numeric input', () => {
    expect(parseTimeToSeconds('abc')).toBeNaN();
    expect(parseTimeToSeconds('1:xx')).toBeNaN();
  });

  it('returns NaN for empty input', () => {
    expect(parseTimeToSeconds('')).toBeNaN();
    expect(parseTimeToSeconds(null)).toBeNaN();
  });

  it('returns NaN for too many segments', () => {
    expect(parseTimeToSeconds('1:2:3:4')).toBeNaN();
  });
});

describe('formatTime', () => {
  it('formats under a minute', () => {
    expect(formatTime(45)).toBe('0:45');
  });

  it('formats minutes and seconds', () => {
    expect(formatTime(150)).toBe('2:30');
  });

  // The previous implementation rendered 3723 as "62:03", which is unreadable
  // for the long videos trimming is most useful on.
  it('formats past an hour as H:MM:SS', () => {
    expect(formatTime(3723)).toBe('1:02:03');
  });

  it('clamps negatives to zero', () => {
    expect(formatTime(-5)).toBe('0:00');
  });

  it('handles null and undefined', () => {
    expect(formatTime(null)).toBe('0:00');
    expect(formatTime(undefined)).toBe('0:00');
  });

  it('round-trips with parseTimeToSeconds', () => {
    for (const secs of [0, 45, 150, 3723, 7199]) {
      expect(parseTimeToSeconds(formatTime(secs))).toBe(secs);
    }
  });
});
