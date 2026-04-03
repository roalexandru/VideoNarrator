import { type CSSProperties, useState } from "react";

interface ButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: "primary" | "secondary" | "ghost" | "danger";
  size?: "sm" | "md" | "lg";
}

const sizes: Record<string, CSSProperties> = {
  sm: { padding: "5px 10px", fontSize: 12 },
  md: { padding: "8px 16px", fontSize: 13 },
  lg: { padding: "10px 24px", fontSize: 14 },
};

export function Button({ variant = "primary", size = "md", style, disabled, children, ...props }: ButtonProps) {
  const [hover, setHover] = useState(false);

  const base: CSSProperties = {
    display: "inline-flex", alignItems: "center", justifyContent: "center", gap: 6,
    borderRadius: 8, border: "none", fontWeight: 600,
    cursor: disabled ? "not-allowed" : "pointer",
    opacity: disabled ? 0.4 : 1, transition: "all 0.12s",
    fontFamily: "inherit", ...sizes[size],
  };

  const v: Record<string, CSSProperties> = {
    primary: {
      background: hover && !disabled ? "linear-gradient(135deg,#5558e6,#7c4fd4)" : "linear-gradient(135deg,#6366f1,#8b5cf6)",
      color: "#fff", boxShadow: hover && !disabled ? "0 4px 20px rgba(99,102,241,0.4)" : "0 2px 8px rgba(99,102,241,0.2)",
    },
    secondary: {
      background: hover && !disabled ? "rgba(255,255,255,0.08)" : "rgba(255,255,255,0.05)",
      color: "#b0b0c0", border: "1px solid rgba(255,255,255,0.1)",
    },
    ghost: {
      background: hover && !disabled ? "rgba(255,255,255,0.06)" : "transparent",
      color: "#8b8ba0",
    },
    danger: {
      background: hover && !disabled ? "#dc2626" : "rgba(239,68,68,0.15)",
      color: hover && !disabled ? "#fff" : "#ef4444",
    },
  };

  return (
    <button disabled={disabled} style={{ ...base, ...v[variant], ...style }}
      onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)} {...props}>
      {children}
    </button>
  );
}
