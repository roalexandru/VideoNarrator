import { useState, useEffect } from "react";
import { Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useExportStore } from "../../stores/exportStore";
import { useScriptStore } from "../../stores/scriptStore";
import { useConfigStore } from "../../stores/configStore";
import { useProjectStore } from "../../stores/projectStore";
import { exportScript, getElevenLabsConfig, saveElevenLabsConfig, listElevenLabsVoices, generateTts, mergeAudioVideo, openFolder, getHomeDir,
  type TtsResult, type ElevenLabsConfig, type ElevenLabsVoice } from "../../lib/tauri/commands";
import { EXPORT_FORMATS } from "../../lib/constants";
import { Button } from "../../components/ui/Button";
import { ProgressBar } from "../../components/ui/ProgressBar";
import type { ExportResult } from "../../types/export";
import type { ProgressEvent } from "../../types/processing";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };
const sLabel = { fontSize: 11, fontWeight: 700 as const, color: C.muted, textTransform: "uppercase" as const, letterSpacing: 1.2, marginBottom: 8 };
const sliderLabel = { display: "block" as const, fontSize: 11, fontWeight: 600 as const, color: C.dim, marginBottom: 4 };
const selectStyle = { width: "100%", padding: "7px 10px", border: `1px solid ${C.border}`, borderRadius: 6, fontSize: 12, background: "rgba(255,255,255,0.04)", color: C.text, fontFamily: "inherit", cursor: "pointer", appearance: "none" as const };

const FORMAT_TOOLTIPS: Record<string, string> = {
  json: "Structured data",
  srt: "Subtitle file",
  vtt: "Web subtitles",
  txt: "Plain text",
  md: "Markdown",
  ssml: "Speech markup for TTS",
};

const ELEVEN_MODELS = [
  { id: "eleven_multilingual_v2", label: "Multilingual v2" },
  { id: "eleven_flash_v2_5", label: "Flash v2.5" },
  { id: "eleven_turbo_v2_5", label: "Turbo v2.5" },
  { id: "eleven_v3", label: "v3" },
];

