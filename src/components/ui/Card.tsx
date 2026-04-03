import { useState, type CSSProperties } from "react";

interface CardProps {
  children: React.ReactNode;
  style?: CSSProperties;
  selected?: boolean;
  onClick?: () => void;
}

export function Card({ children, style: s, selected, onClick }: CardProps) {
  const [hover, setHover] = useState(false);

  return (
    <div onClick={onClick}
      style={{
        borderRadius: 10, padding: 16,
        border: selected ? "1px solid rgba(99,102,241,0.5)" : `1px solid rgba(255,255,255,${hover && onClick ? "0.1" : "0.06"})`,
        background: selected ? "rgba(99,102,241,0.08)" : hover && onClick ? "rgba(255,255,255,0.04)" : "rgba(255,255,255,0.02)",
        cursor: onClick ? "pointer" : "default",
        transition: "all 0.12s",
        boxShadow: selected ? "0 0 0 1px rgba(99,102,241,0.2), 0 4px 12px rgba(99,102,241,0.08)" : "none",
        ...s,
      }}
      onMouseEnter={() => setHover(true)} onMouseLeave={() => setHover(false)}>
      {children}
    </div>
  );
}
