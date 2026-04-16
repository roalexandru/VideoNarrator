import { useState, useCallback } from "react";

const C = { border: "rgba(255,255,255,0.07)", dim: "#8b8ba0" };

/**
 * A numeric input that allows free typing (including deleting all digits).
 * Validates and clamps on blur/Enter. Uses type="text" to avoid browser number input quirks.
 */
export function NumericInput({ value, onChange, min = 0, max = 999, width = 40, color = C.dim, style }: {
  value: number;
  onChange: (v: number) => void;
  min?: number;
  max?: number;
  width?: number;
  color?: string;
  style?: React.CSSProperties;
}) {
  const [editing, setEditing] = useState<string | null>(null);

  const commit = useCallback(() => {
    if (editing === null) return;
    const v = parseFloat(editing);
    if (!isNaN(v)) {
      onChange(Math.max(min, Math.min(max, v)));
    }
    setEditing(null);
  }, [editing, onChange, min, max]);

  return (
    <input
      type="text"
      inputMode="numeric"
      value={editing !== null ? editing : value}
      onChange={(e) => {
        const raw = e.target.value;
        setEditing(raw);
        // Live update for valid values while typing
        const v = parseFloat(raw);
        if (!isNaN(v)) onChange(Math.max(min, Math.min(max, v)));
      }}
      onFocus={(e) => setEditing(e.target.value)}
      onBlur={commit}
      onKeyDown={(e) => { if (e.key === "Enter") { commit(); (e.target as HTMLInputElement).blur(); } }}
      style={{
        width,
        padding: "2px 4px",
        borderRadius: 4,
        border: `1px solid ${C.border}`,
        background: "rgba(255,255,255,0.04)",
        color,
        fontSize: 11,
        fontWeight: 600,
        textAlign: "center" as const,
        outline: "none",
        fontFamily: "inherit",
        ...style,
      }}
    />
  );
}
