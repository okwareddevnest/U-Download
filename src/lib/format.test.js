import { describe, it, expect } from 'vitest';
import { formatSpeed, formatEta } from './format';
import { appendSample, SPEED_WINDOW } from './speedSeries';

describe('formatSpeed', () => {
  it('omits a reading rather than printing a zero that looks measured', () => {
    expect(formatSpeed(0)).toBeNull();
    expect(formatSpeed(null)).toBeNull();
    expect(formatSpeed(undefined)).toBeNull();
  });

  it('scales through B, KB and MB', () => {
    expect(formatSpeed(512)).toBe('512 B/s');
    expect(formatSpeed(1024 * 400)).toBe('400 KB/s');
    expect(formatSpeed(1024 * 1024 * 1.5)).toBe('1.5 MB/s');
  });
});

describe('formatEta', () => {
  it('has no countdown to give without an estimate', () => {
    expect(formatEta(null)).toBeNull();
    expect(formatEta(undefined)).toBeNull();
    expect(formatEta(0)).toBeNull();
    expect(formatEta(-5)).toBeNull();
    expect(formatEta(Infinity)).toBeNull();
  });

  it('speaks in words, not timecodes', () => {
    expect(formatEta(48)).toBe('48s left');
    expect(formatEta(134)).toBe('2m 14s left');
    expect(formatEta(180)).toBe('3m left');
    expect(formatEta(3780)).toBe('1h 3m left');
    expect(formatEta(7200)).toBe('2h left');
  });
});

describe('appendSample', () => {
  it('starts a series and keeps order', () => {
    expect(appendSample(undefined, 10)).toEqual([10]);
    expect(appendSample([10], 20)).toEqual([10, 20]);
  });

  it('floors anything that is not a positive reading', () => {
    expect(appendSample([], null)).toEqual([0]);
    expect(appendSample([], -1)).toEqual([0]);
  });

  it('never grows past the window', () => {
    let series = [];
    for (let i = 0; i < SPEED_WINDOW * 3; i += 1) series = appendSample(series, i + 1);
    expect(series).toHaveLength(SPEED_WINDOW);
    expect(series[series.length - 1]).toBe(SPEED_WINDOW * 3);
    expect(series[0]).toBe(SPEED_WINDOW * 2 + 1);
  });

  it('returns a new array so React sees the change', () => {
    const series = [1];
    expect(appendSample(series, 2)).not.toBe(series);
    expect(series).toEqual([1]);
  });
});
