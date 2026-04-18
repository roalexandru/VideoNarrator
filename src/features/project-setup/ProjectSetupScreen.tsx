import { useCallback, useState, useEffect, type CSSProperties } from "react";
import { open, message } from "@tauri-apps/plugin-dialog";
import { useProjectStore } from "../../stores/projectStore";
import { probeVideo, checkFileReadable, recordScreenNative, startScreenRecording } from "../../lib/tauri/commands";
import { fileNameFromPath } from "../../lib/formatters";
import { trackEvent, trackError } from "../telemetry/analytics";
import { Button } from "../../components/ui/Button";
import { formatFileSize, formatDuration } from "../../lib/formatters";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { listen } from "@tauri-apps/api/event";

const C = { text: "#e0e0ea", textDim: "#8b8ba0", textMuted: "#5a5a6e", bg: "rgba(255,255,255,0.03)", border: "rgba(255,255,255,0.07)", accent: "#818cf8", inputBg: "rgba(255,255,255,0.04)" };
const sectionLabel: CSSProperties = { fontSize: 11, fontWeight: 700, color: C.textMuted, textTransform: "uppercase", letterSpacing: 1.2, marginBottom: 10 };
const inputStyle: CSSProperties = { width: "100%", padding: "10px 14px", border: `1px solid ${C.border}`, borderRadius: 8, fontSize: 14, background: C.inputBg, color: C.text, outline: "none", fontFamily: "inherit" };

