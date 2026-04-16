/**
 * Renders visual overlays for active timeline effects on top of the video preview.
 * When an effect is selected, its region becomes interactive (draggable/resizable).
 */
import { useRef, useCallback } from "react";
import type { TimelineEffect } from "../../stores/editStore";
import { effectOpacity as calcEffectOpacity } from "./easing";

interface EffectsOverlayProps {
  effects: TimelineEffect[];
  outputTime: number;
  videoWidth: number;
  videoHeight: number;
  selectedEffectId: string | null;
  onUpdateEffect: (id: string, partial: Partial<TimelineEffect>) => void;
  onUpdateEffectLive: (id: string, partial: Partial<TimelineEffect>) => void;
  onCommitEffect: () => void;
}

function getEffectOpacity(effect: TimelineEffect, outputTime: number): number {
  const localTime = outputTime - effect.startTime;
  const duration = effect.endTime - effect.startTime;
  return calcEffectOpacity(localTime, duration, effect.transitionIn ?? 0, effect.transitionOut ?? 0, effect.reverse ?? false);
}

// ── Draggable Rectangle (for blur) ──

function DraggableRect({ x, y, width, height, color, videoW, videoH, isSelected, label, onMove, onCommit }: {
  x: number; y: number; width: number; height: number; color: string;
  videoW: number; videoH: number; isSelected: boolean; label: string;
  onMove: (x: number, y: number, w: number, h: number) => void; onCommit: () => void;
}) {
  const dragRef = useRef<{ startX: number; startY: number; ox: number; oy: number; ow: number; oh: number; mode: 'move' | 'nw' | 'ne' | 'sw' | 'se' } | null>(null);

  const startDrag = useCallback((e: React.MouseEvent, mode: 'move' | 'nw' | 'ne' | 'sw' | 'se') => {
    e.preventDefault(); e.stopPropagation();
    dragRef.current = { startX: e.clientX, startY: e.clientY, ox: x, oy: y, ow: width, oh: height, mode };
    const onMouseMove = (me: MouseEvent) => {
      if (!dragRef.current) return;
      const dx = (me.clientX - dragRef.current.startX) / videoW;
      const dy = (me.clientY - dragRef.current.startY) / videoH;
      const { ox, oy, ow, oh, mode: m } = dragRef.current;
      if (m === 'move') {
        onMove(Math.max(0, Math.min(1 - ow, ox + dx)), Math.max(0, Math.min(1 - oh, oy + dy)), ow, oh);
      } else {
        let nx = ox, ny = oy, nw = ow, nh = oh;
        if (m === 'nw' || m === 'sw') { nx = ox + dx; nw = ow - dx; }
        if (m === 'ne' || m === 'se') { nw = ow + dx; }
        if (m === 'nw' || m === 'ne') { ny = oy + dy; nh = oh - dy; }
        if (m === 'sw' || m === 'se') { nh = oh + dy; }
        nw = Math.max(0.05, Math.min(1, nw)); nh = Math.max(0.05, Math.min(1, nh));
        nx = Math.max(0, Math.min(1 - nw, nx)); ny = Math.max(0, Math.min(1 - nh, ny));
        onMove(nx, ny, nw, nh);
      }
    };
    const onMouseUp = () => { dragRef.current = null; onCommit(); document.removeEventListener("mousemove", onMouseMove); document.removeEventListener("mouseup", onMouseUp); };
    document.addEventListener("mousemove", onMouseMove); document.addEventListener("mouseup", onMouseUp);
  }, [x, y, width, height, videoW, videoH, onMove, onCommit]);

  const px = x * videoW, py = y * videoH, pw = width * videoW, ph = height * videoH;
  const handle = (cursor: string): React.CSSProperties => ({
    position: "absolute", width: 10, height: 10, borderRadius: "50%", background: color, border: "2px solid #fff",
    cursor, zIndex: 3, boxShadow: "0 1px 3px rgba(0,0,0,0.5)",
  });

  return (
    <div
      onMouseDown={(e) => startDrag(e, 'move')}
      style={{
        position: "absolute", left: px, top: py, width: pw, height: ph,
        border: `2px ${isSelected ? 'solid' : 'dashed'} ${color}`,
        background: isSelected ? `${color}20` : `${color}10`,
        cursor: isSelected ? "move" : "default",
        pointerEvents: isSelected ? "auto" : "none",
        boxSizing: "border-box", zIndex: isSelected ? 6 : 3,
      }}
    >
      <div style={{ position: "absolute", top: -20, left: 0, fontSize: 9, fontWeight: 700, color: "#fff", background: color, padding: "1px 6px", borderRadius: "3px 3px 0 0", pointerEvents: "none", textTransform: "uppercase", letterSpacing: 0.5 }}>{label}</div>
      {isSelected && (
        <>
          <div onMouseDown={(e) => startDrag(e, 'nw')} style={{ ...handle("nwse-resize"), top: -5, left: -5 }} />
          <div onMouseDown={(e) => startDrag(e, 'ne')} style={{ ...handle("nesw-resize"), top: -5, right: -5 }} />
          <div onMouseDown={(e) => startDrag(e, 'sw')} style={{ ...handle("nesw-resize"), bottom: -5, left: -5 }} />
          <div onMouseDown={(e) => startDrag(e, 'se')} style={{ ...handle("nwse-resize"), bottom: -5, right: -5 }} />
        </>
      )}
    </div>
  );
}

