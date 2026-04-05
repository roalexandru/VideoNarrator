import { useEffect, useCallback } from "react";
import { Channel } from "@tauri-apps/api/core";
import { useProcessingStore } from "../../stores/processingStore";
import { useProjectStore } from "../../stores/projectStore";
import { useConfigStore } from "../../stores/configStore";
import { useScriptStore } from "../../stores/scriptStore";
import { startGeneration, cancelGeneration } from "../../lib/tauri/commands";
import { trackEvent } from "../telemetry/analytics";
import { Button } from "../../components/ui/Button";
import { ProgressBar } from "../../components/ui/ProgressBar";
import { secondsToTimestamp } from "../../lib/formatters";
import type { ProgressEvent, ProcessingPhase } from "../../types/processing";
import type { GenerationParams } from "../../types/config";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };

const PHASE_LABELS: Record<ProcessingPhase, string> = {
  idle: "Preparing...", extracting_frames: "Extracting frames from video...",
  processing_docs: "Processing context documents...", generating_narration: "Generating narration with AI...",
  done: "Generation complete!", error: "Error occurred", cancelled: "Cancelled",
};

export function ProcessingScreen() {
  const proc = useProcessingStore();
  const project = useProjectStore();
  const config = useConfigStore();
  const setScript = useScriptStore((s) => s.setScript);

  const run = useCallback(async () => {
    proc.reset(); proc.setPhase("extracting_frames");
    const ch = new Channel<ProgressEvent>();
    ch.onmessage = (e: ProgressEvent) => {
      if (e.kind === "phase_change") proc.setPhase(e.phase);
      else if (e.kind === "progress") proc.setProgress(e.percent);
      else if (e.kind === "frame_extracted") proc.appendFrame(e.frame);
      else if (e.kind === "segment_streamed") proc.appendSegment(e.segment);
      else if (e.kind === "error") proc.setError(e.message);
    };
    const params: GenerationParams = {
      project_id: project.projectId,
      video_path: project.videoFile!.path,
      document_paths: project.contextDocuments.map((d) => d.path),
      title: project.title, description: project.description,
      style: config.style, primary_language: config.primaryLanguage,
      additional_languages: config.languages.filter((l) => l !== config.primaryLanguage),
      frame_config: { density: config.frameDensity, scene_threshold: config.sceneThreshold, max_frames: config.maxFrames },
      ai_config: { provider: config.aiProvider, model: config.model, temperature: config.temperature },
      custom_prompt: config.customPrompt,
    };
    try {
      const script = await startGeneration(params, ch);
      setScript(config.primaryLanguage, script); proc.setPhase("done");
      trackEvent("processing_completed", { segments: script.segments.length, provider: config.aiProvider, style: config.style });
    } catch (err: any) {
      proc.setError(typeof err === "string" ? err : err?.message || "Unknown error");
      proc.setPhase("error");
    }
  }, [project, config, proc, setScript]);

  // Only auto-run if idle AND no script exists yet (don't re-run on revisit)
  const hasScript = Object.keys(useScriptStore.getState().scripts).length > 0;
  useEffect(() => {
    if (proc.phase === "idle" && !hasScript) run();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // If we already have a script and phase is idle (navigated back), show completed
  const showCompleted = hasScript && (proc.phase === "idle" || proc.phase === "done");

  const phases = ["extracting_frames", "processing_docs", "generating_narration"] as const;
  const pi = phases.indexOf(proc.phase as any);
  const pct = showCompleted ? 100 : proc.phase === "done" ? 100 : (proc.phase === "extracting_frames" && proc.frames.length === 0) ? 5 : pi >= 0 ? ((pi + 0.5) / 3) * 100 : 5;
  const elapsed = proc.frames.length > 0 ? `${proc.frames.length} frames extracted` : "";
  const segCount = proc.streamingSegments.length;
  const scriptSegCount = Object.values(useScriptStore.getState().scripts)[0]?.segments.length || 0;

  if (showCompleted) {
    return (
      <div style={{ maxWidth: 640, margin: "0 auto" }}>
        <div style={{ marginBottom: 32 }}>
          <h2 style={{ fontSize: 22, fontWeight: 700, color: C.text }}>Processing</h2>
          <p style={{ color: "#4ade80", marginTop: 4, fontSize: 14 }}>Generation complete!</p>
        </div>
        <ProgressBar value={100} height={4} />
        <div style={{ marginTop: 32, padding: "24px", borderRadius: 12, border: `1px solid rgba(34,197,94,0.2)`, background: "rgba(34,197,94,0.04)", textAlign: "center" }}>
          <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke="#4ade80" strokeWidth="2" strokeLinecap="round" style={{ margin: "0 auto 12px", display: "block" }}><path d="M22 11.08V12a10 10 0 11-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/></svg>
          <p style={{ fontSize: 16, fontWeight: 600, color: C.text, marginBottom: 6 }}>Narration Generated</p>
          <p style={{ fontSize: 13, color: C.dim }}>{scriptSegCount} segments ready for review</p>
          <div style={{ marginTop: 16 }}>
            <Button onClick={run}>Regenerate</Button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column" }}>
      {/* Header */}
      <div style={{ marginBottom: 20 }}>
        <h2 style={{ fontSize: 22, fontWeight: 700, color: C.text }}>Processing</h2>
        <p style={{ color: C.dim, marginTop: 4, fontSize: 14 }}>{PHASE_LABELS[proc.phase]}</p>
      </div>

      {/* Progress bar with percentage */}
      <div style={{ marginBottom: 20 }}>
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
          <span style={{ fontSize: 12, color: C.dim }}>
            {elapsed}{elapsed && segCount > 0 ? " · " : ""}{segCount > 0 ? `${segCount} segments generated` : ""}
          </span>
          <span style={{ fontSize: 12, color: C.accent, fontWeight: 600 }}>{Math.round(pct)}%</span>
        </div>
        <ProgressBar value={pct} height={6} />
      </div>

      {/* Two-column: steps + live preview */}
      <div style={{ flex: 1, display: "grid", gridTemplateColumns: "280px 1fr", gap: 20, minHeight: 0 }}>
        {/* Left: Steps + frames */}
        <div style={{ display: "flex", flexDirection: "column", gap: 16, overflowY: "auto" }}>
          {/* Phase steps */}
          {([
            { p: "extracting_frames", l: "Extract Frames", detail: `${proc.frames.length} frames` },
            { p: "processing_docs", l: "Process Documents", detail: "Analyzing context" },
            { p: "generating_narration", l: "Generate Narration", detail: `${segCount} segments` },
          ] as const).map(({ p, l, detail }, i) => {
            const active = proc.phase === p;
            const done = proc.phase === "done" || (pi > i && pi >= 0);
            return (
              <div key={p} style={{
                display: "flex", alignItems: "center", gap: 12, padding: "12px 14px",
                borderRadius: 10, background: active ? "rgba(99,102,241,0.06)" : "rgba(255,255,255,0.02)",
                border: active ? "1px solid rgba(99,102,241,0.2)" : `1px solid ${C.border}`,
              }}>
                <div style={{
                  width: 36, height: 36, borderRadius: 9, display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0,
                  background: done ? "rgba(34,197,94,0.12)" : active ? "rgba(99,102,241,0.15)" : "rgba(255,255,255,0.04)",
                  color: done ? "#4ade80" : active ? C.accent : C.muted,
                }}>
                  {done ? <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round"><polyline points="20 6 9 17 4 12"/></svg>
                    : active ? <div style={{ width: 8, height: 8, borderRadius: "50%", background: C.accent, animation: "pulse 1.5s infinite" }} />
                    : <span style={{ fontSize: 13, fontWeight: 600 }}>{i + 1}</span>}
                </div>
                <div>
                  <div style={{ fontSize: 14, fontWeight: active ? 600 : 400, color: active ? C.text : C.dim }}>{l}</div>
                  {(active || done) && <div style={{ fontSize: 11, color: C.muted, marginTop: 2 }}>{detail}</div>}
                </div>
              </div>
            );
          })}

          {/* Frames filmstrip */}
          {proc.frames.length > 0 && (
            <div>
              <div style={{ fontSize: 11, fontWeight: 600, color: C.muted, marginBottom: 6, textTransform: "uppercase", letterSpacing: 0.8 }}>
                Extracted Frames
              </div>
              <div style={{ display: "grid", gridTemplateColumns: "repeat(4, 1fr)", gap: 4 }}>
                {proc.frames.slice(0, 16).map((f) => (
                  <div key={f.index} style={{
                    aspectRatio: "16/10", background: "rgba(255,255,255,0.04)", borderRadius: 5,
                    display: "flex", alignItems: "center", justifyContent: "center",
                    color: C.muted, fontSize: 10, border: `1px solid ${C.border}`,
                  }}>
                    {f.timestamp_seconds.toFixed(0)}s
                  </div>
                ))}
                {proc.frames.length > 16 && (
                  <div style={{ aspectRatio: "16/10", background: "rgba(99,102,241,0.06)", borderRadius: 5, display: "flex", alignItems: "center", justifyContent: "center", color: C.accent, fontSize: 10, border: `1px solid rgba(99,102,241,0.15)` }}>
                    +{proc.frames.length - 16}
                  </div>
                )}
              </div>
            </div>
          )}
        </div>

        {/* Right: Live narration preview */}
        <div style={{
          borderRadius: 12, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)",
          display: "flex", flexDirection: "column", overflow: "hidden",
        }}>
          <div style={{ padding: "12px 16px", borderBottom: `1px solid ${C.border}`, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <span style={{ fontSize: 12, fontWeight: 600, color: C.dim, textTransform: "uppercase", letterSpacing: 0.8 }}>
              Live Narration Preview
            </span>
            {segCount > 0 && (
              <span style={{ fontSize: 12, color: C.accent }}>{segCount} segment{segCount !== 1 ? "s" : ""}</span>
            )}
          </div>

          {segCount === 0 ? (
            <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", color: C.muted, fontSize: 14 }}>
              {proc.phase === "generating_narration" ? (
                <div style={{ textAlign: "center" }}>
                  <div style={{ width: 24, height: 24, border: "2px solid rgba(99,102,241,0.3)", borderTopColor: C.accent, borderRadius: "50%", animation: "spin 0.8s linear infinite", margin: "0 auto 12px" }} />
                  Waiting for AI response...
                </div>
              ) : "Narration will appear here as it's generated..."}
            </div>
          ) : (
            <div style={{ flex: 1, overflowY: "auto", padding: 16, display: "flex", flexDirection: "column", gap: 8 }}>
              {proc.streamingSegments.map((s, i) => {
                const isLast = i === segCount - 1;
                return (
                  <div key={s.index} style={{
                    padding: "12px 16px", borderRadius: 8,
                    background: isLast ? "rgba(99,102,241,0.06)" : "rgba(255,255,255,0.02)",
                    border: isLast ? "1px solid rgba(99,102,241,0.2)" : `1px solid ${C.border}`,
                    animation: isLast ? "fadeIn 0.3s ease" : "none",
                  }}>
                    <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
                      <span style={{ fontSize: 11, fontFamily: "monospace", color: C.accent, fontWeight: 600 }}>
                        {secondsToTimestamp(s.start_seconds)} - {secondsToTimestamp(s.end_seconds)}
                      </span>
                      <span style={{ fontSize: 10, color: C.muted }}>{s.pace}</span>
                    </div>
                    <p style={{ fontSize: 13, color: isLast ? C.text : C.dim, lineHeight: 1.6, margin: 0 }}>{s.text}</p>
                    {s.visual_description && (
                      <p style={{ fontSize: 11, color: C.muted, marginTop: 6, fontStyle: "italic" }}>{s.visual_description}</p>
                    )}
                  </div>
                );
              })}
            </div>
          )}
        </div>
      </div>

      {/* Error */}
      {proc.error && (
        <div style={{ marginTop: 16, padding: "12px 16px", background: "rgba(239,68,68,0.08)", border: "1px solid rgba(239,68,68,0.2)", borderRadius: 8, fontSize: 13, color: "#f87171" }}>
          {proc.error}
        </div>
      )}

      {/* Actions */}
      <div style={{ display: "flex", justifyContent: "center", gap: 10, marginTop: 16, flexShrink: 0 }}>
        {!["done", "error", "cancelled"].includes(proc.phase) && (
          <Button variant="secondary" onClick={() => cancelGeneration().then(() => proc.setPhase("cancelled"))}>Cancel</Button>
        )}
        {(proc.phase === "error" || proc.phase === "cancelled") && (
          <Button onClick={run}>Retry</Button>
        )}
      </div>

      {/* CSS animations */}
      <style>{`
        @keyframes spin { to { transform: rotate(360deg); } }
        @keyframes fadeIn { from { opacity: 0; transform: translateY(4px); } to { opacity: 1; transform: translateY(0); } }
        @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }
      `}</style>
    </div>
  );
}
