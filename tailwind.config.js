/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      // Every colour resolves through a CSS variable so the light and dark
      // palettes are two authored sets, not one set inverted. See App.css.
      colors: {
        canvas: "var(--c-canvas)",
        panel: "var(--c-panel)",
        sunken: "var(--c-sunken)",
        stage: "var(--c-stage)",
        hair: "var(--c-hair)",
        "hair-strong": "var(--c-hair-strong)",
        fg: "var(--c-fg)",
        "fg-muted": "var(--c-fg-muted)",
        accent: "var(--c-accent)",
        "accent-hover": "var(--c-accent-hover)",
        "accent-fg": "var(--c-accent-fg)",
        "accent-soft": "var(--c-accent-soft)",
        ok: "var(--c-ok)",
        danger: "var(--c-danger)",
        "danger-soft": "var(--c-danger-soft)",
      },
      fontFamily: {
        sans: [
          "-apple-system",
          "BlinkMacSystemFont",
          "Segoe UI Variable Text",
          "Segoe UI",
          "Ubuntu",
          "Cantarell",
          "Noto Sans",
          "sans-serif",
        ],
      },
      fontSize: {
        // A tight product scale. 11px is the floor and is used only for
        // uppercase section labels and secondary metrics.
        micro: ["0.6875rem", { lineHeight: "1rem", letterSpacing: "0.08em" }],
        meta: ["0.75rem", { lineHeight: "1.125rem" }],
        ui: ["0.8125rem", { lineHeight: "1.25rem" }],
        body: ["0.875rem", { lineHeight: "1.375rem" }],
        lead: ["1rem", { lineHeight: "1.5rem" }],
      },
      borderRadius: {
        field: "5px",
      },
      transitionTimingFunction: {
        out: "cubic-bezier(0.2, 0.8, 0.2, 1)",
      },
    },
  },
  plugins: [],
}