// ── Draggable Circle (for spotlight) ──

function DraggableCircle({ cx, cy, radius, color, videoW, videoH, isSelected, onMove, onCommit }: {
  cx: number; cy: number; radius: number; color: string;
  videoW: number; videoH: number; isSelected: boolean;
  onMove: (cx: number, cy: number, r: number) => void; onCommit: () => void;
}) {
  const dragRef = useRef<{ startX: number; startY: number; ocx: number; ocy: number; or: number; mode: 'move' | 'resize' } | null>(null);

  const startDrag = useCallback((e: React.MouseEvent, mode: 'move' | 'resize') => {
    e.preventDefault(); e.stopPropagation();
    dragRef.current = { startX: e.clientX, startY: e.clientY, ocx: cx, ocy: cy, or: radius, mode };
    const onMouseMove = (me: MouseEvent) => {
      if (!dragRef.current) return;
      const dx = (me.clientX - dragRef.current.startX) / videoW;
      const dy = (me.clientY - dragRef.current.startY) / videoH;
      if (dragRef.current.mode === 'move') {
        onMove(Math.max(0, Math.min(1, dragRef.current.ocx + dx)), Math.max(0, Math.min(1, dragRef.current.ocy + dy)), radius);
      } else {
        const dist = Math.sqrt(dx * dx + dy * dy);
        const sign = (me.clientX - dragRef.current.startX) > 0 ? 1 : -1;
        onMove(cx, cy, Math.max(0.03, Math.min(0.5, dragRef.current.or + dist * sign)));
      }
    };
    const onMouseUp = () => { dragRef.current = null; onCommit(); document.removeEventListener("mousemove", onMouseMove); document.removeEventListener("mouseup", onMouseUp); };
    document.addEventListener("mousemove", onMouseMove); document.addEventListener("mouseup", onMouseUp);
  }, [cx, cy, radius, videoW, videoH, onMove, onCommit]);

  const pxCx = cx * videoW, pxCy = cy * videoH;
  const pxR = radius * Math.max(videoW, videoH);

  return (
    <div
      onMouseDown={(e) => startDrag(e, 'move')}
      style={{
        position: "absolute", left: pxCx - pxR, top: pxCy - pxR, width: pxR * 2, height: pxR * 2,
        borderRadius: "50%", border: `2px ${isSelected ? 'solid' : 'dashed'} ${color}`,
        background: isSelected ? `${color}15` : "transparent",
        cursor: isSelected ? "move" : "default",
        pointerEvents: isSelected ? "auto" : "none",
        boxSizing: "border-box", zIndex: isSelected ? 6 : 3,
      }}
    >
      <div style={{ position: "absolute", top: -20, left: "50%", transform: "translateX(-50%)", fontSize: 9, fontWeight: 700, color: "#fff", background: color, padding: "1px 6px", borderRadius: 3, pointerEvents: "none", textTransform: "uppercase", letterSpacing: 0.5, whiteSpace: "nowrap" }}>Spotlight</div>
      {isSelected && (
        <div
          onMouseDown={(e) => startDrag(e, 'resize')}
          style={{ position: "absolute", right: -5, top: "50%", transform: "translateY(-50%)", width: 10, height: 10, borderRadius: "50%", background: color, border: "2px solid #fff", cursor: "ew-resize", zIndex: 3, boxShadow: "0 1px 3px rgba(0,0,0,0.5)" }}
        />
      )}
    </div>
  );
}

// ── Draggable Text ──

