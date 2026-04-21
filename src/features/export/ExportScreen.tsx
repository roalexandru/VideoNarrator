import { useState, useEffect, useCallback } from "react";
import { Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useExportStore, slugify } from "../../stores/exportStore";
import { useScriptStore } from "../../stores/scriptStore";
import { useConfigStore } from "../../stores/configStore";
import { useProjectStore } from "../../stores/projectStore";
import { useEditStore } from "../../stores/editStore";
import {
  exportScript, getElevenLabsConfig, saveElevenLabsConfig, listElevenLabsVoices,
  generateTts, mergeAudioVideo, burnSubtitles, openFolder, getHomeDir,
  getAzureTtsConfig, saveAzureTtsConfig, applyVideoEdits, fileExists,
  type ElevenLabsConfig, type ElevenLabsVoice, type AzureTtsConfig,
} from "../../lib/tauri/commands";
import { buildEditPlan, planRequiresRender } from "../../lib/buildEditPlan";
import { predictExport } from "../../lib/speechRate";
import { computeEditPlanHash } from "../../lib/editPlanHash";
import { EXPORT_FORMATS } from "../../lib/constants";
import { useOpenSettings } from "../../contexts/SettingsContext";
import { trackError } from "../telemetry/analytics";
import { toUserMessage } from "../../lib/errorMessages";
import { trackEvent } from "../telemetry/analytics";
import { Button } from "../../components/ui/Button";
import { ProgressBar } from "../../components/ui/ProgressBar";
import type { ExportResult } from "../../types/export";
import type { ProgressEvent } from "../../types/processing";
import { PRESET_LABELS, PRESET_DESCRIPTIONS, type SubtitlePreset } from "./subtitlePresets";

// ── Design tokens ──
const C = {
  text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e",
  border: "rgba(255,255,255,0.07)", borderHover: "rgba(255,255,255,0.12)",
  accent: "#818cf8", accentDim: "rgba(99,102,241,0.15)",
  bg: "rgba(255,255,255,0.02)", bgHover: "rgba(255,255,255,0.04)",
  success: "#4ade80", error: "#f87171",
};

// ── Section wrapper ──
function Section({ title, icon, children, collapsible, defaultOpen = true }: {
  title: string; icon: React.ReactNode; children: React.ReactNode;
  collapsible?: boolean; defaultOpen?: boolean;
}) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div style={{ borderRadius: 12, border: `1px solid ${C.border}`, background: C.bg, overflow: "hidden" }}>
      <div
        onClick={collapsible ? () => setOpen(!open) : undefined}
        style={{
          display: "flex", alignItems: "center", gap: 8, padding: "14px 18px",
          cursor: collapsible ? "pointer" : "default", userSelect: "none",
          borderBottom: open ? `1px solid ${C.border}` : "none",
        }}
      >
        {icon}
        <span style={{ fontSize: 13, fontWeight: 700, color: C.text, flex: 1, letterSpacing: 0.3 }}>{title}</span>
        {collapsible && (
          <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke={C.muted} strokeWidth="2" strokeLinecap="round"
            style={{ transform: open ? "rotate(180deg)" : "rotate(0deg)", transition: "transform 0.2s" }}>
            <polyline points="6 9 12 15 18 9" />
          </svg>
        )}
      </div>
      {open && <div style={{ padding: "16px 18px" }}>{children}</div>}
    </div>
  );
}

// ── Toggle switch ──
function Toggle({ checked, onChange, label }: { checked: boolean; onChange: (v: boolean) => void; label: string }) {
  return (
    <label style={{ display: "flex", alignItems: "center", gap: 10, cursor: "pointer", fontSize: 13, color: C.dim }}>
      <div
        onClick={() => onChange(!checked)}
        style={{
          width: 36, height: 20, borderRadius: 10, position: "relative",
          background: checked ? C.accent : "rgba(255,255,255,0.1)",
          transition: "background 0.2s", cursor: "pointer",
        }}
      >
        <div style={{
          position: "absolute", top: 2, left: checked ? 18 : 2,
          width: 16, height: 16, borderRadius: "50%", background: "#fff",
          transition: "left 0.2s", boxShadow: "0 1px 3px rgba(0,0,0,0.3)",
        }} />
      </div>
      {label}
    </label>
  );
}

// ── Radio option ──
function Radio({ checked, onClick, label }: { checked: boolean; onClick: () => void; label: string }) {
  return (
    <label onClick={onClick} style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer", fontSize: 13, color: C.dim }}>
      <div style={{
        width: 16, height: 16, borderRadius: "50%",
        border: `2px solid ${checked ? C.accent : C.muted}`,
        display: "flex", alignItems: "center", justifyContent: "center",
      }}>
        {checked && <div style={{ width: 8, height: 8, borderRadius: "50%", background: C.accent }} />}
      </div>
      {label}
    </label>
  );
}

// ── Inline file output preview ──
function FilePreview({ name }: { name: string }) {
  return (
    <div style={{ display: "flex", alignItems: "center", gap: 6, padding: "6px 10px", borderRadius: 6, background: "rgba(255,255,255,0.03)", fontSize: 12, color: C.dim, fontFamily: "monospace" }}>
      <span style={{ color: C.muted }}>→</span> {name}
    </div>
  );
}

