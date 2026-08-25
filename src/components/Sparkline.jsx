import { formatSpeed } from '../lib/format';

/**
 * A download's throughput trace: one series, one row, no chart furniture.
 *
 * Geometry matches the icon set's discipline — a fixed box, a 2px stroke in
 * `currentColor`, round caps and joins — so the line reads as part of the same
 * hand as everything else in the window.
 *
 * The y-axis runs from zero to the peak of the window on screen. Zero is
 * pinned rather than floating on the window's minimum for one reason: a stall
 * has to look like a stall. With a floating floor, a download wobbling between
 * 4.9 and 5.1 MB/s would draw a mountain range, and one that dropped to nothing
 * would draw the same shape as one that never slowed. The cost is that the top
 * of the box means "this job's best so far", not a fixed rate — which is why
 * the actual number is printed beside the line rather than left to be read off
 * a height. Nothing here is a value the user has to measure with their eye.
 *
 * There is deliberately no axis, no gridline, no marker per sample and no
 * hover: at 84x22 inside a queue row, each of those is noise standing between
 * the reader and a shape.
 */

const WIDTH = 84;
const HEIGHT = 22;
const STROKE = 2;
// Half the stroke, so the line's edge sits inside the box instead of being
// clipped by it at the extremes.
const INSET = STROKE / 2;

export default function Sparkline({ samples, className = '' }) {
  if (!samples || samples.length < 2) return null;

  const peak = samples.reduce((max, v) => (v > max ? v : max), 0);
  if (peak <= 0) return null;

  const innerW = WIDTH - INSET * 2;
  const innerH = HEIGHT - INSET * 2;
  const step = innerW / (samples.length - 1);

  const d = samples
    .map((value, i) => {
      const x = INSET + i * step;
      const y = INSET + innerH - (Math.max(0, value) / peak) * innerH;
      return `${i === 0 ? 'M' : 'L'}${x.toFixed(2)} ${y.toFixed(2)}`;
    })
    .join(' ');

  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox={`0 0 ${WIDTH} ${HEIGHT}`}
      width={WIDTH}
      height={HEIGHT}
      role="img"
      aria-label={`Download speed over the last ${samples.length} readings, peaking at ${formatSpeed(peak)}`}
      className={`block text-accent ${className}`}
    >
      <path
        d={d}
        fill="none"
        stroke="currentColor"
        strokeWidth={STROKE}
        strokeLinecap="round"
        strokeLinejoin="round"
      />
    </svg>
  );
}