function DraggableText({ x, y, content, fontSize, color, fontFamily, bold, italic, underline, background, align, videoW, videoH, isSelected, onMove, onCommit }: {
  x: number; y: number; content: string; fontSize: number; color: string;
  fontFamily?: string; bold?: boolean; italic?: boolean; underline?: boolean; background?: string; align?: string;
  videoW: number; videoH: number; isSelected: boolean;
  onMove: (x: number, y: number) => void; onCommit: () => void;
}) {
  const dragRef = useRef<{ startX: number; startY: number; ox: number; oy: number } | null>(null);

  const startDrag = useCallback((e: React.MouseEvent) => {
    e.preventDefault(); e.stopPropagation();
    dragRef.current = { startX: e.clientX, startY: e.clientY, ox: x, oy: y };
    const onMouseMove = (me: MouseEvent) => {
      if (!dragRef.current) return;
      const dx = (me.clientX - dragRef.current.startX) / videoW;
      const dy = (me.clientY - dragRef.current.startY) / videoH;
      onMove(Math.max(0, Math.min(1, dragRef.current.ox + dx)), Math.max(0, Math.min(1, dragRef.current.oy + dy)));
    };
    const onMouseUp = () => { dragRef.current = null; onCommit(); document.removeEventListener("mousemove", onMouseMove); document.removeEventListener("mouseup", onMouseUp); };
    document.addEventListener("mousemove", onMouseMove); document.addEventListener("mouseup", onMouseUp);
  }, [x, y, videoW, videoH, onMove, onCommit]);

  const scaledFont = fontSize * (videoW / 1920);

  return (
    <div
      onMouseDown={startDrag}
      style={{
        position: "absolute", left: x * videoW, top: y * videoH,
        transform: "translate(-50%, -50%)",
        fontSize: scaledFont,
        fontWeight: bold ? 700 : 400,
        fontStyle: italic ? "italic" : "normal",
        textDecoration: underline ? "underline" : "none",
        fontFamily: fontFamily || "Inter, system-ui, sans-serif",
        color,
        background: background || "transparent",
        textShadow: !background ? "0 2px 8px rgba(0,0,0,0.8), 0 1px 3px rgba(0,0,0,0.6)" : "none",
        whiteSpace: "pre-wrap",
        textAlign: (align as React.CSSProperties['textAlign']) || "center",
        maxWidth: videoW * 0.8,
        lineHeight: 1.3,
        cursor: isSelected ? "move" : "default",
        pointerEvents: isSelected ? "auto" : "none",
        userSelect: "none", zIndex: isSelected ? 6 : 3,
        outline: isSelected ? "2px dashed rgba(16,185,129,0.5)" : "none",
        outlineOffset: 6, borderRadius: 4,
        padding: background ? "6px 14px" : "4px 8px",
      }}
    >
      {content || "Text"}
    </div>
  );
}

// ── Main Overlay ──

