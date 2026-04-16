import { useCallback, useState, useEffect } from "react";
import { Channel } from "@tauri-apps/api/core";
import { useProcessingStore } from "../../stores/processingStore";
import { useProjectStore } from "../../stores/projectStore";
import { useConfigStore } from "../../stores/configStore";
import { useScriptStore } from "../../stores/scriptStore";
import { useEditStore } from "../../stores/editStore";
import { startGeneration, cancelGeneration, applyVideoEdits, getHomeDir, getProviderStatus } from "../../lib/tauri/commands";
import { trackEvent, trackError, resetErrorCount } from "../telemetry/analytics";
import { toUserMessage } from "../../lib/errorMessages";
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
  const [rateLimitCooldown, setRateLimitCooldown] = useState(0);

  // Rate limit cooldown timer
  useEffect(() => {
    if (rateLimitCooldown <= 0) return;
    const timer = setInterval(() => setRateLimitCooldown((v) => Math.max(0, v - 1)), 1000);
    return () => clearInterval(timer);
  }, [rateLimitCooldown]);

  const run = useCallback(async () => {
    const generationStart = Date.now();
    proc.reset(); proc.setPhase("extracting_frames");
    trackEvent("generation_started", {
      provider: config.aiProvider,
      model: config.model,
      style: config.style,
      language: config.primaryLanguage,
      has_custom_prompt: !!config.customPrompt.trim(),
      has_context_docs: project.contextDocuments.length > 0,
      doc_count: project.contextDocuments.length,
    });

    // Snapshot edit state at generation start to prevent mid-run mutations
    const editSnapshot = useEditStore.getState();

    const ch = new Channel<ProgressEvent>();
    ch.onmessage = (e: ProgressEvent) => {
      if (e.kind === "phase_change") proc.setPhase(e.phase);
      else if (e.kind === "progress") proc.setProgress(e.percent);
      else if (e.kind === "frame_extracted") proc.appendFrame(e.frame);
      else if (e.kind === "segment_streamed") proc.appendSegment(e.segment);
      else if (e.kind === "error") proc.setError(e.message);
    };

    // Check if video edits need to be applied first
    let videoPath = project.videoFile!.path;
    const hasEdits = editSnapshot.clips.length > 1
      || editSnapshot.clips.some((c) => c.speed !== 1.0)
      || (editSnapshot.clips.length === 1 && editSnapshot.sourceDuration > 0
          && (editSnapshot.clips[0].sourceStart > 0.5 || Math.abs(editSnapshot.clips[0].sourceEnd - editSnapshot.sourceDuration) > 0.5));

    if (hasEdits && editSnapshot.clips.length > 0) {
      trackEvent("video_edited", {
        clips_count: editSnapshot.clips.length,
        has_speed_change: editSnapshot.clips.some(c => c.speed !== 1.0),
        has_trim: editSnapshot.clips.length === 1 && (editSnapshot.clips[0].sourceStart > 0.5 || Math.abs(editSnapshot.clips[0].sourceEnd - editSnapshot.sourceDuration) > 0.5),
      });
      try {
        proc.setPhase("extracting_frames"); // reuse phase for "applying edits" visual
        const homeDir = await getHomeDir();
        const ext = videoPath.split(".").pop() || "mp4";
        const editedPath = `${homeDir}/.narrator/projects/${project.projectId}/edited.${ext}`;
        // Map effects from the effects track onto clips for the Rust backend.
        // The Rust pipeline processes zoom_pan per-clip, so we find which effect overlaps each clip.
        const effectsTrack = editSnapshot.effects || [];
        let cumOutTime = 0;
        const editPlan = {
          clips: editSnapshot.clips.map((c) => {
            const clipDur = c.type === 'freeze' ? (c.freezeDuration ?? 3) : (c.sourceEnd - c.sourceStart) / c.speed;
            const clipStart = cumOutTime;
            const clipEnd = cumOutTime + clipDur;
            cumOutTime = clipEnd;
            // Find the first zoom-pan effect that overlaps this clip
            const overlapping = effectsTrack.find((e) =>
              e.type === 'zoom-pan' && e.zoomPan && e.startTime < clipEnd && e.endTime > clipStart
            );
            // Use effect's zoom_pan if found, otherwise fall back to clip-level zoom_pan
            const zoomPan = overlapping?.zoomPan ?? c.zoomPan;
            return {
              start_seconds: c.sourceStart,
              end_seconds: c.sourceEnd,
              speed: c.speed,
              fps_override: c.fpsOverride,
              clip_type: c.type ?? 'normal',
              freeze_source_time: c.freezeSourceTime,
              freeze_duration: c.freezeDuration,
              zoom_pan: zoomPan ? {
                startRegion: zoomPan.startRegion,
                endRegion: zoomPan.endRegion,
                easing: zoomPan.easing,
              } : null,
            };
          }),
        };
        const editCh = new Channel<ProgressEvent>();
        editCh.onmessage = (e: ProgressEvent) => {
          if (e.kind === "progress") proc.setProgress(e.percent * 0.3); // 0-30% for edits
        };
        videoPath = await applyVideoEdits(videoPath, editedPath, editPlan, editCh);
        editSnapshot.setEditedVideoPath(videoPath);
      } catch (err: unknown) {
        trackError("apply_video_edits", err);
        proc.setError(toUserMessage(err));
        proc.setPhase("error");
        return;
      }
    }

    const params: GenerationParams = {
      project_id: project.projectId,
      video_path: videoPath,
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
      resetErrorCount("generate_narration");
      const wallTime = Math.round((Date.now() - generationStart) / 1000);
      trackEvent("processing_completed", {
        segments: script.segments.length,
        provider: config.aiProvider,
        model: config.model,
        style: config.style,
        language: config.primaryLanguage,
        duration_s: Math.round(script.total_duration_seconds),
        wall_time_s: wallTime,
        has_edits: editSnapshot.clips.length > 1 || editSnapshot.clips.some((c) => c.speed !== 1.0),
        frame_density: config.frameDensity,
      });
    } catch (err: unknown) {
      trackError("generate_narration", err, { provider: config.aiProvider, model: config.model, style: config.style });
      const errMsg = toUserMessage(err);
      proc.setError(errMsg);
      proc.setPhase("error");
      // Detect rate limit errors and enforce cooldown to prevent retry spam
      const errStr = String(err).toLowerCase();
      if (errStr.includes("rate limit") || errStr.includes("429") || errStr.includes("too many requests")) {
        setRateLimitCooldown(30);
      }
    }
  }, [project, config, proc, setScript]);

  const hasScript = Object.keys(useScriptStore.getState().scripts).length > 0;

  // If we already have a script and phase is idle (navigated back), show completed
  const showCompleted = hasScript && (proc.phase === "idle" || proc.phase === "done");
  // Show start button when idle with no script (first time on this step)
  const showStart = !hasScript && proc.phase === "idle";

  const phases = ["extracting_frames", "processing_docs", "generating_narration"] as const;
  const pi = phases.indexOf(proc.phase as any);
  const pct = showCompleted ? 100 : proc.phase === "done" ? 100 : proc.phase === "generating_narration" && proc.progress > 0 ? 67 + (proc.progress / 100) * 33 : (proc.phase === "extracting_frames" && proc.frames.length === 0) ? 5 : pi >= 0 ? ((pi + 0.5) / 3) * 100 : 5;
  const elapsed = proc.frames.length > 0 ? `${proc.frames.length} frames extracted` : "";
  const segCount = proc.streamingSegments.length;
  const scriptSegCount = Object.values(useScriptStore.getState().scripts)[0]?.segments.length || 0;

  const [hasApiKey, setHasApiKey] = useState(true);
  useEffect(() => {
    getProviderStatus().then((statuses) => {
      const current = statuses.find((s) => s.provider === config.aiProvider);
      setHasApiKey(current?.has_key ?? false);
    }).catch(() => {});
  }, [config.aiProvider]);

  const noVideo = !project.videoFile?.path;
  const noApiKey = !hasApiKey;

  if (showStart) {
    const blocked = noVideo || noApiKey;
    return (
      <div style={{ maxWidth: 640, margin: "0 auto" }}>
        <div style={{ marginBottom: 32 }}>
          <h2 style={{ fontSize: 22, fontWeight: 700, color: C.text }}>Processing</h2>
          <p style={{ color: C.dim, marginTop: 4, fontSize: 14 }}>Ready to generate narration for your video.</p>
        </div>
        <div style={{ padding: "32px 24px", borderRadius: 12, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)", textAlign: "center" }}>
          <svg width="40" height="40" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="1.5" strokeLinecap="round" style={{ margin: "0 auto 12px", display: "block" }}>
            <polygon points="5 3 19 12 5 21 5 3" />
          </svg>
          <p style={{ fontSize: 16, fontWeight: 600, color: C.text, marginBottom: 6 }}>Generate Narration</p>
          <p style={{ fontSize: 13, color: C.dim, marginBottom: 20 }}>This will extract frames, analyze the video, and generate a narration script using AI.</p>
          {noVideo && <p style={{ fontSize: 12, color: "#fb923c", marginBottom: 12 }}>No video selected. Go to Project Setup to add one.</p>}
          {noApiKey && <p style={{ fontSize: 12, color: "#fb923c", marginBottom: 12 }}>No API key for {config.aiProvider}. Go to Settings to add one.</p>}
          <Button onClick={run} disabled={blocked}>Start Generation</Button>
        </div>
      </div>
    );
  }

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
          <Button onClick={run} disabled={rateLimitCooldown > 0}>
            {rateLimitCooldown > 0 ? `Wait ${rateLimitCooldown}s (rate limited)` : "Retry"}
          </Button>
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
