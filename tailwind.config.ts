import type { Config } from "tailwindcss";

export default {
  content: ["./index.html", "./src/**/*.{ts,tsx}"],
  darkMode: "class",
  theme: {
    extend: {
      colors: {
        bg: {
          DEFAULT: "rgb(var(--bg) / <alpha-value>)",
          subtle: "rgb(var(--bg-subtle) / <alpha-value>)",
          muted: "rgb(var(--bg-muted) / <alpha-value>)",
        },
        fg: {
          DEFAULT: "rgb(var(--fg) / <alpha-value>)",
          subtle: "rgb(var(--fg-subtle) / <alpha-value>)",
          muted: "rgb(var(--fg-muted) / <alpha-value>)",
        },
        accent: "rgb(var(--accent) / <alpha-value>)",
        border: "rgb(var(--border) / <alpha-value>)",
        success: "rgb(34 197 94 / <alpha-value>)",
        warn: "rgb(234 179 8 / <alpha-value>)",
        danger: "rgb(239 68 68 / <alpha-value>)",
      },
      fontFamily: {
        sans: ["Inter", "system-ui", "sans-serif"],
        mono: ["JetBrains Mono", "ui-monospace", "monospace"],
      },
    },
  },
} satisfies Config;
