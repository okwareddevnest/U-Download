/**
 * The icon set, drawn here rather than pulled from a library.
 *
 * One geometry for all of them: a 24-unit box rendered at 20px, 1.5 stroke in
 * `currentColor`, round caps and joins, no fills. Anything that cannot be said
 * clearly at that weight is said in words instead, which is why this set is
 * short: icons appear only where a control is too small for a label or where
 * the shape is faster to read than the word.
 */

function Glyph({ size = 20, className = '', children, ...rest }) {
  return (
    <svg
      xmlns="http://www.w3.org/2000/svg"
      viewBox="0 0 24 24"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth="1.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      focusable="false"
      className={className}
      {...rest}
    >
      {children}
    </svg>
  );
}

export const IconChevronDown = (props) => (
  <Glyph {...props}>
    <path d="M6 9.5 12 15.5 18 9.5" />
  </Glyph>
);

export const IconDownload = (props) => (
  <Glyph {...props}>
    <path d="M12 3.5v11" />
    <path d="M7.5 10.5 12 15l4.5-4.5" />
    <path d="M4.5 18.5h15" />
  </Glyph>
);

export const IconScissors = (props) => (
  <Glyph {...props}>
    <circle cx="6.5" cy="17.5" r="2.5" />
    <circle cx="17.5" cy="17.5" r="2.5" />
    <path d="M8.3 15.7 19 4" />
    <path d="M15.7 15.7 5 4" />
  </Glyph>
);

export const IconPlay = (props) => (
  <Glyph {...props}>
    <path d="M8 5.5 18.5 12 8 18.5Z" />
  </Glyph>
);

export const IconPause = (props) => (
  <Glyph {...props}>
    <path d="M9.5 5.5v13" />
    <path d="M14.5 5.5v13" />
  </Glyph>
);

export const IconSound = (props) => (
  <Glyph {...props}>
    <path d="M4.5 9.5h3L12 5.5v13L7.5 14.5h-3Z" />
    <path d="M15.5 9.5a3.5 3.5 0 0 1 0 5" />
    <path d="M18 7a7 7 0 0 1 0 10" />
  </Glyph>
);

export const IconSoundOff = (props) => (
  <Glyph {...props}>
    <path d="M4.5 9.5h3L12 5.5v13L7.5 14.5h-3Z" />
    <path d="M16 10l4 4" />
    <path d="M20 10l-4 4" />
  </Glyph>
);

export const IconFolder = (props) => (
  <Glyph {...props}>
    <path d="M3.5 6.5a1 1 0 0 1 1-1h4l2 2.5h8a1 1 0 0 1 1 1v9a1 1 0 0 1-1 1h-14a1 1 0 0 1-1-1Z" />
  </Glyph>
);

export const IconSun = (props) => (
  <Glyph {...props}>
    <circle cx="12" cy="12" r="4" />
    <path d="M12 3v2" />
    <path d="M12 19v2" />
    <path d="M3 12h2" />
    <path d="M19 12h2" />
    <path d="M5.6 5.6 7 7" />
    <path d="M17 17l1.4 1.4" />
    <path d="M18.4 5.6 17 7" />
    <path d="M7 17l-1.4 1.4" />
  </Glyph>
);

export const IconMoon = (props) => (
  <Glyph {...props}>
    <path d="M19 14.5A7.5 7.5 0 0 1 9.5 5a7.5 7.5 0 1 0 9.5 9.5Z" />
  </Glyph>
);

export const IconQueue = (props) => (
  <Glyph {...props}>
    <path d="M4 7h11" />
    <path d="M4 12h16" />
    <path d="M4 17h7" />
  </Glyph>
);
