import { useState, useEffect, useCallback } from "react";
import { Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useExportStore, slugify } from "../../stores/exportStore";
import { useScriptStore } from "../../stores/scriptStore";
import { useConfigStore } from "../../stores/configStore";
import { useProjectStore } from "../../stores/projectStore";
import {
  exportScript, getElevenLabsConfig, saveElevenLabsConfig, listElevenLabsVoices,
  generateTts, mergeAudioVideo, burnSubtitles, openFolder, getHomeDir,
  getAzureTtsConfig, saveAzureTtsConfig,
  type ElevenLabsConfig, type ElevenLabsVoice, type AzureTtsConfig,
} from "../../lib/tauri/commands";
import { EXPORT_FORMATS } from "../../lib/constants";
import { useOpenSettings } from "../../contexts/SettingsContext";
import { trackEvent } from "../telemetry/analytics";
import { Button } from "../../components/ui/Button";
import { ProgressBar } from "../../components/ui/ProgressBar";
import type { ExportResult } from "../../types/export";
import type { ProgressEvent } from "../../types/processing";

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
  const videoPath = useProjectStore((s) => s.videoFile?.path);

  // Script export
  const [scriptResults, setScriptResults] = useState<ExportResult[]>([]);
  const [scriptExporting, setScriptExporting] = useState(false);
  const [copied, setCopied] = useState(false);

  // TTS / Voice
  const [elConfig, setElConfig] = useState<ElevenLabsConfig | null>(null);
  const [elVoices, setElVoices] = useState<ElevenLabsVoice[]>([]);
  const [azureConfig, setAzureConfig] = useState<AzureTtsConfig | null>(null);
  const ttsProvider = useConfigStore((s) => s.ttsProvider);
  const openSettings = useOpenSettings();

  // Video export pipeline
  const [videoPhase, setVideoPhase] = useState<"idle" | "audio" | "merge" | "subtitles" | "done" | "error">("idle");
  const [videoProgress, setVideoProgress] = useState(0);
  const [videoError, setVideoError] = useState<string | null>(null);
  const [, setVideoOutputPath] = useState<string | null>(null);

  // Audio-only export
  const [audioPhase, setAudioPhase] = useState<"idle" | "generating" | "done" | "error">("idle");
  const [audioProgress, setAudioProgress] = useState(0);
  const [, setAudioOutputPath] = useState<string | null>(null);
  const [audioError, setAudioError] = useState<string | null>(null);

  const hasTtsKey = ttsProvider === "elevenlabs" ? !!elConfig?.api_key : !!azureConfig?.api_key;
  const primaryScript = Object.values(scripts)[0];
  const segCount = primaryScript?.segments.length || 0;

  // Init output directory + basename from title
  useEffect(() => {
    if (projectTitle) {
      getHomeDir().then((home) => {
        exp.initFromTitle(projectTitle, home);
      }).catch(() => {
        exp.initFromTitle(projectTitle, "/tmp");
      });
    }
  }, [projectTitle]);

  // Load TTS config
  useEffect(() => {
    getElevenLabsConfig().then((cfg) => {
      if (cfg?.api_key) {
        setElConfig(cfg);
        listElevenLabsVoices(cfg.api_key).then(setElVoices).catch(() => {});
      }
    }).catch(() => {});
    getAzureTtsConfig().then(setAzureConfig).catch(() => {});
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

    if (ttsProvider === "elevenlabs" && elConfig) await saveElevenLabsConfig(elConfig);
    if (ttsProvider === "azure" && azureConfig) await saveAzureTtsConfig(azureConfig);

    const script = getScript();
    if (!script) return;

    const dir = exp.outputDirectory;
    const base = exp.basename;

    setVideoPhase("audio");
    setVideoProgress(0);
    setVideoError(null);
    setVideoOutputPath(null);

    try {
      // Ensure directory exists
      // Phase 1: Generate TTS audio
      const ch = new Channel<ProgressEvent>();
      ch.onmessage = (e: ProgressEvent) => {
        if (e.kind === "progress") setVideoProgress(e.percent * 0.6); // 0-60%
      };

      const audioDir = `${dir}/audio`;
      const ttsResults = await generateTts(script.segments, audioDir, true, ch, ttsProvider);
      const ttsOk = ttsResults.filter((r) => r.success);
      if (ttsOk.length === 0) throw new Error("Audio generation failed");

      const audioFile = ttsOk[0].file_path;

      // Phase 2: Merge audio with video
      setVideoPhase("merge");
      setVideoProgress(65);

      const finalPath = `${dir}/${base}.mp4`;

      if (exp.burnSubtitles) {
        // Merge to temp file first, then burn subtitles to final
        const mergedPath = `${dir}/${base}_merged.mp4`;
        await mergeAudioVideo(videoPath, audioFile, mergedPath, exp.replaceAudio);
        setVideoProgress(80);

        setVideoPhase("subtitles");
        setVideoProgress(85);
        const srtContent = generateSrt(script);
        await burnSubtitles(mergedPath, srtContent, finalPath);
      } else {
        // Merge directly to final path
        await mergeAudioVideo(videoPath, audioFile, finalPath, exp.replaceAudio);
      }

      setVideoProgress(100);
      setVideoOutputPath(finalPath);
      setVideoPhase("done");
      trackEvent("export_completed", { type: "video" });
    } catch (e: any) {
      console.error("Export video:", e);
      setVideoError(typeof e === "string" ? e : e?.message || "Export failed");
      setVideoPhase("error");
    }
  };

  // ── Export Audio Only ──
  const doExportAudio = async () => {
    if (!exp.outputDirectory) return;
    if (ttsProvider === "elevenlabs" && !elConfig) return;
    if (ttsProvider === "azure" && !azureConfig) return;

    if (ttsProvider === "elevenlabs" && elConfig) await saveElevenLabsConfig(elConfig);
    if (ttsProvider === "azure" && azureConfig) await saveAzureTtsConfig(azureConfig);

    const script = getScript();
    if (!script) return;

    setAudioPhase("generating");
    setAudioProgress(0);
    setAudioError(null);
    setAudioOutputPath(null);

    try {
      const ch = new Channel<ProgressEvent>();
      ch.onmessage = (e: ProgressEvent) => {
        if (e.kind === "progress") setAudioProgress(e.percent);
      };

      const audioDir = `${exp.outputDirectory}/audio`;
      const ttsResults = await generateTts(script.segments, audioDir, true, ch, ttsProvider);
      const ttsOk = ttsResults.filter((r) => r.success);
      if (ttsOk.length === 0) throw new Error("Audio generation failed");

      setAudioOutputPath(ttsOk[0].file_path);
      setAudioPhase("done");
      trackEvent("export_completed", { type: "audio" });
    } catch (e: any) {
      console.error("Export audio:", e);
      setAudioError(typeof e === "string" ? e : e?.message || "Audio export failed");
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
      trackEvent("export_completed", { type: "script", format_count: results.length });
    } catch (e) {
      console.error(e);
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
          <p style={{ fontSize: 13, color: C.muted, margin: 0 }}>
            Add a TTS API key in Settings to enable video export.
          </p>
        ) : (
          <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
            {/* Audio mode */}
            <div style={{ display: "flex", gap: 20 }}>
              <Radio checked={exp.replaceAudio} onClick={() => exp.setReplaceAudio(true)} label="Narration only" />
              <Radio checked={!exp.replaceAudio} onClick={() => exp.setReplaceAudio(false)} label="Mix with original" />
            </div>

            {/* Subtitles */}
            <Toggle checked={exp.burnSubtitles} onChange={exp.setBurnSubtitles} label="Burn subtitles into video" />

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
                {ttsProvider === "elevenlabs"
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
            {videoPhase === "error" && (
              <div style={{ fontSize: 12, color: C.error, fontWeight: 500 }}>{videoError}</div>
            )}

            {/* Export button */}
            <Button
              onClick={doExportVideo}
              disabled={videoPhase === "audio" || videoPhase === "merge" || videoPhase === "subtitles" || !videoPath || segCount === 0}
              style={{ width: "100%", fontSize: 13 }}
            >
              {videoPhase === "audio" || videoPhase === "merge" || videoPhase === "subtitles"
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
          <p style={{ fontSize: 13, color: C.muted, margin: 0 }}>
            Add a TTS API key in Settings to generate audio.
          </p>
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

// ── Helper: generate SRT content from script (client-side for subtitle burn) ──
function generateSrt(script: { segments: { index: number; start_seconds: number; end_seconds: number; text: string }[] }): string {
  return script.segments.map((seg, i) => {
    const fmtTime = (s: number) => {
      const h = Math.floor(s / 3600);
      const m = Math.floor((s % 3600) / 60);
      const sec = Math.floor(s % 60);
      const ms = Math.round((s % 1) * 1000);
      return `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}:${String(sec).padStart(2, "0")},${String(ms).padStart(3, "0")}`;
    };
    return `${i + 1}\n${fmtTime(seg.start_seconds)} --> ${fmtTime(seg.end_seconds)}\n${seg.text}\n`;
  }).join("\n");
}
