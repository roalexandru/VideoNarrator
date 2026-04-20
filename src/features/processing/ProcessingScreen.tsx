import { useCallback, useState, useEffect, useMemo } from "react";
import { Channel, convertFileSrc } from "@tauri-apps/api/core";
import { useProcessingStore } from "../../stores/processingStore";
import { useProjectStore } from "../../stores/projectStore";
import { useConfigStore } from "../../stores/configStore";
import { useScriptStore } from "../../stores/scriptStore";
import { useEditStore } from "../../stores/editStore";
import { startGeneration, cancelGeneration, applyVideoEdits, getHomeDir, getProviderStatus } from "../../lib/tauri/commands";
import { computeEditPlanHash } from "../../lib/editPlanHash";
import { buildEditPlan } from "../../lib/buildEditPlan";
import { trackEvent, trackError, resetErrorCount } from "../telemetry/analytics";
import { toUserMessage, isContextOverflowError } from "../../lib/errorMessages";
import { recommendedMaxFrames, isReasoningModel } from "../../lib/frameBudget";
import { Button } from "../../components/ui/Button";
import { ProgressBar } from "../../components/ui/ProgressBar";
import type { ProgressEvent, ProcessingPhase } from "../../types/processing";
import type { GenerationParams } from "../../types/config";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };

// Global weighting for the progress bar. The edit channel (separate Tauri
// command) maps 0–100% into 0–EDIT_BUDGET% of the global scale when edits run;
// the main generation channel then maps 0–100% into the remaining slice.
// Without edits the main channel drives the full 0–100%.
const EDIT_BUDGET = 35;

const PHASE_LABELS: Record<ProcessingPhase, string> = {
  idle: "Preparing...",
  applying_edits: "Applying video edits...",
  extracting_frames: "Extracting frames from video...",
  processing_docs: "Processing context documents...",
  generating_narration: "Generating narration with AI...",
  done: "Generation complete!",
  error: "Error occurred",
  cancelled: "Cancelled",
};

// Ordered phase list. `idle` / `done` / `error` / `cancelled` are pseudo-states
// handled by the outer branches and don't appear in the step tracker.
const FULL_PHASES = [
  "applying_edits",
  "extracting_frames",
  "processing_docs",
  "generating_narration",
] as const satisfies ReadonlyArray<ProcessingPhase>;
type ActivePhase = (typeof FULL_PHASES)[number];