export function ProjectSetupScreen() {
  const { videoFile, contextDocuments, title, description } = useProjectStore();
  const { setVideoFile, addDocuments, removeDocument, setTitle, setDescription } = useProjectStore();
  const projectId = useProjectStore((s) => s.projectId);
  const [isRecording, setIsRecording] = useState(false);
  const [probing, setProbing] = useState(false);
  const [titleTouched, setTitleTouched] = useState(false);
  const [dragOver, setDragOver] = useState(false);

  // Drag-and-drop file import (video + context documents)
  useEffect(() => {
    const VIDEO_EXTS = ["mp4", "mov", "avi", "mkv", "webm"];
    const DOC_EXTS = ["md", "txt", "pdf"];

    const unDrop = listen<{ paths: string[] }>("tauri://drag-drop", async (event) => {
      setDragOver(false);
      const paths = event.payload.paths;
      if (!paths?.length) return;

      const videos = paths.filter((p) => VIDEO_EXTS.some((ext) => p.toLowerCase().endsWith(`.${ext}`)));
      const docs = paths.filter((p) => DOC_EXTS.some((ext) => p.toLowerCase().endsWith(`.${ext}`)));

      // Import the first video file
      if (videos.length > 0 && !videoFile) {
        setProbing(true);
        try {
          const m = await probeVideo(videos[0]);
          setVideoFile({ path: m.path, name: fileNameFromPath(m.path), size: m.file_size, duration: m.duration_seconds, resolution: { width: m.width, height: m.height }, codec: m.codec, fps: m.fps });
          trackEvent("video_imported", { source: "drag_drop" });
        } catch (err) {
          trackError("probe_video_drag_drop", err);
          await message(`Failed to load video: ${String(err)}`, { title: "Video Error", kind: "error" });
        } finally { setProbing(false); }
      }

      // Import document files
      if (docs.length > 0) {
        addDocuments(docs.map((p) => ({ id: crypto.randomUUID(), path: p, name: fileNameFromPath(p), size: 0, type: (p.split(".").pop() || "txt") as "md" | "txt" | "pdf" })));
        trackEvent("context_docs_added", { count: docs.length, source: "drag_drop" });
      }
    });

    const unOver = listen("tauri://drag-over", () => setDragOver(true));
    const unLeave = listen("tauri://drag-leave", () => setDragOver(false));

    return () => { unDrop.then((fn) => fn()); unOver.then((fn) => fn()); unLeave.then((fn) => fn()); };
  }, [videoFile, setVideoFile, addDocuments]);

  // Listen for recording-stopped event (Windows: overlay sends stop, backend emits this)
  useEffect(() => {
    const unlistenPromise = listen<string>("recording-stopped", async (event) => {
      setIsRecording(false);
      await getCurrentWindow().unminimize();
      await getCurrentWindow().setFocus();
      try {
        const m = await probeVideo(event.payload);
        setVideoFile({ path: m.path, name: "Screen Recording", size: m.file_size, duration: m.duration_seconds, resolution: { width: m.width, height: m.height }, codec: m.codec, fps: m.fps });
        trackEvent("video_imported", { source: "screen_recording", duration_s: Math.round(m.duration_seconds), codec: m.codec, width: m.width, height: m.height });
      } catch (e) {
        console.error("Failed to probe recorded video:", e);
        trackError("probe_recorded_video", e);
      }
    });
    return () => { unlistenPromise.then((fn) => fn()); };
  }, [setVideoFile]);

  const handleRecordScreen = useCallback(async () => {
    const isMac = navigator.userAgent.includes("Mac");
    setIsRecording(true);

    if (isMac) {
      // macOS: use native screencapture (Cmd+Shift+5 experience)
      try {
        await getCurrentWindow().minimize();
        const outputPath = await recordScreenNative(projectId);
        await getCurrentWindow().unminimize();
        await getCurrentWindow().setFocus();
        const m = await probeVideo(outputPath);
        setVideoFile({ path: m.path, name: "Screen Recording", size: m.file_size, duration: m.duration_seconds, resolution: { width: m.width, height: m.height }, codec: m.codec, fps: m.fps });
        trackEvent("video_imported", { source: "screen_recording", duration_s: Math.round(m.duration_seconds), codec: m.codec, width: m.width, height: m.height });
      } catch (e: unknown) {
        try { await getCurrentWindow().unminimize(); await getCurrentWindow().setFocus(); } catch { /* ignore */ }
        if (!String(e).includes("Cancelled")) { console.error("Recording failed:", e); trackError("screen_recording_macos", e); }
      } finally {
        setIsRecording(false);
      }
    } else {
      // Windows: start recording with overlay, overlay handles stop/pause
      try {
        await getCurrentWindow().minimize();
        await startScreenRecording(projectId);
        // Recording running; overlay window open; recording-stopped event will fire on stop
      } catch (e) {
        console.error("Failed to start recording:", e);
        trackError("screen_recording_windows", e);
        setIsRecording(false);
        try { await getCurrentWindow().unminimize(); await getCurrentWindow().setFocus(); } catch { /* ignore */ }
        await message(`Recording failed: ${String(e)}`, { title: "Narrator", kind: "error" });
      }
    }
  }, [projectId, setVideoFile]);

  const handleVideoSelect = useCallback(async () => {
    const file = await open({ multiple: false, filters: [{ name: "Video", extensions: ["mp4", "mov", "avi", "mkv", "webm"] }] });
    if (!file) return;
    setProbing(true);
    try {
      // First verify macOS/OS actually lets us read this file. Without this
      // check, a TCC-denied file would pass probe (ffmpeg has its own perms)
      // and silently break the video preview in the Edit screen.
      await checkFileReadable(file as string);
      const m = await probeVideo(file as string);
      setVideoFile({ path: m.path, name: fileNameFromPath(m.path), size: m.file_size, duration: m.duration_seconds, resolution: { width: m.width, height: m.height }, codec: m.codec, fps: m.fps });
      trackEvent("video_imported", { source: "file", duration_s: Math.round(m.duration_seconds), codec: m.codec, width: m.width, height: m.height, fps: Math.round(m.fps), size_mb: Math.round(m.file_size / 1048576) });
    } catch (err) {
      console.error("Probe failed:", err);
      trackError("probe_video_file_select", err);
      await message(String(err).replace(/^(Error: )?/, ""), { title: "Can't open video", kind: "error" });
    }
    finally { setProbing(false); }
  }, [setVideoFile]);

  const handleDocSelect = useCallback(async () => {
    const files = await open({ multiple: true, filters: [{ name: "Documents", extensions: ["md", "txt", "pdf"] }] });
    if (!files) return;
    const paths = Array.isArray(files) ? files : [files];
    addDocuments(paths.map((p) => ({ id: crypto.randomUUID(), path: p, name: p.split("/").pop() || "document", size: 0, type: (p.split(".").pop() || "txt") as "md" | "txt" | "pdf" })));
    trackEvent("context_docs_added", { count: paths.length, types: paths.map(p => p.split(".").pop() || "unknown").join(",") });
  }, [addDocuments]);

  return (
    <div style={{ maxWidth: 640, margin: "0 auto", position: "relative" }}>
      {/* Drag-and-drop overlay */}
      {dragOver && (
        <div style={{
          position: "absolute", inset: -16, zIndex: 50, borderRadius: 16,
          border: "2px dashed rgba(99,102,241,0.6)", background: "rgba(99,102,241,0.08)",
          display: "flex", alignItems: "center", justifyContent: "center",
          pointerEvents: "none",
        }}>
          <div style={{ fontSize: 16, fontWeight: 600, color: C.accent }}>Drop files here</div>
        </div>
      )}
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
        ) : isRecording ? (
          /* Recording in progress — controls are in the overlay (Windows) or native UI (macOS) */
          <div style={{
            padding: "28px 20px", borderRadius: 12,
            border: "2px solid rgba(239,68,68,0.3)", background: "rgba(239,68,68,0.04)",
            textAlign: "center",
          }}>
            <div style={{ display: "flex", alignItems: "center", justifyContent: "center", gap: 10, marginBottom: 12 }}>
              <div style={{ width: 12, height: 12, borderRadius: "50%", background: "#ef4444", animation: "recpulse 1s infinite" }} />
              <span style={{ fontWeight: 600, fontSize: 15, color: "#ef4444" }}>Recording in progress</span>
            </div>
            <p style={{ color: C.textMuted, fontSize: 12 }}>Use the recording controls to stop or pause.</p>
            <style>{`@keyframes recpulse { 0%,100% { opacity:1; } 50% { opacity:0.3; } }`}</style>
          </div>
        ) : (
          <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 10 }}>
            <button onClick={handleVideoSelect} disabled={probing} style={{
              padding: "32px 20px", border: `2px dashed rgba(255,255,255,0.1)`, borderRadius: 12,
              background: "rgba(255,255,255,0.02)", cursor: probing ? "wait" : "pointer", textAlign: "center", fontFamily: "inherit",
              opacity: probing ? 0.7 : 1,
            }}
              onMouseEnter={(e) => { if (!probing) { e.currentTarget.style.borderColor = "rgba(99,102,241,0.4)"; e.currentTarget.style.background = "rgba(99,102,241,0.04)"; } }}
              onMouseLeave={(e) => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.1)"; e.currentTarget.style.background = "rgba(255,255,255,0.02)"; }}
            >
              {probing ? (
                <>
                  <div style={{ fontWeight: 600, color: C.accent, fontSize: 13 }}>Probing video...</div>
                  <div style={{ color: C.textMuted, fontSize: 11, marginTop: 3 }}>Analyzing file metadata</div>
                </>
              ) : (
                <>
                  <svg width="28" height="28" viewBox="0 0 24 24" fill="none" stroke="#6366f1" strokeWidth="1.5" strokeLinecap="round" style={{ margin: "0 auto 8px", display: "block" }}>
                    <rect x="2" y="2" width="20" height="20" rx="2.18"/><line x1="7" y1="2" x2="7" y2="22"/><line x1="17" y1="2" x2="17" y2="22"/><line x1="2" y1="12" x2="22" y2="12"/>
                  </svg>
                  <div style={{ fontWeight: 600, color: C.textDim, fontSize: 13 }}>Select Video File</div>
                  <div style={{ color: C.textMuted, fontSize: 11, marginTop: 3 }}>MP4, MOV, AVI, MKV, WebM</div>
                </>
              )}
            </button>

            <button onClick={handleRecordScreen} style={{
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
                <button
                  onClick={() => removeDocument(doc.id)}
                  style={{
                    display: "inline-flex", alignItems: "center", gap: 4,
                    padding: "4px 8px", borderRadius: 6, border: "none",
                    background: "transparent", color: C.textMuted,
                    cursor: "pointer", fontSize: 12, fontFamily: "inherit", fontWeight: 500,
                    transition: "color 0.15s, background 0.15s",
                  }}
                  onMouseEnter={(e) => { e.currentTarget.style.color = "#f87171"; e.currentTarget.style.background = "rgba(239,68,68,0.08)"; }}
                  onMouseLeave={(e) => { e.currentTarget.style.color = C.textMuted; e.currentTarget.style.background = "transparent"; }}
                >
                  <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round">
                    <polyline points="3 6 5 6 21 6"/><path d="M19 6l-1 14a2 2 0 01-2 2H8a2 2 0 01-2-2L5 6"/><path d="M10 11v6"/><path d="M14 11v6"/><path d="M9 6V4a1 1 0 011-1h4a1 1 0 011 1v2"/>
                  </svg>
                  Remove
                </button>
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
            <input type="text" value={title} onChange={(e) => setTitle(e.target.value)} placeholder="e.g., UiPath Studio Walkthrough"
              style={{ ...inputStyle, borderColor: titleTouched && !title.trim() ? "rgba(239,68,68,0.5)" : inputStyle.borderColor }}
              onFocus={(e) => e.target.style.borderColor = "rgba(99,102,241,0.4)"}
              onBlur={(e) => { setTitleTouched(true); e.target.style.borderColor = !title.trim() ? "rgba(239,68,68,0.5)" : C.border; }} />
            {titleTouched && !title.trim() && (
              <p style={{ fontSize: 11, color: "#f87171", marginTop: 4 }}>Title is required</p>
            )}
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
