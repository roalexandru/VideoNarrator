// Centralized design tokens for the Narrator app

export const colors = {
  // Backgrounds
  bg: {
    app: "#0c0c10",
    sidebar: "#111118",
    card: "#16161e",
    raised: "#1e1e28",
    input: "rgba(255,255,255,0.04)",
    hover: "rgba(255,255,255,0.06)",
  },
  // Borders
  border: {
    default: "rgba(255,255,255,0.07)",
    hover: "rgba(255,255,255,0.14)",
    focus: "rgba(99,102,241,0.4)",
    selected: "rgba(99,102,241,0.5)",
  },
  // Text
  text: {
    primary: "#e0e0ea",
    secondary: "#8b8ba0",
    muted: "#5a5a6e",
    disabled: "#3a3a4a",
  },
  // Accent
  accent: {
    primary: "#818cf8",
    purple: "#a855f7",
    green: "#4ade80",
    red: "#ef4444",
    orange: "#f59e0b",
    gradient: "linear-gradient(135deg, #6366f1, #8b5cf6)",
  },
} as const;

export const spacing = {
  xs: 4,
  sm: 8,
  md: 12,
  lg: 16,
  xl: 20,
  xxl: 28,
  section: 28,    // between sections
  labelGap: 8,    // label to content
  inlineGap: 6,   // between inline elements
} as const;

export const typography = {
  pageTitle: { fontSize: 22, fontWeight: 700, letterSpacing: -0.3 } as const,
  sectionLabel: { fontSize: 11, fontWeight: 700, textTransform: "uppercase" as const, letterSpacing: 1.2 } as const,
  body: { fontSize: 13, fontWeight: 400 } as const,
  caption: { fontSize: 11, fontWeight: 400 } as const,
  mono: { fontFamily: "'SF Mono', 'Cascadia Code', 'Segoe UI Mono', Menlo, 'Courier New', monospace" } as const,
} as const;
