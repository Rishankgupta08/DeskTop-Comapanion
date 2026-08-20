/** @type {import('tailwindcss').Config} */
export default {
  content: [
    "./index.html",
    "./src/**/*.{js,ts,jsx,tsx}",
  ],
  theme: {
    extend: {
      colors: {
        darkBg: "#0f0f0f",
        accent: {
          DEFAULT: "#6366f1",
          hover: "#4f46e5",
          light: "#818cf8",
          dark: "#4338ca",
        },
        surface: {
          DEFAULT: "#0f0f0f",
          elevated: "#18181b",
          card: "#1f1f23",
          border: "#27272a",
        },
        status: {
          allow: "#22c55e",
          ask: "#f59e0b",
          off: "#71717a",
          error: "#ef4444",
        },
      },
    },
  },
  plugins: [],
};