const STEP_LABELS: Record<ActivePhase, string> = {
  applying_edits: "Apply Edits",
  extracting_frames: "Extract Frames",
  processing_docs: "Process Docs",
  generating_narration: "Generate Narration",
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

  const run = useCallback(async ({ resume = false }: { resume?: boolean } = {}) => {
    const generationStart = Date.now();
    // Snapshot segments from any prior partial run BEFORE we clear state.
    // On resume we send them to the backend so completed chunks aren't re-billed.
    const resumeSegments = resume ? useProcessingStore.getState().streamingSegments : [];
    if (resume) {
      // Preserve streamingSegments; explicitly reset progress (setProgress
      // treats 0 as explicit reset), clear error + statusMessage so the UI
      // unblocks.
      proc.setError(null);
      proc.setProgress(0);
      proc.setStatusMessage(null);
    } else {
      proc.reset();
    }
    proc.setPhase("extracting_frames");
    trackEvent("generation_started", {
      provider: config.aiProvider,
      model: config.model,
      style: config.style,
      language: config.primaryLanguage,
      has_custom_prompt: !!config.customPrompt.trim(),
      has_context_docs: project.contextDocuments.length > 0,
      doc_count: project.contextDocuments.length,
      resume: resumeSegments.length > 0,
      resume_segments: resumeSegments.length,
    });

    // Snapshot edit state at generation start to prevent mid-run mutations
    const editSnapshot = useEditStore.getState();

    // Decide edit branch BEFORE wiring the main-channel handler so its
    // percent scaling matches.
    let videoPath = project.videoFile!.path;
    const hasEdits = editSnapshot.clips.length > 1
      || editSnapshot.clips.some((c) => c.speed !== 1.0)
      || editSnapshot.clips.some((c) => c.type === 'freeze')
      || editSnapshot.clips.some((c) => !!c.zoomPan)
      || (editSnapshot.effects && editSnapshot.effects.length > 0 && editSnapshot.effects.some((e) => e.type === 'zoom-pan'))
      || (editSnapshot.clips.length === 1 && editSnapshot.sourceDuration > 0
          && (editSnapshot.clips[0].sourceStart > 0.5 || Math.abs(editSnapshot.clips[0].sourceEnd - editSnapshot.sourceDuration) > 0.5));

    // Main generation channel. `progress` events scale into the remaining
    // budget (whole bar when no edits, EDIT_BUDGET→100 when edits ran).
    // Messages become the live sub-label.
    const ch = new Channel<ProgressEvent>();
    ch.onmessage = (e: ProgressEvent) => {
      if (e.kind === "phase_change") proc.setPhase(e.phase);
      else if (e.kind === "progress") {
        const global = hasEdits
          ? EDIT_BUDGET + (e.percent / 100) * (100 - EDIT_BUDGET)
          : e.percent;
        proc.setProgress(global);
        if (e.message !== undefined) proc.setStatusMessage(e.message);
      }
      else if (e.kind === "frame_extracted") proc.appendFrame(e.frame);
      else if (e.kind === "segment_streamed") proc.appendSegment(e.segment);
      else if (e.kind === "segments_replaced") proc.replaceStreamingSegments(e.segments);
      else if (e.kind === "error") proc.setError(e.message);
    };

    if (hasEdits && editSnapshot.clips.length > 0) {
      trackEvent("video_edited", {
        clips_count: editSnapshot.clips.length,
        has_speed_change: editSnapshot.clips.some(c => c.speed !== 1.0),
        has_freeze: editSnapshot.clips.some(c => c.type === 'freeze'),
        has_zoom: editSnapshot.clips.some(c => !!c.zoomPan) || (editSnapshot.effects || []).some(e => e.type === 'zoom-pan'),
        effects_count: (editSnapshot.effects || []).length,
        has_trim: editSnapshot.clips.length === 1 && (editSnapshot.clips[0].sourceStart > 0.5 || Math.abs(editSnapshot.clips[0].sourceEnd - editSnapshot.sourceDuration) > 0.5),
      });
      try {
        // Backend emits phase_change("applying_edits") on this channel before
        // the first percent tick so the step tracker highlights correctly.
        const homeDir = await getHomeDir();
        const ext = videoPath.split(".").pop() || "mp4";
        const editedPath = `${homeDir}/.narrator/projects/${project.projectId}/edited.${ext}`;
        const editPlan = buildEditPlan(editSnapshot.clips, editSnapshot.effects);
        const editCh = new Channel<ProgressEvent>();
        editCh.onmessage = (e: ProgressEvent) => {
          if (e.kind === "phase_change") proc.setPhase(e.phase);
          else if (e.kind === "progress") {
            // Edit channel owns 0 → EDIT_BUDGET% of the global bar.
            proc.setProgress((e.percent / 100) * EDIT_BUDGET);
            if (e.message !== undefined) proc.setStatusMessage(e.message);
          }
          else if (e.kind === "error") proc.setError(e.message);
        };
        videoPath = await applyVideoEdits(videoPath, editedPath, editPlan, editCh);
        editSnapshot.setEditedVideoPath(videoPath);
        // Record the plan hash so Export can detect stale caches later.
        editSnapshot.setEditedVideoPlanHash(
          computeEditPlanHash(editSnapshot.clips, editSnapshot.effects),
        );
      } catch (err: unknown) {
        console.error("apply_video_edits failed:", err);
        trackError("apply_video_edits", err);
        proc.setError(toUserMessage(err));
        proc.setPhase("error");
        return;
      }
    }

    // Derive effective max_frames from the edited video's duration so long
    // videos aren't silently truncated at 30 frames — matches the
    // recommendation shown in Configuration.
    const editedDuration = editSnapshot.clips.reduce((acc, c) => {
      const len = Math.max(0, c.sourceEnd - c.sourceStart);
      const speed = c.speed > 0 ? c.speed : 1;
      return acc + len / speed;
    }, 0);
    const effectiveMaxFrames = Math.max(
      config.maxFrames,
      recommendedMaxFrames(editedDuration, config.frameDensity),
    );
    // Reasoning models reject user-set temperature — send 1.0 so the UI and
    // request agree (backend also strips it, but consistency > silent mutation).
    const effectiveTemperature = isReasoningModel(config.aiProvider, config.model)
      ? 1.0
      : config.temperature;

    const params: GenerationParams = {
      project_id: project.projectId,
      video_path: videoPath,
      document_paths: project.contextDocuments.map((d) => d.path),
      title: project.title, description: project.description,
      style: config.style, primary_language: config.primaryLanguage,
      additional_languages: config.languages.filter((l) => l !== config.primaryLanguage),
      frame_config: { density: config.frameDensity, scene_threshold: config.sceneThreshold, max_frames: effectiveMaxFrames, skip_dedup: true },
      ai_config: { provider: config.aiProvider, model: config.model, temperature: effectiveTemperature },
      custom_prompt: config.customPrompt,
      ...(resumeSegments.length > 0 ? { resume_segments: resumeSegments } : {}),
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

      // Speech-rate validation breakdown. Tells us how often the WORD BUDGET
      // prompt guidance actually lands a clean script, so we can measure
      // whether the upstream fix is pulling its weight or whether most scripts
      // still need compression/padding at export.
      const report = script.speech_rate_report;
      if (report && report.length > 0) {
        let fit = 0, tight = 0, compress = 0, overflow = 0;
        for (const o of report) {
          if (o.severity === "fit") fit++;
          else if (o.severity === "tight") tight++;
          else if (o.severity === "compress") compress++;
          else if (o.severity === "overflow") overflow++;
        }
        trackEvent("export_script_validation", {
          segments_total: report.length,
          segments_fit: fit,
          segments_tight: tight,
          segments_compress: compress,
          segments_overflow: overflow,
          language: config.primaryLanguage,
          style: config.style,
        });
      }
    } catch (err: unknown) {
      trackError("generate_narration", err, { provider: config.aiProvider, model: config.model, style: config.style });
      const errMsg = toUserMessage(err);
      proc.setError(errMsg);
      proc.setPhase("error");
      // Rate-limit: enforce a cooldown so rapid retries don't hammer the API.
      // Context-overflow: do NOT cool down — retry with the same payload will
      // always fail. The user must change settings (density, docs, model) first.
      const errStr = String(err).toLowerCase();
      const isRateLimit = errStr.includes("rate limit") || errStr.includes("429") || errStr.includes("too many requests");
      if (isRateLimit && !isContextOverflowError(err)) {
        setRateLimitCooldown(30);
      }
    }
  }, [project, config, proc, setScript]);

  // Retry preserves accumulated streamingSegments and asks the backend to
  // resume after the last successful chunk. Start Over discards them for a
  // clean run.
  const retry = useCallback(() => run({ resume: true }), [run]);
  const startOver = useCallback(() => {
    setRateLimitCooldown(0);
    proc.reset();
  }, [proc]);

  const hasScript = Object.keys(useScriptStore.getState().scripts).length > 0;

  // If we already have a script and phase is idle (navigated back), show completed
  const showCompleted = hasScript && (proc.phase === "idle" || proc.phase === "done");
  // Show start button when idle with no script (first time on this step)
  const showStart = !hasScript && proc.phase === "idle";

  // `hasEdits` must match the `run()` branch so the step tracker counts right
  // even before the first edit-channel event lands.
  const editSnapshot = useEditStore();
  const editsDetected = editSnapshot.clips.length > 1
    || editSnapshot.clips.some((c) => c.speed !== 1.0)
    || editSnapshot.clips.some((c) => c.type === 'freeze')
    || editSnapshot.clips.some((c) => !!c.zoomPan)
    || (editSnapshot.effects && editSnapshot.effects.length > 0 && editSnapshot.effects.some((e) => e.type === 'zoom-pan'))
    || (editSnapshot.clips.length === 1 && editSnapshot.sourceDuration > 0
        && (editSnapshot.clips[0].sourceStart > 0.5 || Math.abs(editSnapshot.clips[0].sourceEnd - editSnapshot.sourceDuration) > 0.5));
  // Include an observed `applying_edits` phase too — covers edge cases where
  // the edit store was wiped between render cycles while the backend is mid-run.
  const showEditStep = editsDetected || proc.phase === "applying_edits";

  const activePhases: readonly ActivePhase[] = useMemo(
    () => (showEditStep ? FULL_PHASES : (FULL_PHASES.slice(1) as unknown as readonly ActivePhase[])),
    [showEditStep],
  );
  const activeIdx = activePhases.indexOf(proc.phase as ActivePhase);

  // Backend emits a monotonic percent that the frontend scales into the
  // global bar. No phase-index math — we just render the store value.
  const pct = showCompleted ? 100
    : proc.phase === "done" ? 100
    : Math.min(100, Math.max(0, proc.progress));

  const elapsed = proc.frames.length > 0 ? `${proc.frames.length} frames extracted` : "";
  const segCount = proc.streamingSegments.length;
  const scriptSegCount = Object.values(useScriptStore.getState().scripts)[0]?.segments.length || 0;

  const statusLine = proc.statusMessage ?? PHASE_LABELS[proc.phase];

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
          <Button onClick={() => run()} disabled={blocked}>Start Generation</Button>
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
            <Button onClick={() => run()}>Regenerate</Button>
          </div>
        </div>
      </div>
    );
  }

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", maxWidth: 1100, margin: "0 auto", width: "100%" }}>
      {/* Header */}
      <div style={{ marginBottom: 20 }}>
        <h2 style={{ fontSize: 22, fontWeight: 700, color: C.text }}>Processing</h2>
        <p style={{ color: C.dim, marginTop: 4, fontSize: 14 }}>{statusLine}</p>
      </div>

      {/* Progress bar with percentage */}
      <div style={{ marginBottom: 24 }}>
        <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 6 }}>
          <span style={{ fontSize: 12, color: C.dim }}>
            {elapsed}{elapsed && segCount > 0 ? " · " : ""}{segCount > 0 ? `${segCount} segments generated` : ""}
          </span>
          <span style={{ fontSize: 12, color: C.accent, fontWeight: 600 }}>{Math.round(pct)}%</span>
        </div>
        <ProgressBar value={pct} height={6} />
      </div>

      {/* Horizontal step tracker */}
      <div style={{ display: "flex", gap: 12, marginBottom: 28 }}>
        {activePhases.map((p, i) => {
          const active = proc.phase === p;
          const done = proc.phase === "done" || (activeIdx > i && activeIdx >= 0);
          return (
            <div key={p} style={{
              flex: 1, minWidth: 0,
              display: "flex", alignItems: "center", gap: 10, padding: "12px 14px",
              borderRadius: 10,
              background: active ? "rgba(99,102,241,0.06)" : "rgba(255,255,255,0.02)",
              border: active ? "1px solid rgba(99,102,241,0.2)" : `1px solid ${C.border}`,
            }}>
              <div style={{
                width: 32, height: 32, borderRadius: 9, display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0,
                background: done ? "rgba(34,197,94,0.12)" : active ? "rgba(99,102,241,0.15)" : "rgba(255,255,255,0.04)",
                color: done ? "#4ade80" : active ? C.accent : C.muted,
              }}>
                {done ? <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round"><polyline points="20 6 9 17 4 12"/></svg>
                  : active ? <div style={{ width: 7, height: 7, borderRadius: "50%", background: C.accent, animation: "pulse 1.5s infinite" }} />
                  : <span style={{ fontSize: 12, fontWeight: 600 }}>{i + 1}</span>}
              </div>
              <div style={{ minWidth: 0, flex: 1 }}>
                <div style={{ fontSize: 13, fontWeight: active ? 600 : 400, color: active ? C.text : C.dim, whiteSpace: "nowrap", overflow: "hidden", textOverflow: "ellipsis" }}>{STEP_LABELS[p]}</div>
              </div>
            </div>
          );
        })}
      </div>

      {/* Filmstrip — full width, auto-fills across the row */}
      {proc.frames.length > 0 && (
        <div style={{ marginBottom: 20 }}>
          <div style={{ fontSize: 11, fontWeight: 600, color: C.muted, marginBottom: 8, textTransform: "uppercase", letterSpacing: 0.8 }}>
            Extracted Frames
          </div>
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(96px, 1fr))", gap: 6 }}>
            {proc.frames.slice(0, 32).map((f) => (
              <div key={f.index} style={{
                aspectRatio: "16/10", borderRadius: 6, overflow: "hidden", position: "relative",
                border: `1px solid ${C.border}`,
              }}>
                <img
                  src={convertFileSrc(typeof f.path === 'string' ? f.path : (f.path as { toString(): string }).toString())}
                  alt={`Frame at ${f.timestamp_seconds.toFixed(2)}s`}
                  style={{ width: "100%", height: "100%", objectFit: "cover", display: "block" }}
                  onError={(e) => { (e.target as HTMLImageElement).style.display = "none"; }}
                />
                <div style={{
                  position: "absolute", bottom: 2, left: 2, fontSize: 9, fontWeight: 700,
                  color: "#fff", background: "rgba(0,0,0,0.6)", padding: "1px 4px", borderRadius: 3,
                }}>
                  {f.timestamp_seconds.toFixed(1)}s
                </div>
              </div>
            ))}
            {proc.frames.length > 32 && (
              <div style={{ aspectRatio: "16/10", background: "rgba(99,102,241,0.06)", borderRadius: 6, display: "flex", alignItems: "center", justifyContent: "center", color: C.accent, fontSize: 11, border: `1px solid rgba(99,102,241,0.15)` }}>
                +{proc.frames.length - 32}
              </div>
            )}
          </div>
        </div>
      )}

      <div style={{ flex: 1 }} />

      {/* Error — click to copy */}
      {proc.error && (
        <div
          onClick={() => { navigator.clipboard.writeText(proc.error!).catch(() => {}); }}
          title="Click to copy error message"
          style={{ marginTop: 16, padding: "12px 16px", background: "rgba(239,68,68,0.08)", border: "1px solid rgba(239,68,68,0.2)", borderRadius: 8, fontSize: 13, color: "#f87171", cursor: "pointer", userSelect: "text" }}>
          {proc.error}
        </div>
      )}

      {/* Actions */}
      <div style={{ display: "flex", justifyContent: "center", gap: 10, marginTop: 16, flexShrink: 0 }}>
        {!["done", "error", "cancelled"].includes(proc.phase) && (
          <Button variant="secondary" onClick={() => cancelGeneration().then(() => proc.setPhase("cancelled"))}>Cancel</Button>
        )}
        {(proc.phase === "error" || proc.phase === "cancelled") && (
          <>
            {/* Start Over is the always-available escape from a retry-fail loop.
               It clears progress so the user isn't forced to retry. */}
            <Button variant="secondary" onClick={startOver}>Start Over</Button>
            <Button onClick={retry} disabled={rateLimitCooldown > 0}>
              {rateLimitCooldown > 0
                ? `Wait ${rateLimitCooldown}s (rate limited)`
                : proc.streamingSegments.length > 0
                  ? `Resume (${proc.streamingSegments.length} saved)`
                  : "Retry"}
            </Button>
          </>
        )}
      </div>

      {/* CSS animations */}
      <style>{`
        @keyframes pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.4; } }
      `}</style>
    </div>
  );
}
