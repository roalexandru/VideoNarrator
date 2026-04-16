import { useRef, useCallback, useEffect } from "react";
import type { ZoomRegion } from "../../stores/editStore";

interface ZoomPanOverlayProps {
  videoRect: { width: number; height: number; left: number; top: number };
  startRegion: ZoomRegion;
  endRegion: ZoomRegion;
  activeRegion: 'start' | 'end';
  onActiveRegionChange: (region: 'start' | 'end') => void;
  onStartChange: (region: ZoomRegion) => void;
  onEndChange: (region: ZoomRegion) => void;
  onCommit: () => void;
}

const MIN_SIZE = 0.1; // 10% minimum region dimension

type Handle = 'nw' | 'ne' | 'sw' | 'se';

function clampRegion(r: ZoomRegion): ZoomRegion {
  const w = Math.max(MIN_SIZE, Math.min(1, r.width));
  const h = Math.max(MIN_SIZE, Math.min(1, r.height));
  const x = Math.max(0, Math.min(1 - w, r.x));
  const y = Math.max(0, Math.min(1 - h, r.y));
  return { x, y, width: w, height: h };
}

function RegionRect({
  region,
  color,
  label,
  isActive,
  videoW,
  videoH,
  onRegionChange,
  onCommit,
  onClick,
}: {
  region: ZoomRegion;
  color: string;
  label: string;
  isActive: boolean;
  videoW: number;
  videoH: number;
  onRegionChange: (r: ZoomRegion) => void;
  onCommit: () => void;
  onClick: () => void;
}) {
  const dragRef = useRef<{ startX: number; startY: number; startRegion: ZoomRegion; handle: Handle | 'move' } | null>(null);
  const dragHandlersRef = useRef<{ onMove: (e: MouseEvent) => void; onUp: () => void } | null>(null);

  // Clean up drag listeners on unmount to prevent leaks if unmounted mid-drag
  useEffect(() => {
    return () => {
      if (dragHandlersRef.current) {
        document.removeEventListener("mousemove", dragHandlersRef.current.onMove);
        document.removeEventListener("mouseup", dragHandlersRef.current.onUp);
        dragHandlersRef.current = null;
      }
    };
  }, []);

  const px = region.x * videoW;
  const py = region.y * videoH;
  const pw = region.width * videoW;
  const ph = region.height * videoH;

  const handleMouseDown = useCallback((e: React.MouseEvent, handle: Handle | 'move') => {
    e.preventDefault();
    e.stopPropagation();
    onClick();
    dragRef.current = { startX: e.clientX, startY: e.clientY, startRegion: { ...region }, handle };

    const onMove = (me: MouseEvent) => {
      if (!dragRef.current) return;
      const dx = (me.clientX - dragRef.current.startX) / videoW;
      const dy = (me.clientY - dragRef.current.startY) / videoH;
      const sr = dragRef.current.startRegion;

      if (dragRef.current.handle === 'move') {
        onRegionChange(clampRegion({ ...sr, x: sr.x + dx, y: sr.y + dy }));
      } else {
        let { x, y, width, height } = sr;
        const h = dragRef.current.handle;
        if (h === 'nw' || h === 'sw') { x = sr.x + dx; width = sr.width - dx; }
        if (h === 'ne' || h === 'se') { width = sr.width + dx; }
        if (h === 'nw' || h === 'ne') { y = sr.y + dy; height = sr.height - dy; }
        if (h === 'sw' || h === 'se') { height = sr.height + dy; }
        onRegionChange(clampRegion({ x, y, width, height }));
      }
    };

    const onUp = () => {
      dragRef.current = null;
      dragHandlersRef.current = null;
      onCommit();
      document.removeEventListener("mousemove", onMove);
      document.removeEventListener("mouseup", onUp);
    };

    dragHandlersRef.current = { onMove, onUp };
    document.addEventListener("mousemove", onMove);
    document.addEventListener("mouseup", onUp);
  }, [region, videoW, videoH, onRegionChange, onCommit, onClick]);

  const handleStyle = (cursor: string): React.CSSProperties => ({
    position: "absolute",
    width: 10,
    height: 10,
    borderRadius: "50%",
    background: color,
    border: "2px solid #fff",
    cursor,
    zIndex: 3,
    boxShadow: "0 1px 3px rgba(0,0,0,0.5)",
  });

  return (
    <div
      onMouseDown={(e) => handleMouseDown(e, 'move')}
      style={{
        position: "absolute",
        left: px,
        top: py,
        width: pw,
        height: ph,
        border: `2px ${isActive ? 'solid' : 'dashed'} ${color}`,
        background: isActive ? `${color}18` : `${color}08`,
        cursor: "move",
        zIndex: isActive ? 2 : 1,
        transition: "background 0.1s",
        boxSizing: "border-box",
      }}
    >
      {/* Label — clickable to switch active region */}
      <div
        onMouseDown={(e) => { e.preventDefault(); e.stopPropagation(); onClick(); }}
        style={{
          position: "absolute",
          top: -22,
          left: 0,
          fontSize: 10,
          fontWeight: 700,
          color: "#fff",
          background: color,
          padding: "1px 6px",
          borderRadius: "3px 3px 0 0",
          letterSpacing: 0.5,
          textTransform: "uppercase",
          pointerEvents: "auto",
          cursor: "pointer",
          zIndex: 5,
        }}>
        {label}
      </div>

      {/* Grid overlay (rule of thirds) */}
      {isActive && (
        <svg width="100%" height="100%" style={{ position: "absolute", inset: 0, pointerEvents: "none", opacity: 0.2 }}>
          <line x1="33.3%" y1="0" x2="33.3%" y2="100%" stroke="#fff" strokeWidth="1" />
          <line x1="66.6%" y1="0" x2="66.6%" y2="100%" stroke="#fff" strokeWidth="1" />
          <line x1="0" y1="33.3%" x2="100%" y2="33.3%" stroke="#fff" strokeWidth="1" />
          <line x1="0" y1="66.6%" x2="100%" y2="66.6%" stroke="#fff" strokeWidth="1" />
        </svg>
      )}

      {/* Corner handles */}
      <div onMouseDown={(e) => handleMouseDown(e, 'nw')} style={{ ...handleStyle("nwse-resize"), top: -5, left: -5 }} />
      <div onMouseDown={(e) => handleMouseDown(e, 'ne')} style={{ ...handleStyle("nesw-resize"), top: -5, right: -5 }} />
      <div onMouseDown={(e) => handleMouseDown(e, 'sw')} style={{ ...handleStyle("nesw-resize"), bottom: -5, left: -5 }} />
      <div onMouseDown={(e) => handleMouseDown(e, 'se')} style={{ ...handleStyle("nwse-resize"), bottom: -5, right: -5 }} />
    </div>
  );
}

