import { useState, useEffect } from "react";
import { Channel } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import { useExportStore } from "../../stores/exportStore";
import { useScriptStore } from "../../stores/scriptStore";
import { useConfigStore } from "../../stores/configStore";
import { exportScript, getElevenLabsConfig, saveElevenLabsConfig, listElevenLabsVoices, generateTts, type TtsResult, type ElevenLabsConfig, type ElevenLabsVoice } from "../../lib/tauri/commands";
import { EXPORT_FORMATS } from "../../lib/constants";
import { Button } from "../../components/ui/Button";
import { ProgressBar } from "../../components/ui/ProgressBar";
import type { ExportResult } from "../../types/export";
import type { ProgressEvent } from "../../types/processing";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };
const sLabel = { fontSize: 11, fontWeight: 700 as const, color: C.muted, textTransform: "uppercase" as const, letterSpacing: 1.2, marginBottom: 10 };
const sliderLabel = { display: "block" as const, fontSize: 12, fontWeight: 600 as const, color: C.dim, marginBottom: 5 };

const ELEVEN_MODELS = [
  { id: "eleven_multilingual_v2", label: "Multilingual v2 (Best quality)" },
  { id: "eleven_flash_v2_5", label: "Flash v2.5 (Ultra-low latency)" },
  { id: "eleven_turbo_v2_5", label: "Turbo v2.5 (Balanced)" },
  { id: "eleven_v3", label: "v3 (Dramatic delivery)" },
];

