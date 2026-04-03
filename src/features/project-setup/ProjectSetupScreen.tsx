import { useCallback, useEffect, type CSSProperties } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../../stores/projectStore";
import { probeVideo, openRecorderWindow } from "../../lib/tauri/commands";
import { Button } from "../../components/ui/Button";
import { formatFileSize, formatDuration } from "../../lib/formatters";
import { listen } from "@tauri-apps/api/event";

const C = { text: "#e0e0ea", textDim: "#8b8ba0", textMuted: "#5a5a6e", bg: "rgba(255,255,255,0.03)", border: "rgba(255,255,255,0.07)", accent: "#818cf8", inputBg: "rgba(255,255,255,0.04)" };
const sectionLabel: CSSProperties = { fontSize: 11, fontWeight: 700, color: C.textMuted, textTransform: "uppercase", letterSpacing: 1.2, marginBottom: 10 };
const inputStyle: CSSProperties = { width: "100%", padding: "10px 14px", border: `1px solid ${C.border}`, borderRadius: 8, fontSize: 14, background: C.inputBg, color: C.text, outline: "none", fontFamily: "inherit" };

export function ProjectSetupScreen() {
  const { videoFile, contextDocuments, title, description } = useProjectStore();
  const { setVideoFile, addDocuments, removeDocument, setTitle, setDescription } = useProjectStore();

  // Listen for recorder-stopped event from the recorder window
  useEffect(() => {
    const unlisten = listen<{ path: string }>("recorder-stopped", async (event) => {
      try {
        const m = await probeVideo(event.payload.path);
        setVideoFile({ path: m.path, name: "Screen Recording", size: m.file_size, duration: m.duration_seconds, resolution: { width: m.width, height: m.height }, codec: m.codec, fps: m.fps });
      } catch (e) { console.error("Failed to load recording:", e); }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [setVideoFile]);

  const handleVideoSelect = useCallback(async () => {
    const file = await open({ multiple: false, filters: [{ name: "Video", extensions: ["mp4", "mov", "avi", "mkv", "webm"] }] });
    if (!file) return;
    try {
      const m = await probeVideo(file as string);
      setVideoFile({ path: m.path, name: m.path.split("/").pop() || "video", size: m.file_size, duration: m.duration_seconds, resolution: { width: m.width, height: m.height }, codec: m.codec, fps: m.fps });
    } catch (err) { console.error("Probe failed:", err); }
  }, [setVideoFile]);

  const handleDocSelect = useCallback(async () => {
    const files = await open({ multiple: true, filters: [{ name: "Documents", extensions: ["md", "txt", "pdf"] }] });
    if (!files) return;
    const paths = Array.isArray(files) ? files : [files];
    addDocuments(paths.map((p) => ({ id: crypto.randomUUID(), path: p, name: p.split("/").pop() || "document", size: 0, type: (p.split(".").pop() || "txt") as "md" | "txt" | "pdf" })));
  }, [addDocuments]);

  return (
    <div style={{ maxWidth: 640, margin: "0 auto" }}>
      <div style={{ marginBottom: 32 }}>
        <h2 style={{ fontSize: 24, fontWeight: 700, color: C.text, letterSpacing: -0.3 }}>Project Setup</h2>
        <p style={{ color: C.textDim, marginTop: 4, fontSize: 14 }}>Add your video file and context documents.</p>
      </div>

      {/* Video */}
      <section style={{ marginBottom: 28 }}>
        <div style={sectionLabel}>Video File</div>
        {videoFile ? (
          <div style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "14px 18px", background: C.bg, borderRadius: 10, border: `1px solid ${C.border}` }}>
            <div>
              <div style={{ fontWeight: 600, color: C.text, fontSize: 14 }}>{videoFile.name}</div>
              <div style={{ fontSize: 12, color: C.textDim, marginTop: 3 }}>
                {formatDuration(videoFile.duration)} &middot; {videoFile.resolution.width}&times;{videoFile.resolution.height} &middot; {videoFile.codec.toUpperCase()} &middot; {formatFileSize(videoFile.size)}
              </div>
            </div>
            <Button variant="ghost" size="sm" onClick={() => setVideoFile(null)}>Remove</Button>
          </div>
        ) : (
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
            <button onClick={handleVideoSelect} style={{
              padding: "32px 20px", border: `2px dashed rgba(255,255,255,0.1)`, borderRadius: 12,
              background: "rgba(255,255,255,0.02)", cursor: "pointer", textAlign: "center", fontFamily: "inherit",
            }}
              onMouseEnter={(e) => { e.currentTarget.style.borderColor = "rgba(99,102,241,0.4)"; e.currentTarget.style.background = "rgba(99,102,241,0.04)"; }}
              onMouseLeave={(e) => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.1)"; e.currentTarget.style.background = "rgba(255,255,255,0.02)"; }}
            >
              <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="#6366f1" strokeWidth="1.5" strokeLinecap="round" style={{ margin: "0 auto 8px", display: "block" }}>
                <rect x="2" y="2" width="20" height="20" rx="2.18"/><line x1="7" y1="2" x2="7" y2="22"/><line x1="17" y1="2" x2="17" y2="22"/><line x1="2" y1="12" x2="22" y2="12"/>
              </svg>
              <div style={{ fontWeight: 600, color: C.textDim, fontSize: 13 }}>Select Video File</div>
              <div style={{ color: C.textMuted, fontSize: 11, marginTop: 3 }}>MP4, MOV, AVI, MKV, WebM</div>
            </button>

            <button onClick={() => openRecorderWindow()} style={{
              padding: "32px 20px", border: `2px dashed rgba(255,255,255,0.1)`, borderRadius: 12,
              background: "rgba(255,255,255,0.02)", cursor: "pointer", textAlign: "center", fontFamily: "inherit",
            }}
              onMouseEnter={(e) => { e.currentTarget.style.borderColor = "rgba(239,68,68,0.4)"; e.currentTarget.style.background = "rgba(239,68,68,0.04)"; }}
              onMouseLeave={(e) => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.1)"; e.currentTarget.style.background = "rgba(255,255,255,0.02)"; }}
            >
              <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="#ef4444" strokeWidth="1.5" strokeLinecap="round" style={{ margin: "0 auto 8px", display: "block" }}>
                <circle cx="12" cy="12" r="10"/><circle cx="12" cy="12" r="4" fill="#ef4444"/>
              </svg>
              <div style={{ fontWeight: 600, color: C.textDim, fontSize: 13 }}>Record Screen</div>
              <div style={{ color: C.textMuted, fontSize: 11, marginTop: 3 }}>Capture your screen</div>
            </button>
          </div>
        )}
      </section>

      {/* Context documents */}
      <section style={{ marginBottom: 28 }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
          <div style={sectionLabel}>Context Documents</div>
          <Button variant="secondary" size="sm" onClick={handleDocSelect}>+ Add</Button>
        </div>
        {contextDocuments.length > 0 ? (
          <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
            {contextDocuments.map((doc) => (
              <div key={doc.id} style={{ display: "flex", alignItems: "center", justifyContent: "space-between", padding: "10px 14px", background: C.bg, borderRadius: 8, border: `1px solid ${C.border}` }}>
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <span style={{ fontSize: 10, fontWeight: 700, color: C.accent, background: "rgba(99,102,241,0.1)", padding: "2px 7px", borderRadius: 5, textTransform: "uppercase", fontFamily: "monospace" }}>{doc.type}</span>
                  <span style={{ fontSize: 13, color: C.textDim }}>{doc.name}</span>
                </div>
                <Button variant="ghost" size="sm" onClick={() => removeDocument(doc.id)}>&times;</Button>
              </div>
            ))}
          </div>
        ) : (
          <p style={{ fontSize: 13, color: C.textMuted, fontStyle: "italic" }}>Brand guides, product docs, or glossaries improve narration quality.</p>
        )}
      </section>

      {/* Details */}
      <section style={{ marginBottom: 32 }}>
        <div style={sectionLabel}>Project Details</div>
        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          <div>
            <label style={{ display: "block", fontSize: 12, fontWeight: 600, color: C.textDim, marginBottom: 5 }}>Title *</label>
            <input type="text" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="e.g., UiPath Studio Walkthrough" style={inputStyle}
              onFocus={(e) => e.target.style.borderColor = "rgba(99,102,241,0.4)"} onBlur={(e) => e.target.style.borderColor = C.border} />
          </div>
          <div>
            <label style={{ display: "block", fontSize: 12, fontWeight: 600, color: C.textDim, marginBottom: 5 }}>Description</label>
            <textarea value={description} onChange={(e) => setDescription(e.target.value)} placeholder="What should the viewer learn?" rows={3}
              style={{ ...inputStyle, resize: "none" as const }}
              onFocus={(e) => e.target.style.borderColor = "rgba(99,102,241,0.4)"} onBlur={(e) => e.target.style.borderColor = C.border} />
          </div>
        </div>
      </section>

    </div>
  );
}