export function ZoomPanOverlay({
  videoRect,
  startRegion,
  endRegion,
  activeRegion,
  onActiveRegionChange,
  onStartChange,
  onEndChange,
  onCommit,
}: ZoomPanOverlayProps) {
  // Keyboard: Tab to switch active region, Escape handled by parent
  useEffect(() => {
    const handler = (e: KeyboardEvent) => {
      if (e.code === "Tab") {
        e.preventDefault();
        onActiveRegionChange(activeRegion === 'start' ? 'end' : 'start');
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [activeRegion, onActiveRegionChange]);

  return (
    <div style={{
      position: "absolute",
      left: "50%",
      top: "50%",
      transform: `translate(-50%, -50%)`,
      width: videoRect.width,
      height: videoRect.height,
      pointerEvents: "auto",
      zIndex: 5,
    }}>
      {/* Render inactive region first (behind), active region on top */}
      <RegionRect
        region={activeRegion === 'start' ? endRegion : startRegion}
        color={activeRegion === 'start' ? "#ef4444" : "#4ade80"}
        label={activeRegion === 'start' ? "End" : "Start"}
        isActive={false}
        videoW={videoRect.width}
        videoH={videoRect.height}
        onRegionChange={activeRegion === 'start' ? onEndChange : onStartChange}
        onCommit={onCommit}
        onClick={() => onActiveRegionChange(activeRegion === 'start' ? 'end' : 'start')}
      />
      <RegionRect
        region={activeRegion === 'start' ? startRegion : endRegion}
        color={activeRegion === 'start' ? "#4ade80" : "#ef4444"}
        label={activeRegion === 'start' ? "Start" : "End"}
        isActive={true}
        videoW={videoRect.width}
        videoH={videoRect.height}
        onRegionChange={activeRegion === 'start' ? onStartChange : onEndChange}
        onCommit={onCommit}
        onClick={() => {}}
      />

      {/* Region toggle pill — always accessible regardless of overlap */}
      <div style={{
        position: "absolute",
        top: 8,
        right: 8,
        display: "flex",
        gap: 2,
        background: "rgba(0,0,0,0.7)",
        borderRadius: 6,
        padding: 3,
        zIndex: 10,
      }}>
        <button
          onMouseDown={(e) => { e.preventDefault(); e.stopPropagation(); onActiveRegionChange('start'); }}
          style={{
            padding: "4px 10px", borderRadius: 4, border: "none", fontSize: 10, fontWeight: 700,
            background: activeRegion === 'start' ? "#4ade80" : "transparent",
            color: activeRegion === 'start' ? "#000" : "rgba(255,255,255,0.6)",
            cursor: "pointer", fontFamily: "inherit", textTransform: "uppercase", letterSpacing: 0.5,
          }}>Start</button>
        <button
          onMouseDown={(e) => { e.preventDefault(); e.stopPropagation(); onActiveRegionChange('end'); }}
          style={{
            padding: "4px 10px", borderRadius: 4, border: "none", fontSize: 10, fontWeight: 700,
            background: activeRegion === 'end' ? "#ef4444" : "transparent",
            color: activeRegion === 'end' ? "#fff" : "rgba(255,255,255,0.6)",
            cursor: "pointer", fontFamily: "inherit", textTransform: "uppercase", letterSpacing: 0.5,
          }}>End</button>
      </div>

      {/* Region toggle hint */}
      <div style={{
        position: "absolute",
        bottom: 8,
        left: "50%",
        transform: "translateX(-50%)",
        fontSize: 10,
        color: "rgba(255,255,255,0.6)",
        background: "rgba(0,0,0,0.6)",
        padding: "3px 10px",
        borderRadius: 4,
        pointerEvents: "none",
        whiteSpace: "nowrap",
      }}>
        Drag corners to resize &middot; Esc to close
      </div>
    </div>
  );
}