export function ExportScreen() {
  const exp = useExportStore();
  const scripts = useScriptStore((s) => s.scripts);
  const languages = useConfigStore((s) => s.languages);
  const [results, setResults] = useState<ExportResult[]>([]);
  const [exporting, setExporting] = useState(false);
  const [copied, setCopied] = useState(false);

  // TTS state
  const [elConfig, setElConfig] = useState<ElevenLabsConfig | null>(null);
  const [elVoices, setElVoices] = useState<ElevenLabsVoice[]>([]);
  const [customVoiceId, setCustomVoiceId] = useState("");
  const [ttsResults, setTtsResults] = useState<TtsResult[]>([]);
  const [ttsRunning, setTtsRunning] = useState(false);
  const [ttsProgress, setTtsProgress] = useState(0);
  const [ttsCompact, setTtsCompact] = useState(true);

  useEffect(() => {
    getElevenLabsConfig().then((cfg) => {
      if (cfg && cfg.api_key) {
        setElConfig(cfg);
        listElevenLabsVoices(cfg.api_key).then(setElVoices).catch(() => {});
      }
    }).catch(() => {});
  }, []);

  const pickDir = async () => { const d = await open({ directory: true }); if (d && typeof d === "string") exp.setOutputDirectory(d); };

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
    // Save voice settings before generating
    await saveElevenLabsConfig(elConfig);
    setTtsRunning(true); setTtsProgress(0); setTtsResults([]);
    try {
      const lang = Object.entries(exp.languageToggles).find(([, on]) => on)?.[0];
      const script = lang ? scripts[lang] : Object.values(scripts)[0];
      if (!script) return;
      const ch = new Channel<ProgressEvent>();
      ch.onmessage = (e: ProgressEvent) => { if (e.kind === "progress") setTtsProgress(e.percent); };
      setTtsResults(await generateTts(script.segments, exp.outputDirectory + "/audio", ttsCompact, ch));
    } catch (e: any) { console.error("TTS failed:", e); }
    finally { setTtsRunning(false); }
  };

  const copy = () => {
    const l = Object.keys(scripts)[0]; const s = scripts[l]; if (!s) return;
    navigator.clipboard.writeText(s.segments.map((seg) => `[${seg.start_seconds.toFixed(1)}s - ${seg.end_seconds.toFixed(1)}s]\n${seg.text}`).join("\n\n"));
    setCopied(true); setTimeout(() => setCopied(false), 2000);
  };

  const successCount = results.filter((r) => r.success).length;
  const ttsSuccessCount = ttsResults.filter((r) => r.success).length;
  const segCount = Object.values(scripts)[0]?.segments.length || 0;
  const hasElevenLabs = !!elConfig?.api_key;
  const selectStyle = { width: "100%", padding: "8px 12px", border: `1px solid ${C.border}`, borderRadius: 8, fontSize: 13, background: "rgba(255,255,255,0.04)", color: C.text, fontFamily: "inherit", cursor: "pointer", appearance: "none" as const };

  return (
    <div style={{ maxWidth: 720, margin: "0 auto" }}>
      <div style={{ marginBottom: 24 }}>
        <h2 style={{ fontSize: 24, fontWeight: 700, color: C.text, letterSpacing: -0.3 }}>Export</h2>
        <p style={{ color: C.dim, marginTop: 4, fontSize: 14 }}>Export scripts and generate audio.</p>
      </div>

      {/* Output Directory */}
      <section style={{ marginBottom: 20 }}>
        <div style={sLabel}>Output Directory</div>
        <div style={{ display: "flex", gap: 10, alignItems: "center" }}>
          <div style={{ flex: 1, padding: "10px 14px", background: "rgba(255,255,255,0.03)", border: `1px solid ${C.border}`, borderRadius: 8, fontSize: 13, color: exp.outputDirectory ? C.dim : C.muted, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
            {exp.outputDirectory || "Select a directory..."}
          </div>
          <Button variant="secondary" onClick={pickDir}>Browse</Button>
        </div>
      </section>

      {/* Two cards */}
      <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 16, marginBottom: 20 }}>
        {/* FILE EXPORT */}
        <div style={{ padding: "18px", borderRadius: 12, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)", display: "flex", flexDirection: "column" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 14 }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="2" strokeLinecap="round"><path d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"/><path d="M14 2v6h6"/></svg>
            <span style={{ fontWeight: 600, color: C.text, fontSize: 14 }}>File Export</span>
          </div>
          <div style={{ display: "flex", flexWrap: "wrap", gap: 5, marginBottom: 12 }}>
            {EXPORT_FORMATS.map((f) => {
              const sel = exp.selectedFormats.includes(f.id);
              return (
                <button key={f.id} onClick={() => exp.toggleFormat(f.id)} style={{
                  padding: "4px 10px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: sel ? 600 : 400,
                  border: sel ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                  background: sel ? "rgba(99,102,241,0.1)" : "rgba(255,255,255,0.02)",
                  color: sel ? C.accent : C.muted, cursor: "pointer",
                }}>{f.label}</button>
              );
            })}
          </div>
          <div style={{ display: "flex", gap: 4, marginBottom: 14 }}>
            {languages.map((l) => {
              const on = exp.languageToggles[l] ?? true; const has = !!scripts[l];
              return (
                <button key={l} onClick={() => exp.toggleLanguageExport(l)} disabled={!has} style={{
                  padding: "3px 10px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: 600,
                  border: on && has ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                  background: on && has ? "rgba(99,102,241,0.08)" : "transparent",
                  color: has ? (on ? C.accent : C.muted) : "#2a2a3a", cursor: has ? "pointer" : "default",
                }}>{l.toUpperCase()}</button>
              );
            })}
          </div>
          <div style={{ marginTop: "auto", display: "flex", gap: 8 }}>
            <Button variant="secondary" size="sm" onClick={copy} style={{ flex: 1 }}>{copied ? "Copied!" : "Copy Text"}</Button>
            <Button size="sm" onClick={doExport} disabled={!exp.outputDirectory || !exp.selectedFormats.length || exporting} style={{ flex: 1 }}>{exporting ? "Exporting..." : "Export"}</Button>
          </div>
          {results.length > 0 && (
            <div style={{ marginTop: 10, padding: "8px 10px", borderRadius: 6, background: successCount === results.length ? "rgba(34,197,94,0.06)" : "rgba(239,68,68,0.06)", border: `1px solid ${successCount === results.length ? "rgba(34,197,94,0.15)" : "rgba(239,68,68,0.15)"}` }}>
              <span style={{ fontSize: 12, color: successCount === results.length ? "#4ade80" : "#f87171", fontWeight: 600 }}>{successCount}/{results.length} exported</span>
            </div>
          )}
        </div>

        {/* GENERATE AUDIO */}
        <div style={{ padding: "18px", borderRadius: 12, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)", display: "flex", flexDirection: "column" }}>
          <div style={{ display: "flex", alignItems: "center", gap: 8, marginBottom: 14 }}>
            <svg width="16" height="16" viewBox="0 0 24 24" fill="none" stroke={C.accent} strokeWidth="2" strokeLinecap="round"><polygon points="11 5 6 9 2 9 2 15 6 15 11 19 11 5"/><path d="M19.07 4.93a10 10 0 010 14.14"/></svg>
            <span style={{ fontWeight: 600, color: C.text, fontSize: 14 }}>Audio</span>
            <span style={{ fontSize: 10, padding: "2px 6px", borderRadius: 4, background: "rgba(99,102,241,0.1)", color: C.accent, fontWeight: 700 }}>ElevenLabs</span>
          </div>

          {!hasElevenLabs ? (
            <div style={{ flex: 1, display: "flex", alignItems: "center", justifyContent: "center", color: C.muted, fontSize: 13, textAlign: "center", padding: "16px 0" }}>
              Add ElevenLabs API key in Settings to enable audio generation.
            </div>
          ) : (
            <>
              {/* Voice & Model selectors */}
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8, marginBottom: 10 }}>
                <div>
                  <label style={sliderLabel}>Voice</label>
                  <select value={elConfig.voice_id} onChange={(e) => {
                    if (e.target.value === "__custom__") return;
                    setElConfig({ ...elConfig, voice_id: e.target.value });
                  }} style={selectStyle}>
                    {elVoices.map((v) => <option key={v.voice_id} value={v.voice_id} style={{ background: "#1a1a24" }}>{v.name} ({v.category})</option>)}
                    <option disabled style={{ background: "#1a1a24", color: "#5a5a6e" }}>──────────</option>
                    <option value="__custom__" style={{ background: "#1a1a24" }}>Custom Voice ID...</option>
                  </select>
                  {/* Custom voice ID input */}
                  <div style={{ marginTop: 6, display: "flex", gap: 4 }}>
                    <input
                      type="text"
                      value={customVoiceId}
                      onChange={(e) => setCustomVoiceId(e.target.value)}
                      placeholder="Paste voice ID here"
                      style={{ flex: 1, padding: "5px 8px", border: `1px solid ${C.border}`, borderRadius: 6, fontSize: 11, background: "rgba(255,255,255,0.04)", color: C.text, fontFamily: "monospace", outline: "none" }}
                    />
                    <button
                      onClick={() => {
                        if (customVoiceId.trim()) {
                          setElConfig({ ...elConfig, voice_id: customVoiceId.trim() });
                          // Add to voice list
                          if (!elVoices.find((v) => v.voice_id === customVoiceId.trim())) {
                            setElVoices([...elVoices, { voice_id: customVoiceId.trim(), name: "Custom", category: "custom" }]);
                          }
                          setCustomVoiceId("");
                        }
                      }}
                      style={{ padding: "5px 10px", borderRadius: 6, border: "none", background: "rgba(99,102,241,0.15)", color: C.accent, fontSize: 11, fontWeight: 600, cursor: "pointer", fontFamily: "inherit" }}
                    >Use</button>
                  </div>
                  <p style={{ fontSize: 10, color: C.muted, marginTop: 4 }}>
                    Add <span style={{ color: C.dim }}>voices_read</span> permission to your key to see all voices, or paste a voice ID.
                  </p>
                </div>
                <div>
                  <label style={sliderLabel}>Model</label>
                  <select value={elConfig.model_id} onChange={(e) => setElConfig({ ...elConfig, model_id: e.target.value })} style={selectStyle}>
                    {ELEVEN_MODELS.map((m) => <option key={m.id} value={m.id} style={{ background: "#1a1a24" }}>{m.label}</option>)}
                  </select>
                </div>
              </div>

              {/* Voice settings */}
              <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8, marginBottom: 12 }}>
                {([
                  ["stability", "Stability", elConfig.stability, 0, 1, 0.05] as const,
                  ["similarity_boost", "Clarity", elConfig.similarity_boost, 0, 1, 0.05] as const,
                  ["speed", "Speed", elConfig.speed, 0.5, 2.0, 0.1] as const,
                  ["style", "Style", elConfig.style, 0, 1, 0.05] as const,
                ]).map(([key, lbl, val, min, max, step]) => (
                  <div key={key}>
                    <label style={{ ...sliderLabel, fontSize: 11 }}>{lbl} ({typeof val === "number" ? val.toFixed(1) : val})</label>
                    <input type="range" min={min} max={max} step={step} value={val}
                      onChange={(e) => setElConfig({ ...elConfig, [key]: parseFloat(e.target.value) })}
                      style={{ width: "100%", accentColor: "#6366f1", height: 4 }} />
                  </div>
                ))}
              </div>

              {/* Mode toggle */}
              <div style={{ display: "flex", gap: 6, marginBottom: 12 }}>
                <button onClick={() => setTtsCompact(true)} style={{
                  flex: 1, padding: "6px 8px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: 600,
                  border: ttsCompact ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                  background: ttsCompact ? "rgba(99,102,241,0.08)" : "transparent", color: ttsCompact ? C.accent : C.muted, cursor: "pointer",
                }}>Single file</button>
                <button onClick={() => setTtsCompact(false)} style={{
                  flex: 1, padding: "6px 8px", borderRadius: 6, fontSize: 11, fontFamily: "inherit", fontWeight: 600,
                  border: !ttsCompact ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                  background: !ttsCompact ? "rgba(99,102,241,0.08)" : "transparent", color: !ttsCompact ? C.accent : C.muted, cursor: "pointer",
                }}>{segCount} segments</button>
              </div>

              {ttsRunning && (
                <div style={{ marginBottom: 10 }}>
                  <div style={{ display: "flex", justifyContent: "space-between", marginBottom: 3 }}>
                    <span style={{ fontSize: 11, color: C.dim }}>Generating...</span>
                    <span style={{ fontSize: 11, color: C.accent }}>{Math.round(ttsProgress)}%</span>
                  </div>
                  <ProgressBar value={ttsProgress} height={3} />
                </div>
              )}

              <Button onClick={doTts} disabled={!exp.outputDirectory || ttsRunning} size="sm" style={{ width: "100%" }}>
                {ttsRunning ? "Generating..." : "Generate Audio"}
              </Button>

              {ttsResults.length > 0 && (
                <div style={{ marginTop: 10, padding: "8px 10px", borderRadius: 6, background: ttsSuccessCount === ttsResults.length ? "rgba(34,197,94,0.06)" : "rgba(239,68,68,0.06)", border: `1px solid ${ttsSuccessCount === ttsResults.length ? "rgba(34,197,94,0.15)" : "rgba(239,68,68,0.15)"}` }}>
                  <span style={{ fontSize: 12, color: ttsSuccessCount === ttsResults.length ? "#4ade80" : "#f87171", fontWeight: 600 }}>{ttsSuccessCount}/{ttsResults.length} generated</span>
                </div>
              )}
            </>
          )}
        </div>
      </div>

      {(results.length > 0 || ttsResults.length > 0) && exp.outputDirectory && (
        <div style={{ padding: "12px 16px", borderRadius: 8, background: "rgba(255,255,255,0.02)", border: `1px solid ${C.border}`, display: "flex", alignItems: "center", gap: 10 }}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#4ade80" strokeWidth="2" strokeLinecap="round"><path d="M22 11.08V12a10 10 0 11-5.93-9.14"/><polyline points="22 4 12 14.01 9 11.01"/></svg>
          <span style={{ fontSize: 13, color: C.dim }}>Exported to: {exp.outputDirectory}</span>
        </div>
      )}
    </div>
  );
}
