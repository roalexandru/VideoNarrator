import { useEffect, useRef } from "react";
import { colors } from "../../lib/theme";

interface Props {
  title: string;
  message: string;
  confirmLabel?: string;
  cancelLabel?: string;
  danger?: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}

export function ConfirmDialog({ title, message, confirmLabel = "Delete", cancelLabel = "Cancel", danger = true, onConfirm, onCancel }: Props) {
  const cancelRef = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    cancelRef.current?.focus();
    const handleKey = (e: KeyboardEvent) => { if (e.key === "Escape") onCancel(); };
    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [onCancel]);

  return (
    <div role="dialog" aria-modal="true" aria-labelledby="confirm-dialog-title" style={{
      position: "fixed", inset: 0, zIndex: 9500, background: "rgba(0,0,0,0.5)", backdropFilter: "blur(4px)",
      display: "flex", alignItems: "center", justifyContent: "center",
    }} onClick={(e) => { if (e.target === e.currentTarget) onCancel(); }}>
      <div style={{
        background: colors.bg.card, borderRadius: 14, border: `1px solid ${colors.border.default}`,
        boxShadow: "0 24px 64px rgba(0,0,0,0.5)", padding: "24px 28px", maxWidth: 380, width: "100%",
      }}>
        <h3 id="confirm-dialog-title" style={{ fontSize: 17, fontWeight: 700, color: colors.text.primary, marginBottom: 8 }}>{title}</h3>
        <p style={{ fontSize: 14, color: colors.text.secondary, lineHeight: 1.5, marginBottom: 24 }}>{message}</p>
        <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
          <button ref={cancelRef} onClick={onCancel} style={{
            padding: "8px 18px", borderRadius: 8, border: `1px solid ${colors.border.default}`,
            background: "transparent", color: colors.text.secondary, fontSize: 14, cursor: "pointer", fontFamily: "inherit",
          }}>{cancelLabel}</button>
          <button onClick={onConfirm} style={{
            padding: "8px 18px", borderRadius: 8, border: "none",
            background: danger ? colors.accent.red : colors.accent.gradient,
            color: "#fff", fontSize: 14, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
          }}>{confirmLabel}</button>
        </div>
      </div>
    </div>
  );
}
