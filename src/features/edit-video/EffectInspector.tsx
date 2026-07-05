/**
 * Inspector controls for each effect type.
 * Spatial controls (position, size) are handled by draggable overlays on the video.
 * This inspector handles non-spatial properties: text content, colors, opacity, blur radius, etc.
 */
import type { TimelineEffect } from "../../stores/editStore";
import { NumericInput } from "./NumericInput";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)" };

function NumInput({ label, value, onChange, onCommit, min = 0, max = 100, width = 40, color = C.dim }: {
  label: string; value: number; onChange: (v: number) => void; onCommit?: () => void;
  min?: number; max?: number; width?: number; color?: string;
}) {
  return (
    <>
      {label && <span style={{ fontSize: 10, color: C.muted }}>{label}</span>}
      <NumericInput value={value} onChange={onChange} onCommit={onCommit} min={min} max={max} width={width} color={color}
        style={{ fontSize: 10, padding: "2px 3px" }} />
    </>
  );
}

interface EffectInspectorProps {
  /** One-shot committed update (its own undo entry). For discrete controls:
   *  buttons, selects, toggles. */
  effect: TimelineEffect;
  onUpdate: (partial: Partial<TimelineEffect>) => void;
  /** Continuous update during an interaction (color-picker drag, typing a
   *  number) — does NOT push undo. Pair with onCommit at the interaction end.
   *  Without this, each change event pushed a separate undo entry, so a single
   *  color-picker drag flooded the 30-slot stack and wiped history. */
  onLiveUpdate: (partial: Partial<TimelineEffect>) => void;
  /** Ends a live interaction, pushing exactly one undo entry. */
  onCommit: () => void;
}

