export function ProgressBar({ value, height = 6 }: { value: number; height?: number }) {
  return (
    <div style={{ height, width: "100%", borderRadius: height, background: "rgba(255,255,255,0.06)", overflow: "hidden" }}>
      <div style={{
        height: "100%", width: `${Math.min(100, Math.max(0, value))}%`,
        borderRadius: height, background: "linear-gradient(90deg,#6366f1,#a855f7)",
        transition: "width 0.4s ease",
        boxShadow: "0 0 12px rgba(99,102,241,0.3)",
      }} />
    </div>
  );
}