export function ExportScreen() {
  const exp = useExportStore();
  const scripts = useScriptStore((s) => s.scripts);
  const languages = useConfigStore((s) => s.languages);
  const projectTitle = useProjectStore((s) => s.title);
  // Export uses the EDITED video (with speed changes, zooms, and effects baked in)
  // when edits were applied; falls back to the original if no edits were made.
  const originalVideoPath = useProjectStore((s) => s.videoFile?.path);
  const editedVideoPath = useEditStore((s) => s.editedVideoPath);
  const videoPath = editedVideoPath || originalVideoPath;

  // Script export
  const [scriptResults, setScriptResults] = useState<ExportResult[]>([]);
  const [scriptExporting, setScriptExporting] = useState(false);
  const [copied, setCopied] = useState(false);

  // TTS / Voice
  const [elConfig, setElConfig] = useState<ElevenLabsConfig | null>(null);
  const [elVoices, setElVoices] = useState<ElevenLabsVoice[]>([]);
  const [azureConfig, setAzureConfig] = useState<AzureTtsConfig | null>(null);
  const ttsProviderRaw = useConfigStore((s) => s.ttsProvider);
  const openSettings = useOpenSettings();

  // Build the full TTS provider string — for builtin, include voice and speed from localStorage
  const ttsProvider = (() => {
    if (ttsProviderRaw !== "builtin") return ttsProviderRaw;
    try {
      const saved = localStorage.getItem("narrator_builtin_tts");
      if (saved) {
        const { voice, speed } = JSON.parse(saved);
        return `builtin:${voice || "default"}:${speed || 1.0}`;
      }
    } catch {}
    return "builtin:default:1.0";
  })();

  // Telemetry-safe provider label. The full `ttsProvider` string for
  // builtin looks like `builtin:Samantha:1.0` — the voice name comes from
  // the host OS ("Samantha" is macOS-only) and leaks the user's platform.
  // For analytics we only need the category, so collapse builtin:* to
  // just "builtin" and keep Azure/ElevenLabs as the literals they already
  // are.
  const ttsProviderTelemetry = ttsProvider.startsWith("builtin") ? "builtin" : ttsProvider;

  // Video export pipeline
  const [videoPhase, setVideoPhase] = useState<"idle" | "rendering" | "audio" | "merge" | "subtitles" | "done" | "error">("idle");
  const [videoProgress, setVideoProgress] = useState(0);
  const [videoError, setVideoError] = useState<string | null>(null);
  const [, setVideoOutputPath] = useState<string | null>(null);
  const [videoNotice, setVideoNotice] = useState<string | null>(null);

  // Audio-only export
  const [audioPhase, setAudioPhase] = useState<"idle" | "generating" | "done" | "error">("idle");
  const [audioProgress, setAudioProgress] = useState(0);
  const [, setAudioOutputPath] = useState<string | null>(null);
  const [audioError, setAudioError] = useState<string | null>(null);

  const hasTtsKey = ttsProvider.startsWith("builtin") ? true : ttsProviderRaw === "elevenlabs" ? !!elConfig?.api_key : !!azureConfig?.api_key;
  const primaryScript = Object.values(scripts)[0];
  const segCount = primaryScript?.segments.length || 0;

  // Init output directory + basename from title (fallback to "untitled" if empty)
  useEffect(() => {
    const title = projectTitle || "untitled";
    getHomeDir().then((home) => {
      exp.initFromTitle(title, home);
    }).catch(() => {
      exp.initFromTitle(title, "/tmp");
    });
  }, [projectTitle]);

  // Load TTS config
  useEffect(() => {
    getElevenLabsConfig().then((cfg) => {
      if (cfg?.api_key) {
        setElConfig(cfg);
        listElevenLabsVoices(cfg.api_key).then(setElVoices).catch((err: unknown) => {
          console.error("Failed to load export config:", err);
        });
      }
    }).catch((err: unknown) => {
      console.error("Failed to load export config:", err);
    });
    getAzureTtsConfig().then(setAzureConfig).catch((err: unknown) => {
      console.error("Failed to load export config:", err);
    });
  }, []);

  const changePath = async () => {
    const d = await open({ directory: true });
    if (d && typeof d === "string") exp.setOutputDirectory(d);
  };

  const getScript = useCallback(() => {
    const lang = Object.entries(exp.languageToggles).find(([, on]) => on)?.[0];
    return lang ? scripts[lang] : primaryScript;
  }, [exp.languageToggles, scripts, primaryScript]);

  // ── Export Video pipeline ──
  const doExportVideo = async () => {
    if (!exp.outputDirectory || !videoPath) return;
    if (ttsProvider === "elevenlabs" && !elConfig) return;
    if (ttsProvider === "azure" && !azureConfig) return;
    // builtin provider needs no config check

    if (ttsProvider === "elevenlabs" && elConfig) await saveElevenLabsConfig(elConfig);
    if (ttsProvider === "azure" && azureConfig) await saveAzureTtsConfig(azureConfig);

    const script = getScript();
    if (!script) return;

    const dir = exp.outputDirectory;
    const base = exp.basename;

    setVideoPhase("idle");
    setVideoProgress(0);
    setVideoError(null);
    setVideoOutputPath(null);
    setVideoNotice(null);
    const exportStart = Date.now();

    try {
      // ── Phase 0: ensure the edited video is present and up-to-date ──
      // The source for the final export is the EDITED video (with clips,
      // zoom/pan, spotlight, blur, text, fade baked in). If the cache is
      // missing or stale, regenerate it here before muxing the audio.
      const editState = useEditStore.getState();
      const needsEdit = planRequiresRender(editState.clips, editState.effects);
      let sourceVideoPath = videoPath;
      if (needsEdit && originalVideoPath) {
        const currentHash = computeEditPlanHash(editState.clips, editState.effects);
        const cacheMatches =
          editState.editedVideoPath &&
          editState.editedVideoPlanHash === currentHash &&
          (await fileExists(editState.editedVideoPath));

        if (!cacheMatches) {
          setVideoPhase("rendering");
          setVideoProgress(0);
          const home = await getHomeDir();
          const projectId = useProjectStore.getState().projectId;
          const srcExt = originalVideoPath.split(".").pop() || "mp4";
          const editedPath = `${home}/.narrator/projects/${projectId}/edited.${srcExt}`;
          const plan = buildEditPlan(editState.clips, editState.effects);
          const renderCh = new Channel<ProgressEvent>();
          renderCh.onmessage = (e: ProgressEvent) => {
            if (e.kind === "progress") setVideoProgress(e.percent * 0.25); // 0-25%
          };
          const rendered = await applyVideoEdits(
            originalVideoPath,
            editedPath,
            plan,
            renderCh,
          );
          editState.setEditedVideoPath(rendered);
          editState.setEditedVideoPlanHash(currentHash);
          sourceVideoPath = rendered;
        } else {
          sourceVideoPath = editState.editedVideoPath!;
        }
      }

      setVideoPhase("audio");
      setVideoProgress(25);
      // Ensure directory exists
      // Phase 1: Generate TTS audio
      const ch = new Channel<ProgressEvent>();
      ch.onmessage = (e: ProgressEvent) => {
        if (e.kind === "progress") setVideoProgress(25 + e.percent * 0.35); // 25-60%
      };

      const audioDir = `${dir}/audio`;
      const ttsResults = await generateTts(script.segments, audioDir, true, ch, ttsProvider);
      const ttsOk = ttsResults.filter((r) => r.success);
      if (ttsOk.length === 0) {
        const firstError = ttsResults.find((r) => r.error)?.error || "Unknown TTS error";
        throw new Error(`Audio generation failed: ${firstError}`);
      }
      const ttsFailed = ttsResults.filter((r) => !r.success);
      if (ttsFailed.length > 0 && ttsOk.length > 0) {
        console.warn(`${ttsFailed.length} of ${ttsResults.length} TTS segments failed:`, ttsFailed.map(r => r.error).join("; "));
      }

      const audioFile = ttsOk[0].file_path;

      // Phase 2: Merge audio with video
      setVideoPhase("merge");
      setVideoProgress(65);

      const finalPath = `${dir}/${base}.mp4`;

      if (exp.burnSubtitles) {
        // Merge to temp file first, then burn subtitles to final.
        // Burn is the slowest phase (full re-encode), so give it the larger
        // slice of the progress bar: merge 65–75 %, burn 75–100 %.
        const mergedPath = `${dir}/${base}_merged.mp4`;
        const mergeChannel = new Channel<ProgressEvent>();
        mergeChannel.onmessage = (e: ProgressEvent) => {
          if (e.kind === "progress") setVideoProgress(65 + e.percent * 0.10); // 65-75%
        };
        const mergeOutcome = await mergeAudioVideo(sourceVideoPath, audioFile, mergedPath, exp.replaceAudio, mergeChannel, exp.duckDb);
        if (mergeOutcome.fell_back_to_narration_only && !exp.replaceAudio) {
          setVideoNotice("Source video had no audio track — exported with narration only.");
        }
        setVideoProgress(75);

        setVideoPhase("subtitles");
        const burnChannel = new Channel<ProgressEvent>();
        burnChannel.onmessage = (e: ProgressEvent) => {
          if (e.kind === "progress") setVideoProgress(75 + e.percent * 0.25); // 75-100%
        };
        await burnSubtitles(
          mergedPath,
          script,
          finalPath,
          burnChannel,
          {
            font_size: exp.subtitleFontSize,
            color: exp.subtitleColor,
            outline_color: exp.subtitleOutlineColor,
            outline: exp.subtitleOutline,
            position: exp.subtitlePosition,
            text_transform: exp.subtitleTextTransform,
            max_words_per_line: exp.subtitleMaxWordsPerLine,
          },
          audioDir,
          mergedPath,
        );
      } else {
        // Merge directly to final path
        const mergeChannel = new Channel<ProgressEvent>();
        mergeChannel.onmessage = (e: ProgressEvent) => {
          if (e.kind === "progress") setVideoProgress(65 + e.percent * 0.35); // 65-100%
        };
        const mergeOutcome = await mergeAudioVideo(sourceVideoPath, audioFile, finalPath, exp.replaceAudio, mergeChannel, exp.duckDb);
        if (mergeOutcome.fell_back_to_narration_only && !exp.replaceAudio) {
          setVideoNotice("Source video had no audio track — exported with narration only.");
        }
      }

      setVideoProgress(100);
      setVideoOutputPath(finalPath);
      setVideoPhase("done");
      trackEvent("export_completed", { type: "video", tts_provider: ttsProviderTelemetry, burn_subtitles: exp.burnSubtitles, replace_audio: exp.replaceAudio, wall_time_s: Math.round((Date.now() - exportStart) / 1000) });

      // Predicted compression / padding stats — computed from the same
      // deterministic math Export uses (see speechRate.predictExport).
      // We don't plumb per-segment actuals back through IPC because the
      // prediction is provably accurate for the non-error path.
      const scriptLang = script.metadata.language || "en";
      const videoDur = script.total_duration_seconds || 0;
      // Same speed multiplier the backend applies, so the prediction matches
      // the actual compression/padding counters at export time.
      const ttsSpeed = (() => {
        if (ttsProviderRaw === "elevenlabs") return elConfig?.speed || 1.0;
        if (ttsProviderRaw === "azure") return azureConfig?.speed || 1.0;
        // builtin: parse from ttsProvider string "builtin:voice:speed"
        const parts = ttsProvider.split(":");
        return Number.parseFloat(parts[2] || "1.0") || 1.0;
      })();
      const prediction = predictExport(script.segments, scriptLang, videoDur, ttsSpeed);
      trackEvent("export_tts_compression", {
        segments_total: script.segments.length,
        segments_compressed: prediction.compressed,
        segments_over_cap: prediction.overCap,
        video_padded_ms: Math.round(prediction.padSeconds * 1000),
        language: scriptLang,
        tts_provider: ttsProviderTelemetry,
      });
    } catch (e: any) {
      console.error("Export video:", e);
      trackError("export_video", e, { tts_provider: ttsProviderTelemetry, burn_subtitles: exp.burnSubtitles, replace_audio: exp.replaceAudio });
      setVideoError(toUserMessage(e));
      setVideoPhase("error");
    }
  };

  // ── Export Audio Only ──
  const doExportAudio = async () => {
    if (!exp.outputDirectory) return;
    if (ttsProvider === "elevenlabs" && !elConfig) return;
    if (ttsProvider === "azure" && !azureConfig) return;
    // builtin provider needs no config check

    if (ttsProvider === "elevenlabs" && elConfig) await saveElevenLabsConfig(elConfig);
    if (ttsProvider === "azure" && azureConfig) await saveAzureTtsConfig(azureConfig);

    const script = getScript();
    if (!script) return;

    setAudioPhase("generating");
    setAudioProgress(0);
    setAudioError(null);
    setAudioOutputPath(null);
    const audioExportStart = Date.now();

    try {
      const ch = new Channel<ProgressEvent>();
      ch.onmessage = (e: ProgressEvent) => {
        if (e.kind === "progress") setAudioProgress(e.percent);
      };

      const audioDir = `${exp.outputDirectory}/audio`;
      const ttsResults = await generateTts(script.segments, audioDir, true, ch, ttsProvider);
      const ttsOk = ttsResults.filter((r) => r.success);
      if (ttsOk.length === 0) {
        const firstError = ttsResults.find((r) => r.error)?.error || "Unknown TTS error";
        throw new Error(`Audio generation failed: ${firstError}`);
      }
      const ttsFailed = ttsResults.filter((r) => !r.success);
      if (ttsFailed.length > 0 && ttsOk.length > 0) {
        console.warn(`${ttsFailed.length} of ${ttsResults.length} TTS segments failed:`, ttsFailed.map(r => r.error).join("; "));
      }

      setAudioOutputPath(ttsOk[0].file_path);
      setAudioPhase("done");
      trackEvent("export_completed", { type: "audio", tts_provider: ttsProviderTelemetry, wall_time_s: Math.round((Date.now() - audioExportStart) / 1000) });
    } catch (e: any) {
      console.error("Export audio:", e);
      trackError("export_audio", e, { tts_provider: ttsProviderTelemetry });
      setAudioError(toUserMessage(e));
      setAudioPhase("error");
    }
  };

  // ── Export Scripts ──
  const doExportScripts = async () => {
    if (!exp.outputDirectory || !exp.selectedFormats.length) return;
    setScriptExporting(true);
    try {
      const langs = Object.entries(exp.languageToggles).filter(([, on]) => on).map(([l]) => l);
      const results = await exportScript({
        formats: exp.selectedFormats,
        languages: langs,
        output_directory: exp.outputDirectory,
        scripts,
        basename: exp.basename,
      });
      setScriptResults(results);
      trackEvent("export_completed", { type: "script", format_count: results.length, formats: exp.selectedFormats.join(","), language_count: langs.length });
    } catch (e) {
      console.error(e);
      trackError("export_script", e);
    } finally {
      setScriptExporting(false);
    }
  };

  const copyScript = () => {
    if (!primaryScript) return;
    navigator.clipboard.writeText(
      primaryScript.segments.map((seg) => `[${seg.start_seconds.toFixed(1)}s - ${seg.end_seconds.toFixed(1)}s]\n${seg.text}`).join("\n\n")
    );
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  };

  const scriptSuccessCount = scriptResults.filter((r) => r.success).length;
  const phaseLabels: Record<string, string> = {
    rendering: "Rendering edited video (applying effects)...",
    audio: "Generating audio...",
    merge: "Merging audio with video...",
    subtitles: "Burning subtitles...",
    done: "Export complete!",
    error: videoError || "Export failed",
  };

  return (
    <div style={{ maxWidth: 680, margin: "0 auto", display: "flex", flexDirection: "column", gap: 14 }}>
      {/* ── Header + Output Folder + Filename ── */}
      <div>
        <h2 style={{ fontSize: 20, fontWeight: 700, color: C.text, marginBottom: 12 }}>Export</h2>

        {/* Folder row */}
        <div style={{
          display: "flex", alignItems: "center", gap: 8, padding: "10px 14px",
          borderRadius: 8, background: C.bg, border: `1px solid ${C.border}`, marginBottom: 8,
        }}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={C.muted} strokeWidth="2" strokeLinecap="round">
            <path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z" />
          </svg>
          <span style={{ flex: 1, fontSize: 12, color: C.dim, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap", fontFamily: "monospace" }}>
            {exp.outputDirectory || "..."}
          </span>
          <button onClick={changePath} style={{
            background: "none", border: "none", color: C.accent, fontSize: 12,
            cursor: "pointer", fontFamily: "inherit", fontWeight: 600,
          }}>Change</button>
          {(videoPhase === "done" || audioPhase === "done" || scriptResults.length > 0) && (
            <button onClick={() => openFolder(exp.outputDirectory!)} style={{
              background: "none", border: "none", color: C.success, fontSize: 12,
              cursor: "pointer", fontFamily: "inherit", fontWeight: 600,
            }}>Open</button>
          )}
        </div>

        {/* Filename row */}
        <div style={{
          display: "flex", alignItems: "center", gap: 8, padding: "8px 14px",
          borderRadius: 8, background: C.bg, border: `1px solid ${C.border}`,
        }}>
          <span style={{ fontSize: 11, color: C.muted, fontWeight: 600, whiteSpace: "nowrap" }}>Filename</span>
          <input
            type="text"
            value={exp.basename}
            onChange={(e) => exp.setBasename(slugify(e.target.value) || "untitled")}
            style={{
              flex: 1, padding: "4px 8px", border: `1px solid ${C.border}`, borderRadius: 6,
              fontSize: 13, background: "rgba(255,255,255,0.04)", color: C.text,
              fontFamily: "monospace", outline: "none",
            }}
          />
        </div>
      </div>

      {/* ══════════════════════════════════════════ */}
      {/* ── 1. FINAL VIDEO (primary) ──           */}
      {/* ══════════════════════════════════════════ */}
      <Section
        title="VIDEO"
        icon={<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="2" strokeLinecap="round"><rect x="2" y="2" width="20" height="20" rx="2.18" /><polygon points="10 8 16 12 10 16 10 8" /></svg>}
      >
        {!hasTtsKey ? (
          <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 14px", borderRadius: 8, background: "rgba(251,146,60,0.06)", border: "1px solid rgba(251,146,60,0.15)" }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#fb923c" strokeWidth="2" strokeLinecap="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
            <div style={{ flex: 1 }}>
              <p style={{ fontSize: 13, color: "#fb923c", margin: 0 }}>Add an ElevenLabs or Azure TTS API key to enable video export.</p>
              <p style={{ fontSize: 11, color: C.muted, margin: "4px 0 0" }}>Or use the free Built-in voice (configure in Settings &gt; Voice).</p>
            </div>
            <button onClick={() => openSettings("voice")} style={{ fontSize: 12, color: C.accent, background: "rgba(99,102,241,0.1)", border: "1px solid rgba(99,102,241,0.25)", borderRadius: 6, padding: "5px 12px", cursor: "pointer", fontFamily: "inherit", fontWeight: 500, whiteSpace: "nowrap" }}>Go to Settings</button>
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
            {/* Audio mode */}
            <div style={{ display: "flex", gap: 20 }}>
              <Radio checked={exp.replaceAudio} onClick={() => exp.setReplaceAudio(true)} label="Narration only" />
              <Radio checked={!exp.replaceAudio} onClick={() => exp.setReplaceAudio(false)} label="Mix with original" />
            </div>

            {/* Ducking strength (only relevant when mixing with the original) */}
            {!exp.replaceAudio && (
              <div style={{ display: "flex", alignItems: "center", gap: 10, paddingLeft: 2 }}>
                <span style={{ fontSize: 11, color: C.muted, fontWeight: 600, minWidth: 92 }}>Duck original</span>
                <input
                  type="range" min={-20} max={0} step={1} value={exp.duckDb}
                  onChange={(e) => exp.setDuckDb(Number(e.target.value))}
                  style={{ flex: 1, accentColor: C.accent }}
                />
                <span style={{ fontSize: 11, color: C.dim, minWidth: 42, textAlign: "right" }}>{exp.duckDb} dB</span>
              </div>
            )}

            {/* Subtitles */}
            <Toggle checked={exp.burnSubtitles} onChange={exp.setBurnSubtitles} label="Burn subtitles into video" />

            {/* Subtitle style options (visible when burn is on) */}
            {exp.burnSubtitles && (
              <div style={{
                display: "flex", flexDirection: "column", gap: 10, padding: "12px 14px",
                borderRadius: 8, background: "rgba(255,255,255,0.03)", border: `1px solid ${C.border}`,
              }}>
                {/* Preset selector */}
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <span style={{ fontSize: 11, color: C.muted, fontWeight: 600, minWidth: 64 }}>Preset</span>
                  <div style={{ display: "flex", gap: 4, flexWrap: "wrap" }}>
                    {(["shorts", "documentary", "clean", "custom"] as const).map((preset) => (
                      <button
                        key={preset}
                        onClick={() => exp.setSubtitlePreset(preset as SubtitlePreset)}
                        title={preset === "custom" ? "Edit every field by hand" : PRESET_DESCRIPTIONS[preset]}
                        style={{
                          padding: "3px 12px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: 600,
                          border: exp.subtitlePreset === preset
                            ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                          background: exp.subtitlePreset === preset ? C.accentDim : "transparent",
                          color: exp.subtitlePreset === preset ? C.accent : C.muted,
                          cursor: "pointer",
                        }}
                      >{PRESET_LABELS[preset]}</button>
                    ))}
                  </div>
                </div>
                {exp.subtitlePreset !== "custom" && (
                  <div style={{ fontSize: 11, color: C.dim, paddingLeft: 74, marginTop: -4 }}>
                    {PRESET_DESCRIPTIONS[exp.subtitlePreset]}
                  </div>
                )}
                {exp.subtitlePreset === "custom" && (<>
                {/* Font size slider */}
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <span style={{ fontSize: 11, color: C.muted, fontWeight: 600, minWidth: 64 }}>Font size</span>
                  <input
                    type="range" min={12} max={48} value={exp.subtitleFontSize}
                    onChange={(e) => exp.setSubtitleFontSize(Number(e.target.value))}
                    style={{ flex: 1, accentColor: C.accent }}
                  />
                  <span style={{ fontSize: 11, color: C.dim, minWidth: 24, textAlign: "right" }}>{exp.subtitleFontSize}</span>
                </div>

                {/* Text color presets */}
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <span style={{ fontSize: 11, color: C.muted, fontWeight: 600, minWidth: 64 }}>Color</span>
                  <div style={{ display: "flex", gap: 4 }}>
                    {([
                      { value: "#ffffff", label: "White" },
                      { value: "#ffff00", label: "Yellow" },
                      { value: "#00ffff", label: "Cyan" },
                      { value: "#00ff00", label: "Green" },
                    ] as const).map((c) => (
                      <button
                        key={c.value}
                        onClick={() => exp.setSubtitleColor(c.value)}
                        title={c.label}
                        style={{
                          width: 22, height: 22, borderRadius: 6, border: exp.subtitleColor === c.value
                            ? `2px solid ${C.accent}` : `1px solid ${C.border}`,
                          background: c.value, cursor: "pointer", padding: 0,
                        }}
                      />
                    ))}
                  </div>
                </div>

                {/* Outline color presets */}
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <span style={{ fontSize: 11, color: C.muted, fontWeight: 600, minWidth: 64 }}>Outline</span>
                  <div style={{ display: "flex", gap: 4 }}>
                    {([
                      { value: "#000000", label: "Black" },
                      { value: "#333333", label: "Dark gray" },
                      { value: "#00000000", label: "None" },
                    ] as const).map((c) => (
                      <button
                        key={c.value}
                        onClick={() => exp.setSubtitleOutlineColor(c.value)}
                        title={c.label}
                        style={{
                          width: 22, height: 22, borderRadius: 6, border: exp.subtitleOutlineColor === c.value
                            ? `2px solid ${C.accent}` : `1px solid ${C.border}`,
                          background: c.value === "#00000000"
                            ? "repeating-conic-gradient(#555 0% 25%, #888 0% 50%) 50% / 8px 8px"
                            : c.value,
                          cursor: "pointer", padding: 0,
                        }}
                      />
                    ))}
                  </div>
                  {/* Outline width slider */}
                  <input
                    type="range" min={0} max={5} value={exp.subtitleOutline}
                    onChange={(e) => exp.setSubtitleOutline(Number(e.target.value))}
                    style={{ flex: 1, accentColor: C.accent }}
                  />
                  <span style={{ fontSize: 11, color: C.dim, minWidth: 16, textAlign: "right" }}>{exp.subtitleOutline}</span>
                </div>

                {/* Position toggle */}
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <span style={{ fontSize: 11, color: C.muted, fontWeight: 600, minWidth: 64 }}>Position</span>
                  <div style={{ display: "flex", gap: 4 }}>
                    {(["bottom", "top"] as const).map((pos) => (
                      <button
                        key={pos}
                        onClick={() => exp.setSubtitlePosition(pos)}
                        style={{
                          padding: "3px 12px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: 600,
                          border: exp.subtitlePosition === pos
                            ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                          background: exp.subtitlePosition === pos ? C.accentDim : "transparent",
                          color: exp.subtitlePosition === pos ? C.accent : C.muted,
                          cursor: "pointer",
                        }}
                      >{pos.charAt(0).toUpperCase() + pos.slice(1)}</button>
                    ))}
                  </div>
                </div>

                {/* Text transform toggle */}
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <span style={{ fontSize: 11, color: C.muted, fontWeight: 600, minWidth: 64 }}>Case</span>
                  <div style={{ display: "flex", gap: 4 }}>
                    {([
                      { value: null, label: "As written" },
                      { value: "uppercase" as const, label: "UPPERCASE" },
                    ] as const).map((opt) => {
                      const active = exp.subtitleTextTransform === opt.value;
                      return (
                        <button
                          key={opt.label}
                          onClick={() => exp.setSubtitleTextTransform(opt.value)}
                          style={{
                            padding: "3px 12px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: 600,
                            border: active
                              ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                            background: active ? C.accentDim : "transparent",
                            color: active ? C.accent : C.muted,
                            cursor: "pointer",
                          }}
                        >{opt.label}</button>
                      );
                    })}
                  </div>
                </div>

                {/* Max words per line */}
                <div style={{ display: "flex", alignItems: "center", gap: 10 }}>
                  <span style={{ fontSize: 11, color: C.muted, fontWeight: 600, minWidth: 64 }}>Words/line</span>
                  <div style={{ display: "flex", gap: 4 }}>
                    {([
                      { value: null, label: "Auto" },
                      { value: 2, label: "2" },
                      { value: 3, label: "3" },
                      { value: 5, label: "5" },
                    ] as const).map((opt) => {
                      const active = exp.subtitleMaxWordsPerLine === opt.value;
                      return (
                        <button
                          key={opt.label}
                          onClick={() => exp.setSubtitleMaxWordsPerLine(opt.value)}
                          style={{
                            padding: "3px 12px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: 600,
                            border: active
                              ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                            background: active ? C.accentDim : "transparent",
                            color: active ? C.accent : C.muted,
                            cursor: "pointer",
                          }}
                        >{opt.label}</button>
                      );
                    })}
                  </div>
                </div>
                </>)}
              </div>
            )}

            {/* Voice summary */}
            <div style={{
              display: "flex", alignItems: "center", gap: 10, padding: "10px 14px",
              borderRadius: 8, background: "rgba(255,255,255,0.03)", border: `1px solid ${C.border}`,
            }}>
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={C.dim} strokeWidth="2" strokeLinecap="round">
                <path d="M12 1a3 3 0 00-3 3v8a3 3 0 006 0V4a3 3 0 00-3-3z"/>
                <path d="M19 10v2a7 7 0 01-14 0v-2"/>
                <line x1="12" y1="19" x2="12" y2="23"/>
                <line x1="8" y1="23" x2="16" y2="23"/>
              </svg>
              <span style={{ flex: 1, fontSize: 13, color: C.dim }}>
                {ttsProvider === "builtin"
                  ? "Built-in (Free) · System Default"
                  : ttsProvider === "elevenlabs"
                  ? `ElevenLabs${elVoices.find(v => v.voice_id === elConfig?.voice_id)?.name ? ` · ${elVoices.find(v => v.voice_id === elConfig?.voice_id)?.name}` : ""}${elConfig?.model_id ? ` · ${elConfig.model_id.replace("eleven_", "").replace(/_/g, " ")}` : ""}`
                  : `Azure TTS${azureConfig?.voice_name ? ` · ${azureConfig.voice_name}` : ""}`
                }
              </span>
              <button onClick={() => openSettings("voice")} style={{
                background: "none", border: "none", color: C.accent, fontSize: 12,
                cursor: "pointer", fontFamily: "inherit", fontWeight: 600,
              }}>Change</button>
            </div>

            {/* Output preview */}
            <FilePreview name={`${exp.basename}.mp4`} />

            {/* Progress */}
            {videoPhase !== "idle" && videoPhase !== "done" && videoPhase !== "error" && (
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <ProgressBar value={videoProgress} height={4} />
                <span style={{ fontSize: 11, color: C.dim }}>{phaseLabels[videoPhase]}</span>
              </div>
            )}

            {videoPhase === "done" && (
              <div style={{ fontSize: 12, color: C.success, fontWeight: 600, display: "flex", alignItems: "center", gap: 8 }}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><polyline points="20 6 9 17 4 12" /></svg>
                Video exported
                <button onClick={() => openFolder(exp.outputDirectory!)} style={{ background: "none", border: "none", color: C.success, fontSize: 11, cursor: "pointer", fontFamily: "inherit", textDecoration: "underline" }}>Open folder</button>
              </div>
            )}
            {videoPhase === "done" && videoNotice && (
              <div style={{ fontSize: 12, color: "#fbbf24", fontWeight: 500 }}>{videoNotice}</div>
            )}
            {videoPhase === "error" && (
              <div style={{ fontSize: 12, color: C.error, fontWeight: 500 }}>{videoError}</div>
            )}

            {/* Export button */}
            <Button
              onClick={doExportVideo}
              disabled={videoPhase === "rendering" || videoPhase === "audio" || videoPhase === "merge" || videoPhase === "subtitles" || !videoPath || segCount === 0}
              style={{ width: "100%", fontSize: 13 }}
            >
              {videoPhase === "rendering" || videoPhase === "audio" || videoPhase === "merge" || videoPhase === "subtitles"
                ? `${Math.round(videoProgress)}% — ${phaseLabels[videoPhase]}`
                : "Export Video"}
            </Button>
          </div>
        )}
      </Section>

      {/* ══════════════════════════════════════════ */}
      {/* ── 2. AUDIO ONLY ──                      */}
      {/* ══════════════════════════════════════════ */}
      <Section
        title="AUDIO ONLY"
        icon={<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="2" strokeLinecap="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5" /><path d="M19.07 4.93a10 10 0 010 14.14" /><path d="M15.54 8.46a5 5 0 010 7.07" /></svg>}
      >
        {!hasTtsKey ? (
          <div style={{ display: "flex", alignItems: "center", gap: 12, padding: "12px 14px", borderRadius: 8, background: "rgba(251,146,60,0.06)", border: "1px solid rgba(251,146,60,0.15)" }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke="#fb923c" strokeWidth="2" strokeLinecap="round"><circle cx="12" cy="12" r="10"/><line x1="12" y1="8" x2="12" y2="12"/><line x1="12" y1="16" x2="12.01" y2="16"/></svg>
            <p style={{ fontSize: 13, color: "#fb923c", margin: 0, flex: 1 }}>Add a TTS API key in Settings to generate audio.</p>
            <button onClick={() => openSettings("voice")} style={{ fontSize: 12, color: C.accent, background: "rgba(99,102,241,0.1)", border: "1px solid rgba(99,102,241,0.25)", borderRadius: 6, padding: "5px 12px", cursor: "pointer", fontFamily: "inherit", fontWeight: 500, whiteSpace: "nowrap" }}>Go to Settings</button>
          </div>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
            <p style={{ fontSize: 12, color: C.dim, margin: 0, lineHeight: 1.5 }}>
              Generate narration audio without video. Uses voice settings from Settings.
            </p>

            <FilePreview name={`${exp.basename}-audio.mp3`} />

            {audioPhase === "generating" && (
              <div style={{ display: "flex", flexDirection: "column", gap: 6 }}>
                <ProgressBar value={audioProgress} height={4} />
                <span style={{ fontSize: 11, color: C.dim }}>Generating audio...</span>
              </div>
            )}
            {audioPhase === "done" && (
              <div style={{ fontSize: 12, color: C.success, fontWeight: 600, display: "flex", alignItems: "center", gap: 8 }}>
                <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round"><polyline points="20 6 9 17 4 12" /></svg>
                Audio exported
                <button onClick={() => openFolder(exp.outputDirectory!)} style={{ background: "none", border: "none", color: C.success, fontSize: 11, cursor: "pointer", fontFamily: "inherit", textDecoration: "underline" }}>Open folder</button>
              </div>
            )}
            {audioPhase === "error" && (
              <div style={{ fontSize: 12, color: C.error, fontWeight: 500 }}>{audioError}</div>
            )}

            <Button
              variant="secondary"
              onClick={doExportAudio}
              disabled={audioPhase === "generating" || segCount === 0}
              style={{ width: "100%", fontSize: 13 }}
            >
              {audioPhase === "generating" ? `${Math.round(audioProgress)}%...` : "Export Audio"}
            </Button>
          </div>
        )}
      </Section>

      {/* ══════════════════════════════════════════ */}
      {/* ── 3. SCRIPTS (collapsible) ──           */}
      {/* ══════════════════════════════════════════ */}
      <Section
        title="SCRIPTS"
        icon={<svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="2" strokeLinecap="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z" /><path d="M14 2v6h6" /></svg>}
        collapsible
        defaultOpen={false}
      >
        <div style={{ display: "flex", flexDirection: "column", gap: 10 }}>
          {/* Format toggles */}
          <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
            {EXPORT_FORMATS.map((f) => {
              const sel = exp.selectedFormats.includes(f.id);
              return (
                <button key={f.id} onClick={() => exp.toggleFormat(f.id)} style={{
                  padding: "4px 10px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: sel ? 600 : 400,
                  border: sel ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                  background: sel ? C.accentDim : "transparent", color: sel ? C.accent : C.muted, cursor: "pointer",
                  transition: "all 0.15s",
                }}>{f.label.split(" ")[0]}</button>
              );
            })}
          </div>

          {/* Language toggles */}
          <div style={{ display: "flex", gap: 4 }}>
            {languages.map((l) => {
              const on = exp.languageToggles[l] ?? true;
              const has = !!scripts[l];
              return (
                <button key={l} onClick={() => exp.toggleLanguageExport(l)} disabled={!has} style={{
                  padding: "3px 10px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: 600,
                  border: on && has ? "1px solid rgba(99,102,241,0.3)" : `1px solid ${C.border}`,
                  background: on && has ? "rgba(99,102,241,0.06)" : "transparent",
                  color: has ? (on ? C.accent : C.muted) : "#2a2a3a",
                  cursor: has ? "pointer" : "default",
                }}>{l.toUpperCase()}</button>
              );
            })}
          </div>

          {/* Results */}
          {scriptResults.length > 0 && (
            <div style={{ fontSize: 12, color: scriptSuccessCount === scriptResults.length ? C.success : C.error, fontWeight: 600 }}>
              {scriptSuccessCount}/{scriptResults.length} files exported
            </div>
          )}

          {/* Actions */}
          <div style={{ display: "flex", gap: 8 }}>
            <Button variant="ghost" size="sm" onClick={copyScript} disabled={!primaryScript} style={{ fontSize: 12 }}>
              {copied ? "Copied!" : "Copy"}
            </Button>
            <Button variant="secondary" size="sm" onClick={doExportScripts}
              disabled={!exp.selectedFormats.length || scriptExporting || !primaryScript}
              style={{ flex: 1, fontSize: 12 }}
            >
              {scriptExporting ? "Exporting..." : "Export Scripts"}
            </Button>
          </div>
        </div>
      </Section>
    </div>
  );
}