export function EffectInspector({ effect, onUpdate, onLiveUpdate, onCommit }: EffectInspectorProps) {
  if (effect.type === 'spotlight' && effect.spotlight) {
    const s = effect.spotlight;
    return (
      <>
        <NumInput label="Dim" value={Math.round(s.dimOpacity * 100)} onChange={(v) => onLiveUpdate({ spotlight: { ...s, dimOpacity: v / 100 } })} onCommit={onCommit} max={100} />
        <span style={{ fontSize: 10, color: C.muted }}>%</span>
      </>
    );
  }

  if (effect.type === 'blur' && effect.blur) {
    const b = effect.blur;
    return (
      <>
        {/* Strength is stored normalized (0,1] = blur radius as a fraction of
            video width, so the exported blur is as strong as the preview at any
            resolution. Displayed 1–100 = tenths of a percent of width. */}
        <NumInput label="Strength" value={Math.round((b.radius <= 1 ? b.radius : 0.03) * 1000)} onChange={(v) => onLiveUpdate({ blur: { ...b, radius: v / 1000 } })} onCommit={onCommit} min={1} max={100} width={36} />
        <div style={{ width: 1, height: 20, background: C.border }} />
        <div style={{ display: "flex", gap: 2, background: "rgba(255,255,255,0.03)", borderRadius: 5, padding: 2 }}>
          <button onClick={() => onUpdate({ blur: { ...b, invert: false } })} style={{
            padding: "3px 8px", borderRadius: 4, border: "none", fontSize: 10, fontWeight: 600,
            background: !b.invert ? "rgba(139,92,246,0.2)" : "transparent",
            color: !b.invert ? "#8b5cf6" : C.muted, cursor: "pointer", fontFamily: "inherit",
          }}>Blur Region</button>
          <button onClick={() => onUpdate({ blur: { ...b, invert: true } })} style={{
            padding: "3px 8px", borderRadius: 4, border: "none", fontSize: 10, fontWeight: 600,
            background: b.invert ? "rgba(139,92,246,0.2)" : "transparent",
            color: b.invert ? "#8b5cf6" : C.muted, cursor: "pointer", fontFamily: "inherit",
          }}>Blur Everything Else</button>
        </div>
      </>
    );
  }

  if (effect.type === 'text' && effect.text) {
    const t = effect.text;
    const fonts = [
      { value: "Inter, system-ui, sans-serif", label: "Inter" },
      { value: "Georgia, serif", label: "Georgia" },
      { value: "'Courier New', monospace", label: "Courier" },
      { value: "Impact, sans-serif", label: "Impact" },
      { value: "'Arial Black', sans-serif", label: "Arial Black" },
      { value: "'Trebuchet MS', sans-serif", label: "Trebuchet" },
      { value: "'Times New Roman', serif", label: "Times" },
    ];
    const toggleBtn = (active: boolean, onClick: () => void, label: string, title: string) => (
      <button onClick={onClick} title={title} style={{
        padding: "3px 7px", borderRadius: 4, border: "none", fontSize: 11, fontWeight: active ? 700 : 400,
        background: active ? "rgba(16,185,129,0.2)" : "transparent",
        color: active ? "#10b981" : C.muted, cursor: "pointer", fontFamily: "inherit",
        fontStyle: label === "I" ? "italic" : "normal",
        textDecoration: label === "U" ? "underline" : "none",
      }}>{label}</button>
    );
    return (
      <>
        {/* Text content */}
        <input
          type="text"
          value={t.content}
          onChange={(e) => onLiveUpdate({ text: { ...t, content: e.target.value } })}
          onBlur={onCommit}
          placeholder="Enter text..."
          style={{
            width: 150, padding: "4px 8px", borderRadius: 4,
            border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.06)",
            color: C.text, fontSize: 12, outline: "none", fontFamily: "inherit",
          }}
        />
        {/* Font family */}
        <select
          value={t.fontFamily || fonts[0].value}
          onChange={(e) => onUpdate({ text: { ...t, fontFamily: e.target.value } })}
          style={{
            padding: "3px 4px", borderRadius: 4, border: `1px solid ${C.border}`,
            background: "rgba(255,255,255,0.04)", color: C.dim, fontSize: 10,
            outline: "none", fontFamily: "inherit", cursor: "pointer", maxWidth: 80,
          }}
        >
          {fonts.map((f) => <option key={f.value} value={f.value}>{f.label}</option>)}
        </select>
        {/* Size */}
        <NumInput label="Size %" value={t.fontSize} onChange={(v) => onLiveUpdate({ text: { ...t, fontSize: v } })} onCommit={onCommit} min={1} max={20} width={36} />
        {/* Bold. Italic/underline/alignment are intentionally omitted: ffmpeg
            drawtext (the export path) has no underline or alignment option and
            ignores font-style on non-fontconfig builds, so exposing them would
            promise styling the exported video can't reproduce. Bold stays — it
            maps to a heavier border in the export. */}
        <div style={{ display: "flex", gap: 1, background: "rgba(255,255,255,0.03)", borderRadius: 4, padding: 1 }}>
          {toggleBtn(!!t.bold, () => onUpdate({ text: { ...t, bold: !t.bold } }), "B", "Bold")}
        </div>
        {/* Text color */}
        <span style={{ fontSize: 9, color: C.muted }}>Text</span>
        <input type="color" value={t.color}
          onChange={(e) => onLiveUpdate({ text: { ...t, color: e.target.value } })}
          onBlur={onCommit}
          title="Text color"
          style={{ width: 22, height: 22, padding: 0, border: `1px solid ${C.border}`, borderRadius: 4, cursor: "pointer", background: "transparent" }}
        />
        {/* Background color */}
        <span style={{ fontSize: 9, color: C.muted }}>Bg</span>
        <input type="color" value={t.background || "#000000"}
          onChange={(e) => onLiveUpdate({ text: { ...t, background: e.target.value } })}
          onBlur={onCommit}
          title="Background color"
          style={{ width: 22, height: 22, padding: 0, border: `1px solid ${C.border}`, borderRadius: 4, cursor: "pointer", background: "transparent" }}
        />
        <button onClick={() => onUpdate({ text: { ...t, background: t.background ? '' : 'rgba(0,0,0,0.6)' } })}
          title={t.background ? "Remove background" : "Add background"}
          style={{
            padding: "2px 6px", borderRadius: 4, border: "none", fontSize: 9, fontWeight: 600,
            background: t.background ? "rgba(16,185,129,0.15)" : "transparent",
            color: t.background ? "#10b981" : C.muted, cursor: "pointer", fontFamily: "inherit",
          }}>
          {t.background ? "BG On" : "BG Off"}
        </button>
      </>
    );
  }

  if (effect.type === 'fade' && effect.fade) {
    const f = effect.fade;
    return (
      <>
        <span style={{ fontSize: 10, color: C.muted }}>Color</span>
        <input
          type="color"
          value={f.color}
          onChange={(e) => onLiveUpdate({ fade: { ...f, color: e.target.value } })}
          onBlur={onCommit}
          style={{ width: 24, height: 24, padding: 0, border: `1px solid ${C.border}`, borderRadius: 4, cursor: "pointer", background: "transparent" }}
        />
        <NumInput label="Opacity" value={Math.round(f.opacity * 100)} onChange={(v) => onLiveUpdate({ fade: { ...f, opacity: v / 100 } })} onCommit={onCommit} max={100} />
        <span style={{ fontSize: 10, color: C.muted }}>%</span>
        <span style={{ fontSize: 10, color: C.muted, fontStyle: "italic" }}>Use In/Out transitions for smooth fades.</span>
      </>
    );
  }

  return null;
}
