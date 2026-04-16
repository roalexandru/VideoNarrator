/**
 * Inspector controls for each effect type.
 * Spatial controls (position, size) are handled by draggable overlays on the video.
 * This inspector handles non-spatial properties: text content, colors, opacity, blur radius, etc.
 */
import type { TimelineEffect } from "../../stores/editStore";
import { NumericInput } from "./NumericInput";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)" };

function NumInput({ label, value, onChange, min = 0, max = 100, width = 40, color = C.dim }: {
  label: string; value: number; onChange: (v: number) => void;
  min?: number; max?: number; width?: number; color?: string;
}) {
  return (
    <>
      {label && <span style={{ fontSize: 10, color: C.muted }}>{label}</span>}
      <NumericInput value={value} onChange={onChange} min={min} max={max} width={width} color={color}
        style={{ fontSize: 10, padding: "2px 3px" }} />
    </>
  );
}

interface EffectInspectorProps {
  effect: TimelineEffect;
  onUpdate: (partial: Partial<TimelineEffect>) => void;
}

export function EffectInspector({ effect, onUpdate }: EffectInspectorProps) {
  if (effect.type === 'spotlight' && effect.spotlight) {
    const s = effect.spotlight;
    return (
      <>
        <NumInput label="Dim" value={Math.round(s.dimOpacity * 100)} onChange={(v) => onUpdate({ spotlight: { ...s, dimOpacity: v / 100 } })} max={100} />
        <span style={{ fontSize: 10, color: C.muted }}>%</span>
      </>
    );
  }

  if (effect.type === 'blur' && effect.blur) {
    const b = effect.blur;
    return (
      <>
        <NumInput label="Blur" value={b.radius} onChange={(v) => onUpdate({ blur: { ...b, radius: v } })} min={1} max={50} width={36} />
        <span style={{ fontSize: 10, color: C.muted }}>px</span>
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
          onChange={(e) => onUpdate({ text: { ...t, content: e.target.value } })}
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
        <NumInput label="Size %" value={t.fontSize} onChange={(v) => onUpdate({ text: { ...t, fontSize: v } })} min={1} max={20} width={36} />
        {/* Bold / Italic / Underline */}
        <div style={{ display: "flex", gap: 1, background: "rgba(255,255,255,0.03)", borderRadius: 4, padding: 1 }}>
          {toggleBtn(!!t.bold, () => onUpdate({ text: { ...t, bold: !t.bold } }), "B", "Bold")}
          {toggleBtn(!!t.italic, () => onUpdate({ text: { ...t, italic: !t.italic } }), "I", "Italic")}
          {toggleBtn(!!t.underline, () => onUpdate({ text: { ...t, underline: !t.underline } }), "U", "Underline")}
        </div>
        {/* Alignment */}
        <div style={{ display: "flex", gap: 1, background: "rgba(255,255,255,0.03)", borderRadius: 4, padding: 1 }}>
          {(['left', 'center', 'right'] as const).map((a) => (
            <button key={a} onClick={() => onUpdate({ text: { ...t, align: a } })} title={`Align ${a}`} style={{
              padding: "3px 6px", borderRadius: 3, border: "none", fontSize: 9, fontWeight: 600,
              background: (t.align || 'center') === a ? "rgba(16,185,129,0.2)" : "transparent",
              color: (t.align || 'center') === a ? "#10b981" : C.muted, cursor: "pointer", fontFamily: "inherit",
            }}>
              {a === 'left' ? '⫷' : a === 'right' ? '⫸' : '⫿'}
            </button>
          ))}
        </div>
        {/* Text color */}
        <span style={{ fontSize: 9, color: C.muted }}>Text</span>
        <input type="color" value={t.color}
          onChange={(e) => onUpdate({ text: { ...t, color: e.target.value } })}
          title="Text color"
          style={{ width: 22, height: 22, padding: 0, border: `1px solid ${C.border}`, borderRadius: 4, cursor: "pointer", background: "transparent" }}
        />
        {/* Background color */}
        <span style={{ fontSize: 9, color: C.muted }}>Bg</span>
        <input type="color" value={t.background || "#000000"}
          onChange={(e) => onUpdate({ text: { ...t, background: e.target.value } })}
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
          onChange={(e) => onUpdate({ fade: { ...f, color: e.target.value } })}
          style={{ width: 24, height: 24, padding: 0, border: `1px solid ${C.border}`, borderRadius: 4, cursor: "pointer", background: "transparent" }}
        />
        <NumInput label="Opacity" value={Math.round(f.opacity * 100)} onChange={(v) => onUpdate({ fade: { ...f, opacity: v / 100 } })} max={100} />
        <span style={{ fontSize: 10, color: C.muted }}>%</span>
        <span style={{ fontSize: 10, color: C.muted, fontStyle: "italic" }}>Use In/Out transitions for smooth fades.</span>
      </>
    );
  }

  return null;
}
