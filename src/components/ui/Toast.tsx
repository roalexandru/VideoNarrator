import { useState, useEffect, useCallback } from "react";
import { colors } from "../../lib/theme";

interface ToastItem { id: number; message: string; type: "success" | "error" | "info"; }

let toastId = 0;
let addToastFn: ((msg: string, type: "success" | "error" | "info") => void) | null = null;

export function showToast(message: string, type: "success" | "error" | "info" = "info") {
  addToastFn?.(message, type);
}

export function ToastContainer() {
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  const addToast = useCallback((message: string, type: "success" | "error" | "info") => {
    const id = ++toastId;
    setToasts((prev) => [...prev, { id, message, type }]);
    setTimeout(() => setToasts((prev) => prev.filter((t) => t.id !== id)), 4000);
  }, []);

  useEffect(() => { addToastFn = addToast; return () => { addToastFn = null; }; }, [addToast]);

  if (toasts.length === 0) return null;

  const typeColors = {
    success: { bg: "rgba(34,197,94,0.12)", border: "rgba(34,197,94,0.25)", text: colors.accent.green },
    error: { bg: "rgba(239,68,68,0.12)", border: "rgba(239,68,68,0.25)", text: colors.accent.red },
    info: { bg: "rgba(99,102,241,0.12)", border: "rgba(99,102,241,0.25)", text: colors.accent.primary },
  };

  return (
    <div style={{ position: "fixed", bottom: 20, right: 20, zIndex: 9000, display: "flex", flexDirection: "column", gap: 8 }}>
      {toasts.map((t) => {
        const c = typeColors[t.type];
        return (
          <div key={t.id} style={{
            padding: "10px 16px", borderRadius: 10, background: c.bg, border: `1px solid ${c.border}`,
            color: c.text, fontSize: 13, fontWeight: 500, maxWidth: 360,
            animation: "toastIn 0.25s ease-out",
            boxShadow: "0 8px 24px rgba(0,0,0,0.3)",
          }}>
            {t.message}
          </div>
        );
      })}
      <style>{`@keyframes toastIn { from { opacity:0; transform:translateY(10px); } to { opacity:1; transform:translateY(0); } }`}</style>
    </div>
  );
}
