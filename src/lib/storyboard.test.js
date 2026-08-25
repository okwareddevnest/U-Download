import { describe, it, expect } from 'vitest';
import { tileAt, tileStyle } from './storyboard';

// The measured `sb1` track of the reported nine-hour livestream, shortened to
// three sheets: 5x5 grids of 160x90 tiles, 249.8956s per sheet — except the
// last, which covers the leftover 39.98s.
const STEP = 249.89564698867025 / 25; // 9.9958s per tile
const sb1 = {
  format_id: 'sb1',
  rows: 5,
  columns: 5,
  tile_width: 160,
  tile_height: 90,
  tile_duration: STEP,
  fragments: [
    { url: 'https://sb/M0.jpg', duration: 249.89564698867025, start: 0 },
    { url: 'https://sb/M1.jpg', duration: 249.89564698867025, start: 249.89564698867025 },
    { url: 'https://sb/M2.jpg', duration: 39.98330351818731, start: 499.7912939773405 },
  ],
};

describe('tileAt', () => {
  it('maps the start of the video to the first tile of the first sheet', () => {
    expect(tileAt(sb1, 0)).toEqual({
      url: 'https://sb/M0.jpg',
      sheet: 0,
      index: 0,
      row: 0,
      column: 0,
    });
  });

  it('walks across a row and then down to the next', () => {
    expect(tileAt(sb1, STEP * 1.5)).toMatchObject({ index: 1, row: 0, column: 1 });
    expect(tileAt(sb1, STEP * 4.5)).toMatchObject({ index: 4, row: 0, column: 4 });
    expect(tileAt(sb1, STEP * 5.5)).toMatchObject({ index: 5, row: 1, column: 0 });
    expect(tileAt(sb1, STEP * 24.5)).toMatchObject({ index: 24, row: 4, column: 4 });
  });

  it('crosses into the next sheet at that sheet\'s own start', () => {
    expect(tileAt(sb1, 249.0)).toMatchObject({ sheet: 0, index: 24 });
    expect(tileAt(sb1, 250.0)).toMatchObject({ url: 'https://sb/M1.jpg', sheet: 1, index: 0 });
  });

  // The tile step comes from a full fragment, so the four real frames on the
  // short final sheet sit at 0..3 — not spread across all twenty-five cells.
  it('indexes the short final sheet on the same tile step as the others', () => {
    const last = sb1.fragments[2].start;
    expect(tileAt(sb1, last + 1)).toMatchObject({ sheet: 2, index: 0 });
    expect(tileAt(sb1, last + STEP * 3.5)).toMatchObject({ sheet: 2, index: 3 });
  });

  it('clamps a time past the end of the storyboard into the last sheet', () => {
    expect(tileAt(sb1, 999999)).toMatchObject({ sheet: 2, index: 24 });
  });

  it('clamps a negative or non-finite time to the first tile', () => {
    expect(tileAt(sb1, -50)).toMatchObject({ sheet: 0, index: 0 });
    expect(tileAt(sb1, NaN)).toMatchObject({ sheet: 0, index: 0 });
    expect(tileAt(sb1, Infinity)).toMatchObject({ sheet: 0, index: 0 });
  });

  // Every one of these must mean "no hover frame", never a thrown error: the
  // scrub bar has to keep working for sources that publish no storyboard.
  it('returns null for anything unusable', () => {
    expect(tileAt(null, 10)).toBeNull();
    expect(tileAt(undefined, 10)).toBeNull();
    expect(tileAt({ ...sb1, fragments: [] }, 10)).toBeNull();
    expect(tileAt({ ...sb1, fragments: undefined }, 10)).toBeNull();
    expect(tileAt({ ...sb1, rows: 0 }, 10)).toBeNull();
    expect(tileAt({ ...sb1, columns: 0 }, 10)).toBeNull();
    expect(tileAt({ ...sb1, tile_duration: 0 }, 10)).toBeNull();
    expect(tileAt({ ...sb1, fragments: [{ start: 0, duration: 10 }] }, 10)).toBeNull();
  });
});

describe('tileStyle', () => {
  it('offsets the sheet so the wanted cell fills the box', () => {
    const tile = tileAt(sb1, STEP * 6.5); // index 6 -> row 1, column 1
    const style = tileStyle(sb1, tile, 160);
    expect(style).toMatchObject({
      width: '160px',
      height: '90px',
      backgroundImage: 'url("https://sb/M0.jpg")',
      backgroundSize: '800px 450px',
      backgroundPosition: '-160px -90px',
      backgroundRepeat: 'no-repeat',
    });
  });

  it('scales a coarse or fine track to the same displayed size', () => {
    const sb0 = { ...sb1, tile_width: 320, tile_height: 180, rows: 3, columns: 3 };
    const style = tileStyle(sb0, tileAt(sb0, 0), 160);
    expect(style).toMatchObject({ width: '160px', height: '90px', backgroundSize: '480px 270px' });

    const sb2 = { ...sb1, tile_width: 80, tile_height: 45, rows: 10, columns: 10 };
    const coarse = tileStyle(sb2, tileAt(sb2, 0), 160);
    expect(coarse).toMatchObject({ width: '160px', height: '90px', backgroundSize: '1600px 900px' });
  });

  it('keeps a non-16:9 tile at its own aspect ratio', () => {
    const square = { ...sb1, tile_width: 100, tile_height: 100 };
    expect(tileStyle(square, tileAt(square, 0), 160)).toMatchObject({
      width: '160px',
      height: '160px',
    });
  });

  it('escapes a URL that would otherwise close the CSS literal', () => {
    const tile = { url: 'https://sb/a"b\\c.jpg', sheet: 0, index: 0, row: 0, column: 0 };
    expect(tileStyle(sb1, tile, 160).backgroundImage).toBe('url("https://sb/a\\"b\\\\c.jpg")');
  });

  it('returns null for anything unusable', () => {
    expect(tileStyle(null, tileAt(sb1, 0), 160)).toBeNull();
    expect(tileStyle(sb1, null, 160)).toBeNull();
    expect(tileStyle(sb1, tileAt(sb1, 0), 0)).toBeNull();
    expect(tileStyle({ ...sb1, tile_width: 0 }, tileAt(sb1, 0), 160)).toBeNull();
    expect(tileStyle({ ...sb1, tile_height: 0 }, tileAt(sb1, 0), 160)).toBeNull();
  });
});
