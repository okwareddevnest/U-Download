/**
 * Storyboard geometry: turning a timestamp into one tile of one sprite sheet.
 *
 * A storyboard is how a nine-hour recording is navigated at all. yt-dlp reports
 * it as an ordered run of sprite sheets — for the measured YouTube `sb1` track,
 * 135 JPEGs, each a 5x5 grid of 160x90 frames, one frame roughly every ten
 * seconds. `resolve_preview` hands the shape of that track to the frontend
 * (grid, tile size, tile step, and every fragment's own duration and cumulative
 * start); the two functions here are the whole of the mapping from a hovered
 * time to the rectangle to paint.
 *
 * Pure and total: every unusable input returns `null` rather than throwing. A
 * source with no storyboard, a malformed one, or a sheet that will not load
 * must mean no hover frame — never a broken scrub bar.
 */

/**
 * The tile covering `time`, or `null` when the storyboard cannot be used.
 *
 * The sheet is found by its cumulative `start`, not by dividing the timeline
 * evenly: fragment durations are not uniform. On the measured track the last of
 * the 135 sheets covers 39.98s where every other covers 249.90s.
 *
 * The tile *within* the sheet comes from `tile_duration`, which the backend
 * derives from a full fragment. Dividing the final short fragment by its own
 * grid size would spread its four real frames across twenty-five positions and
 * every hover near the end of a long video would land on blank canvas.
 */
export function tileAt(storyboard, time) {
  if (!storyboard) return null;

  const { rows, columns, tile_duration: step, fragments } = storyboard;
  if (!Array.isArray(fragments) || fragments.length === 0) return null;
  if (!(rows > 0) || !(columns > 0) || !(step > 0)) return null;

  const t = Number.isFinite(time) ? Math.max(0, time) : 0;

  // Binary search rather than a scan: this runs on every pointer move across a
  // track that may have several hundred fragments behind it.
  let lo = 0;
  let hi = fragments.length - 1;
  while (lo < hi) {
    const mid = (lo + hi + 1) >> 1;
    if (fragments[mid].start <= t) lo = mid;
    else hi = mid - 1;
  }

  const fragment = fragments[lo];
  if (!fragment?.url) return null;

  const perSheet = rows * columns;
  const raw = Math.floor((t - fragment.start) / step);
  const index = Math.max(0, Math.min(perSheet - 1, raw));

  return {
    url: fragment.url,
    sheet: lo,
    index,
    row: Math.floor(index / columns),
    column: index % columns,
  };
}

/**
 * Inline style that paints one tile by offsetting the sheet as a CSS
 * background — the sheet is scaled to `displayWidth` per tile and shifted so
 * the wanted cell sits in the box. No canvas, no slicing: the browser decodes
 * each sheet once and every later hover into it is a repaint.
 *
 * `displayWidth` is the rendered width of one tile, so a coarse track (`sb2` at
 * 80x45) and a fine one (`sb0` at 320x180) both come out the same size on the
 * scrub bar.
 */
export function tileStyle(storyboard, tile, displayWidth) {
  if (!storyboard || !tile || !(displayWidth > 0)) return null;

  const { rows, columns, tile_width: tw, tile_height: th } = storyboard;
  if (!(tw > 0) || !(th > 0) || !(rows > 0) || !(columns > 0)) return null;

  const displayHeight = (th / tw) * displayWidth;
  // CSS string escaping, for the same reason any interpolated string gets it:
  // a URL carrying a quote or a backslash would otherwise close the literal.
  const href = String(tile.url).replace(/["\\]/g, '\\$&');

  return {
    width: `${round(displayWidth)}px`,
    height: `${round(displayHeight)}px`,
    backgroundImage: `url("${href}")`,
    backgroundSize: `${round(columns * displayWidth)}px ${round(rows * displayHeight)}px`,
    backgroundPosition: `${round(-tile.column * displayWidth)}px ${round(-tile.row * displayHeight)}px`,
    backgroundRepeat: 'no-repeat',
  };
}

// Three decimals: enough that a scaled sheet lands on the same sub-pixel grid
// its offsets were computed against, short enough not to bloat the style string.
const round = (n) => Math.round(n * 1000) / 1000;
