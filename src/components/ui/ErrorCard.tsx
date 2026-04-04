import { colors } from "../../lib/theme";

interface Props {
  error: string;
  suggestion?: string;
  actionLabel?: string;
  onAction?: () => void;
  onDismiss?: () => void;
}

export function ErrorCard({ error, suggestion, actionLabel, onAction, onDismiss }: Props) {
  return (
    <div style={{
      padding: "14px 18px", borderRadius: 10,
      background: "rgba(239,68,68,0.08)", border: "1px solid rgba(239,68,68,0.2)",
    }}>
      <div style={{ display: "flex", alignItems: "flex-start", gap: 10 }}>
        <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={colors.accent.red} strokeWidth="2" strokeLinecap="round" style={{ flexShrink: 0, marginTop: 2 }}>
          <circle cx="12" cy="12" r="10"/><line x1="15" y1="9" x2="9" y2="15"/><line x1="9" y1="9" x2="15" y2="15"/>
        </svg>
        <div style={{ flex: 1 }}>
          <p style={{ fontSize: 13, color: colors.accent.red, fontWeight: 600, marginBottom: suggestion ? 4 : 0 }}>{error}</p>
          {suggestion && <p style={{ fontSize: 12, color: colors.text.secondary, lineHeight: 1.4 }}>{suggestion}</p>}
        </div>
        {onDismiss && (
          <button onClick={onDismiss} style={{ background: "none", border: "none", color: colors.text.muted, cursor: "pointer", fontSize: 14, padding: 2 }}>&times;</button>
        )}
      </div>
      {actionLabel && onAction && (
        <button onClick={onAction} style={{
          marginTop: 10, padding: "6px 14px", borderRadius: 6, border: "none",
          background: "rgba(239,68,68,0.15)", color: colors.accent.red,
          fontSize: 12, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
        }}>{actionLabel}</button>
      )}
    </div>
  );
}