export function ExportScreen() {
  const exp = useExportStore();
  const scripts = useScriptStore((s) => s.scripts);
  const languages = useConfigStore((s) => s.languages);
  const projectTitle = useProjectStore((s) => s.title);
  const videoPath = useProjectStore((s) => s.videoFile?.path);

  const [results, setResults] = useState<ExportResult[]>([]);
  const [exporting, setExporting] = useState(false);
  const [copied, setCopied] = useState(false);

  // TTS
  const [elConfig, setElConfig] = useState<ElevenLabsConfig | null>(null);
  const [elVoices, setElVoices] = useState<ElevenLabsVoice[]>([]);
  const [ttsResults, setTtsResults] = useState<TtsResult[]>([]);
  const [ttsRunning, setTtsRunning] = useState(false);
  const [ttsProgress, setTtsProgress] = useState(0);
  const [ttsCompact, setTtsCompact] = useState(true);
  const [customVoiceId, setCustomVoiceId] = useState("");

  // Video merge
  const [merging, setMerging] = useState(false);
  const [mergeResult, setMergeResult] = useState<string | null>(null);
  const [replaceAudio, setReplaceAudio] = useState(true);

  // Smart default output path
  useEffect(() => {
    if (!exp.outputDirectory) {
      const safe = (projectTitle || "Untitled").replace(/[^a-zA-Z0-9_\- ]/g, "").trim() || "Untitled";
      getHomeDir().then((home) => {
        exp.setOutputDirectory(`${home}/Documents/Narrator/${safe}`);
      }).catch(() => {
        exp.setOutputDirectory(`/tmp/Narrator_Export/${safe}`);
      });
    }
  }, [projectTitle]);

  useEffect(() => {
    getElevenLabsConfig().then((cfg) => {
      if (cfg?.api_key) {
        setElConfig(cfg);
        listElevenLabsVoices(cfg.api_key).then(setElVoices).catch(() => {});
      }
    }).catch(() => {});
  }, []);

  const changePath = async () => {
    const d = await open({ directory: true });
    if (d && typeof d === "string") exp.setOutputDirectory(d);
  };

  const doExport = async () => {
    if (!exp.outputDirectory || !exp.selectedFormats.length) return;
    setExporting(true);
    try {
      const langs = Object.entries(exp.languageToggles).filter(([, on]) => on).map(([l]) => l);
      setResults(await exportScript({ formats: exp.selectedFormats, languages: langs, output_directory: exp.outputDirectory, scripts }));
    } catch (e) { console.error(e); }
    finally { setExporting(false); }
  };

  const doTts = async () => {
    if (!exp.outputDirectory || !elConfig) return;
    await saveElevenLabsConfig(elConfig);
    setTtsRunning(true); setTtsProgress(0); setTtsResults([]);
    try {
      const lang = Object.entries(exp.languageToggles).find(([, on]) => on)?.[0];
      const script = lang ? scripts[lang] : Object.values(scripts)[0];
      if (!script) return;
      const ch = new Channel<ProgressEvent>();
      ch.onmessage = (e: ProgressEvent) => { if (e.kind === "progress") setTtsProgress(e.percent); };
      setTtsResults(await generateTts(script.segments, exp.outputDirectory + "/audio", ttsCompact, ch));
    } catch (e: any) { console.error("TTS:", e); }
    finally { setTtsRunning(false); }
  };

  const doMerge = async () => {
    if (!videoPath || !exp.outputDirectory) return;
    const audioFile = exp.outputDirectory + "/audio/narration_full.mp3";
    const outFile = exp.outputDirectory + "/final_video.mp4";
    setMerging(true); setMergeResult(null);
    try {
      const result = await mergeAudioVideo(videoPath, audioFile, outFile, replaceAudio);
      setMergeResult(result);
    } catch (e: any) { console.error("Merge:", e); setMergeResult("error"); }
    finally { setMerging(false); }
  };

  const copy = () => {
    const l = Object.keys(scripts)[0]; const s = scripts[l]; if (!s) return;
    navigator.clipboard.writeText(s.segments.map((seg) => `[${seg.start_seconds.toFixed(1)}s - ${seg.end_seconds.toFixed(1)}s]\n${seg.text}`).join("\n\n"));
    setCopied(true); setTimeout(() => setCopied(false), 2000);
  };

  const successCount = results.filter((r) => r.success).length;
  const ttsOk = ttsResults.filter((r) => r.success).length;
  const segCount = Object.values(scripts)[0]?.segments.length || 0;
  const hasEL = !!elConfig?.api_key;
  const hasTtsAudio = ttsOk > 0;

  return (
    <div style={{ maxWidth: 760, margin: "0 auto" }}>
      <div style={{ marginBottom: 20 }}>
        <h2 style={{ fontSize: 22, fontWeight: 700, color: C.text }}>Export</h2>
      </div>

      {/* Output path — editable */}
      <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 20, padding: "10px 14px", borderRadius: 8, background: "rgba(255,255,255,0.02)", border: `1px solid ${C.border}` }}>
        <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke={C.muted} strokeWidth="2" strokeLinecap="round"><path d="M22 19a2 2 0 01-2 2H4a2 2 0 01-2-2V5a2 2 0 012-2h5l2 3h9a2 2 0 012 2z"/></svg>
        <span style={{ flex: 1, fontSize: 13, color: C.dim, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
          {exp.outputDirectory}
        </span>
        <button onClick={changePath} style={{ background: "none", border: "none", color: C.accent, fontSize: 12, cursor: "pointer", fontFamily: "inherit", fontWeight: 600 }}>Change</button>
        {(results.length > 0 || mergeResult) && (
          <button onClick={() => openFolder(exp.outputDirectory!)} style={{ background: "none", border: "none", color: "#4ade80", fontSize: 12, cursor: "pointer", fontFamily: "inherit", fontWeight: 600 }}>
            Open Folder
          </button>
        )}
      </div>

      {/* Three cards in a row */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr 1fr", gap: 12, marginBottom: 16 }}>
        {/* 1: FILE EXPORT */}
        <div style={{ padding: "16px", borderRadius: 10, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)", display: "flex", flexDirection: "column" }}>
          <div style={{ ...sLabel, display: "flex", alignItems: "center", gap: 6 }}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="2" strokeLinecap="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><path d="M14 2v6h6"/></svg>
            1. Scripts
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 4, marginBottom: 10 }}>
            {EXPORT_FORMATS.map((f) => {
              const sel = exp.selectedFormats.includes(f.id);
              return (
                <button key={f.id} onClick={() => exp.toggleFormat(f.id)} title={FORMAT_TOOLTIPS[f.id]} style={{
                  padding: "3px 8px", borderRadius: 5, fontSize: 10, fontFamily: "inherit", fontWeight: sel ? 600 : 400,
                  border: sel ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                  background: sel ? "rgba(99,102,241,0.1)" : "transparent", color: sel ? C.accent : C.muted, cursor: "pointer",
                }}>{f.label}</button>
              );
            })}
          </div>
          <div style={{ display: "flex", gap: 3, marginBottom: 10 }}>
            {languages.map((l) => {
              const on = exp.languageToggles[l] ?? true; const has = !!scripts[l];
              return (
                <button key={l} onClick={() => exp.toggleLanguageExport(l)} disabled={!has} style={{
                  padding: "2px 8px", borderRadius: 5, fontSize: 10, fontFamily: "inherit", fontWeight: 600,
                  border: on && has ? "1px solid rgba(99,102,241,0.3)" : `1px solid ${C.border}`,
                  background: on && has ? "rgba(99,102,241,0.06)" : "transparent", color: has ? (on ? C.accent : C.muted) : "#2a2a3a", cursor: has ? "pointer" : "default",
                }}>{l.toUpperCase()}</button>
              );
            })}
          </div>
          <div style={{ marginTop: "auto", display: "flex", gap: 6 }}>
            <Button variant="secondary" size="sm" onClick={copy} style={{ flex: 1, fontSize: 11 }}>{copied ? "Copied!" : "Copy"}</Button>
            <Button size="sm" onClick={doExport} disabled={!exp.selectedFormats.length || exporting} style={{ flex: 1, fontSize: 11 }}>
              {exporting ? "..." : "Export"}
            </Button>
          </div>
          {results.length > 0 && (
            <div style={{ marginTop: 8, fontSize: 11, color: successCount === results.length ? "#4ade80" : "#f87171", fontWeight: 600 }}>
              {successCount}/{results.length} exported
            </div>
          )}
        </div>

        {/* 2: AUDIO / TTS */}
        <div style={{ padding: "16px", borderRadius: 10, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)", display: "flex", flexDirection: "column" }}>
          <div style={{ ...sLabel, display: "flex", alignItems: "center", gap: 6 }}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="2" strokeLinecap="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M19.07 4.93a10 10 0 010 14.14"/></svg>
            2. Audio
          </div>
          {!hasEL ? (
            <p style={{ fontSize: 11, color: C.muted, flex: 1 }}>Add ElevenLabs key in Settings.</p>
          ) : (
            <>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 6, marginBottom: 8 }}>
                <div>
                  <label style={sliderLabel}>Voice</label>
                  <select value={elConfig.voice_id} onChange={(e) => { if (e.target.value !== "__custom__") setElConfig({ ...elConfig, voice_id: e.target.value }); }} style={{ ...selectStyle, fontSize: 11 }}>
                    {elVoices.map((v) => <option key={v.voice_id} value={v.voice_id} style={{ background: "#1a1a24" }}>{v.name}</option>)}
                  </select>
                  <div style={{ display: "flex", gap: 3, marginTop: 4 }}>
                    <input type="text" value={customVoiceId} onChange={(e) => setCustomVoiceId(e.target.value)} placeholder="Voice ID" style={{ flex: 1, padding: "3px 6px", border: `1px solid ${C.border}`, borderRadius: 4, fontSize: 10, background: "rgba(255,255,255,0.04)", color: C.text, fontFamily: "monospace", outline: "none" }} />
                    <button onClick={() => { if (customVoiceId.trim()) { setElConfig({ ...elConfig, voice_id: customVoiceId.trim() }); if (!elVoices.find((v) => v.voice_id === customVoiceId.trim())) setElVoices([...elVoices, { voice_id: customVoiceId.trim(), name: "Custom", category: "custom" }]); setCustomVoiceId(""); } }}
                      style={{ padding: "3px 6px", borderRadius: 4, border: "none", background: "rgba(99,102,241,0.15)", color: C.accent, fontSize: 10, cursor: "pointer", fontFamily: "inherit" }}>Use</button>
                  </div>
                </div>
                <div>
                  <label style={sliderLabel}>Model</label>
                  <select value={elConfig.model_id} onChange={(e) => setElConfig({ ...elConfig, model_id: e.target.value })} style={{ ...selectStyle, fontSize: 11 }}>
                    {ELEVEN_MODELS.map((m) => <option key={m.id} value={m.id} style={{ background: "#1a1a24" }}>{m.label}</option>)}
                  </select>
                </div>
              </div>
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 4, marginBottom: 8 }}>
                {([["stability", "Stability", elConfig.stability, 0, 1, 0.05] as const, ["similarity_boost", "Clarity", elConfig.similarity_boost, 0, 1, 0.05] as const]).map(([key, lbl, val, min, max, step]) => (
                  <div key={key}>
                    <label style={{ ...sliderLabel, fontSize: 10 }}>{lbl} ({val.toFixed(1)})</label>
                    <input type="range" min={min} max={max} step={step} value={val} onChange={(e) => setElConfig({ ...elConfig, [key]: parseFloat(e.target.value) })} style={{ width: "100%", accentColor: "#6366f1", height: 3 }} />
                  </div>
                ))}
              </div>
              <div style={{ display: "flex", gap: 4, marginBottom: 8 }}>
                <button onClick={() => setTtsCompact(true)} style={{ flex: 1, padding: "4px", borderRadius: 5, fontSize: 10, fontFamily: "inherit", fontWeight: 600, border: ttsCompact ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`, background: ttsCompact ? "rgba(99,102,241,0.08)" : "transparent", color: ttsCompact ? C.accent : C.muted, cursor: "pointer" }}>Single file</button>
                <button onClick={() => setTtsCompact(false)} style={{ flex: 1, padding: "4px", borderRadius: 5, fontSize: 10, fontFamily: "inherit", fontWeight: 600, border: !ttsCompact ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`, background: !ttsCompact ? "rgba(99,102,241,0.08)" : "transparent", color: !ttsCompact ? C.accent : C.muted, cursor: "pointer" }}>{segCount} segs</button>
              </div>
              {ttsRunning && <ProgressBar value={ttsProgress} height={3} />}
              <Button onClick={doTts} disabled={ttsRunning} size="sm" style={{ width: "100%", marginTop: "auto", fontSize: 11 }}>
                {ttsRunning ? `${Math.round(ttsProgress)}%...` : "Generate Audio"}
              </Button>
              {ttsOk > 0 && <div style={{ marginTop: 6, fontSize: 11, color: "#4ade80", fontWeight: 600 }}>{ttsOk}/{ttsResults.length} generated</div>}
            </>
          )}
        </div>

        {/* 3: FINAL VIDEO (merge audio+video) */}
        <div style={{ padding: "16px", borderRadius: 10, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)", display: "flex", flexDirection: "column" }}>
          <div style={{ ...sLabel, display: "flex", alignItems: "center", gap: 6 }}>
            <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="2" strokeLinecap="round"><rect x="2" y="2" width="20" height="20" rx="2.18"/><line x1="7" y1="2" x2="7" y2="22"/><line x1="17" y1="2" x2="17" y2="22"/></svg>
            3. Final Video
          </div>

          {!hasTtsAudio && !videoPath ? (
            <p style={{ fontSize: 11, color: C.muted, flex: 1 }}>Generate audio first (step 2), then merge with video.</p>
          ) : (
            <>
              <p style={{ fontSize: 11, color: C.dim, marginBottom: 10, lineHeight: 1.4 }}>
                Merge narration audio with your video to create a ready-to-share file.
              </p>

              <div style={{ display: "flex", flexDirection: "column", gap: 6, marginBottom: 12 }}>
                <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer", fontSize: 12, color: C.dim }}
                  onClick={() => setReplaceAudio(true)}>
                  <div style={{ width: 14, height: 14, borderRadius: "50%", border: `2px solid ${replaceAudio ? C.accent : C.muted}`, display: "flex", alignItems: "center", justifyContent: "center" }}>
                    {replaceAudio && <div style={{ width: 6, height: 6, borderRadius: "50%", background: C.accent }} />}
                  </div>
                  Replace audio
                </label>
                <label style={{ display: "flex", alignItems: "center", gap: 8, cursor: "pointer", fontSize: 12, color: C.dim }}
                  onClick={() => setReplaceAudio(false)}>
                  <div style={{ width: 14, height: 14, borderRadius: "50%", border: `2px solid ${!replaceAudio ? C.accent : C.muted}`, display: "flex", alignItems: "center", justifyContent: "center" }}>
                    {!replaceAudio && <div style={{ width: 6, height: 6, borderRadius: "50%", background: C.accent }} />}
                  </div>
                  Mix with original
                </label>
              </div>

              {!hasTtsAudio && (
                <p style={{ fontSize: 11, color: C.muted, marginBottom: 6, fontStyle: "italic" }}>Generate audio first (step 2)</p>
              )}
              <Button onClick={doMerge} disabled={merging || !hasTtsAudio} size="sm" style={{ width: "100%", marginTop: "auto", fontSize: 11 }}>
                {merging ? "Merging..." : "Create Final Video"}
              </Button>

              {mergeResult && mergeResult !== "error" && (
                <div style={{ marginTop: 6, fontSize: 11, color: "#4ade80", fontWeight: 600 }}>
                  Video ready!
                  <button onClick={() => openFolder(exp.outputDirectory!)} style={{ background: "none", border: "none", color: "#4ade80", fontSize: 11, cursor: "pointer", fontFamily: "inherit", marginLeft: 6, textDecoration: "underline" }}>
                    Open
                  </button>
                </div>
              )}
              {mergeResult === "error" && (
                <div style={{ marginTop: 6, fontSize: 11, color: "#f87171" }}>Merge failed. Generate audio first.</div>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}