export function EffectsOverlay({ effects, outputTime, videoWidth, videoHeight, selectedEffectId, onUpdateEffectLive, onCommitEffect }: EffectsOverlayProps) {
  const activeEffects = effects.filter(
    (e) => e.type !== 'zoom-pan' && outputTime >= e.startTime && outputTime <= e.endTime
  );
  // Also show selected effect even if not in time range (so user can position it)
  const selectedNotActive = selectedEffectId && !activeEffects.find((e) => e.id === selectedEffectId)
    ? effects.find((e) => e.id === selectedEffectId && e.type !== 'zoom-pan')
    : null;
  const allVisible = selectedNotActive ? [...activeEffects, selectedNotActive] : activeEffects;

  if (allVisible.length === 0 || videoWidth <= 0) return null;

  return (
    <div style={{ position: "absolute", left: "50%", top: "50%", transform: "translate(-50%, -50%)", width: videoWidth, height: videoHeight, pointerEvents: "none", zIndex: 4 }}>
      {allVisible.map((effect) => {
        const opacity = (outputTime >= effect.startTime && outputTime <= effect.endTime) ? getEffectOpacity(effect, outputTime) : 0.4;
        const isSel = effect.id === selectedEffectId;

        if (effect.type === 'spotlight' && effect.spotlight) {
          const { x, y, radius, dimOpacity } = effect.spotlight;
          const cx = x * videoWidth, cy = y * videoHeight;
          const r = radius * Math.max(videoWidth, videoHeight);
          return (
            <div key={effect.id} style={{ position: "absolute", inset: 0, opacity }}>
              <svg width={videoWidth} height={videoHeight} style={{ position: "absolute", inset: 0 }}>
                <defs>
                  <mask id={`spotmask-${effect.id}`}>
                    <rect width="100%" height="100%" fill="white" />
                    <circle cx={cx} cy={cy} r={r} fill="black" />
                  </mask>
                </defs>
                <rect width="100%" height="100%" fill={`rgba(0,0,0,${dimOpacity})`} mask={`url(#spotmask-${effect.id})`} />
              </svg>
              {isSel && (
                <DraggableCircle
                  cx={x} cy={y} radius={radius} color="#f59e0b"
                  videoW={videoWidth} videoH={videoHeight} isSelected={true}
                  onMove={(ncx, ncy, nr) => onUpdateEffectLive(effect.id, { spotlight: { ...effect.spotlight!, x: ncx, y: ncy, radius: nr } })}
                  onCommit={onCommitEffect}
                />
              )}
            </div>
          );
        }

        if (effect.type === 'blur' && effect.blur) {
          const { x, y, width, height, radius: blurR, invert } = effect.blur;
          return (
            <div key={effect.id} style={{ position: "absolute", inset: 0, opacity }}>
              {invert ? (
                /* Invert mode: blur everything EXCEPT the rectangle */
                <>
                  {/* Top strip */}
                  <div style={{ position: "absolute", left: 0, top: 0, width: videoWidth, height: y * videoHeight, backdropFilter: `blur(${blurR}px)`, WebkitBackdropFilter: `blur(${blurR}px)`, pointerEvents: "none" }} />
                  {/* Bottom strip */}
                  <div style={{ position: "absolute", left: 0, top: (y + height) * videoHeight, width: videoWidth, height: (1 - y - height) * videoHeight, backdropFilter: `blur(${blurR}px)`, WebkitBackdropFilter: `blur(${blurR}px)`, pointerEvents: "none" }} />
                  {/* Left strip */}
                  <div style={{ position: "absolute", left: 0, top: y * videoHeight, width: x * videoWidth, height: height * videoHeight, backdropFilter: `blur(${blurR}px)`, WebkitBackdropFilter: `blur(${blurR}px)`, pointerEvents: "none" }} />
                  {/* Right strip */}
                  <div style={{ position: "absolute", left: (x + width) * videoWidth, top: y * videoHeight, width: (1 - x - width) * videoWidth, height: height * videoHeight, backdropFilter: `blur(${blurR}px)`, WebkitBackdropFilter: `blur(${blurR}px)`, pointerEvents: "none" }} />
                </>
              ) : (
                /* Normal mode: blur inside the rectangle */
                <div style={{
                  position: "absolute",
                  left: x * videoWidth, top: y * videoHeight,
                  width: width * videoWidth, height: height * videoHeight,
                  backdropFilter: `blur(${blurR}px)`, WebkitBackdropFilter: `blur(${blurR}px)`,
                  pointerEvents: "none",
                }} />
              )}
              {/* Interactive rectangle — only when selected */}
              {isSel && (
                <DraggableRect
                  x={x} y={y} width={width} height={height} color="#8b5cf6"
                  videoW={videoWidth} videoH={videoHeight} isSelected={true} label={invert ? "Keep Sharp" : "Blur"}
                  onMove={(nx, ny, nw, nh) => onUpdateEffectLive(effect.id, { blur: { ...effect.blur!, x: nx, y: ny, width: nw, height: nh } })}
                  onCommit={onCommitEffect}
                />
              )}
            </div>
          );
        }

        if (effect.type === 'text' && effect.text) {
          const t = effect.text;
          return (
            <DraggableText key={effect.id}
              x={t.x} y={t.y} content={t.content} fontSize={t.fontSize} color={t.color}
              fontFamily={t.fontFamily} bold={t.bold} italic={t.italic} underline={t.underline}
              background={t.background} align={t.align}
              videoW={videoWidth} videoH={videoHeight} isSelected={isSel}
              onMove={(nx, ny) => onUpdateEffectLive(effect.id, { text: { ...t, x: nx, y: ny } })}
              onCommit={onCommitEffect}
            />
          );
        }

        if (effect.type === 'fade' && effect.fade) {
          return (
            <div key={effect.id} style={{
              position: "absolute", inset: 0,
              background: effect.fade.color,
              opacity: effect.fade.opacity * opacity,
              pointerEvents: "none",
            }} />
          );
        }

        return null;
      })}
    </div>
  );
}
