import { useState, useEffect, useMemo, type CSSProperties } from "react";
import { useConfigStore } from "../../stores/configStore";
import { useEditStore } from "../../stores/editStore";
import { STYLES, LANGUAGES, PROVIDERS, TTS_PROVIDERS } from "../../lib/constants";
import { Card } from "../../components/ui/Card";
import { getProviderStatus, getElevenLabsConfig, getAzureTtsConfig } from "../../lib/tauri/commands";
import { trackEvent } from "../telemetry/analytics";
import { useOpenSettings } from "../../contexts/SettingsContext";
import { recommendedMaxFrames, isReasoningModel } from "../../lib/frameBudget";
import type { ProviderKeyStatus } from "../../types/config";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };
const label: CSSProperties = { fontSize: 11, fontWeight: 700, color: C.muted, textTransform: "uppercase", letterSpacing: 1.2, marginBottom: 10 };

const summaryCard: CSSProperties = {
  padding: "12px 16px",
  borderRadius: 10,
  border: `1px solid ${C.border}`,
  background: "rgba(255,255,255,0.02)",
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
};

const configureBtn: CSSProperties = {
  background: "none",
  border: "none",
  color: C.accent,
  fontSize: 12,
  fontWeight: 600,
  cursor: "pointer",
  fontFamily: "inherit",
  padding: 0,
};

export function ConfigurationScreen() {
  const config = useConfigStore();
  const openSettings = useOpenSettings();

  // Edited video duration = sum of each clip's output length (post-speed).
  // Used to recommend max_frames so it scales with video length instead of
  // silently truncating long videos at 30 frames.
  const clips = useEditStore((s) => s.clips);
  const editedDuration = useMemo(
    () =>
      clips.reduce((acc, c) => {
        const len = Math.max(0, c.sourceEnd - c.sourceStart);
        const speed = c.speed > 0 ? c.speed : 1;
        return acc + len / speed;
      }, 0),
    [clips],
  );
  const recommended = recommendedMaxFrames(editedDuration, config.frameDensity);

  // Auto-raise maxFrames to the recommendation when duration / density change.
  // Never lower — respect user choosing a higher manual cap.
  const { maxFrames, setMaxFrames } = config;
  useEffect(() => {
    if (maxFrames < recommended) setMaxFrames(recommended);
  }, [recommended, maxFrames, setMaxFrames]);

  // Provider status (which AI providers have keys configured)
  const [providerStatuses, setProviderStatuses] = useState<ProviderKeyStatus[]>([]);
  // Voice info
  const [voiceName, setVoiceName] = useState<string | null>(null);
  const [voiceHasKey, setVoiceHasKey] = useState(false);

  useEffect(() => {
    getProviderStatus()
      .then(setProviderStatuses)
      .catch(() => {});
  }, []);

  useEffect(() => {
    if (config.ttsProvider === "elevenlabs") {
      getElevenLabsConfig()
        .then((cfg) => {
          if (cfg) {
            setVoiceHasKey(!!cfg.api_key);
            setVoiceName(cfg.voice_id ? cfg.voice_id : null);
          } else {
            setVoiceHasKey(false);
            setVoiceName(null);
          }
        })
        .catch(() => {});
    } else if (config.ttsProvider === "azure") {
      getAzureTtsConfig()
        .then((cfg) => {
          if (cfg) {
            setVoiceHasKey(!!cfg.api_key);
            setVoiceName(cfg.voice_name || null);
          } else {
            setVoiceHasKey(false);
            setVoiceName(null);
          }
        })
        .catch(() => {});
    }
  }, [config.ttsProvider]);

  // Derived: current AI provider info
  const currentProvider = PROVIDERS.find((p) => p.id === config.aiProvider);
  const currentModel = currentProvider?.models.find((m) => m.id === config.model);
  const currentProviderStatus = providerStatuses.find((s) => s.provider === config.aiProvider);
  const aiHasKey = currentProviderStatus?.has_key ?? false;
  const reasoning = isReasoningModel(config.aiProvider, config.model);

  // Reasoning models reject a user-set temperature. Pin to 1.0 so the effective
  // value the UI shows matches what the backend will send (it strips temperature
  // for these models but users shouldn't see 0.7 on a control they can't change).
  const { temperature, setTemperature } = config;
  useEffect(() => {
    if (reasoning && temperature !== 1.0) setTemperature(1.0);
  }, [reasoning, temperature, setTemperature]);

  // Derived: current TTS provider info
  const currentTts = TTS_PROVIDERS.find((t) => t.id === config.ttsProvider);

  const displayTemp = reasoning ? 1.0 : config.temperature;

  return (
    <div style={{ maxWidth: 680, margin: "0 auto" }}>
      <div style={{ marginBottom: 32 }}>
        <h2 style={{ fontSize: 24, fontWeight: 700, color: C.text, letterSpacing: -0.3 }}>Configuration</h2>
        <p style={{ color: C.dim, marginTop: 4, fontSize: 14 }}>Style, languages, and AI settings.</p>
      </div>

      {/* Style */}
      <section style={{ marginBottom: 28 }}>
        <div style={label}>Narration Style</div>
        <div style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}>
          {STYLES.map((s) => (
            <Card key={s.id} selected={config.style === s.id} onClick={() => { config.setStyle(s.id); trackEvent("narration_style_selected", { style: s.id }); }}>
              <div style={{ fontWeight: 600, color: config.style === s.id ? C.accent : C.text, fontSize: 14, marginBottom: 3 }}>{s.label}</div>
              <div style={{ fontSize: 12, color: C.dim, lineHeight: 1.4 }}>{s.description}</div>
            </Card>
          ))}
        </div>
      </section>

      {/* Languages */}
      <section style={{ marginBottom: 28 }}>
        <div style={label}>Languages</div>
        <div style={{ display: "flex", flexWrap: "wrap", gap: 6 }}>
          {LANGUAGES.map((lang) => {
            const sel = config.languages.includes(lang.code);
            const prim = config.primaryLanguage === lang.code;
            return (
              <div key={lang.code} style={{ display: "flex", alignItems: "center", gap: 2 }}>
                <button
                  onClick={() => config.toggleLanguage(lang.code)}
                  style={{
                    padding: "7px 14px", borderRadius: sel ? "8px 0 0 8px" : 8, fontSize: 13, fontFamily: "inherit",
                    border: sel ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
                    borderRight: sel ? "none" : undefined,
                    background: sel ? "rgba(99,102,241,0.1)" : "rgba(255,255,255,0.02)",
                    color: sel ? C.accent : C.dim, fontWeight: sel ? 600 : 400, cursor: "pointer",
                  }}>
                  {lang.flag} {lang.label}
                </button>
                {sel && (
                  <button
                    onClick={() => config.setPrimaryLanguage(lang.code)}
                    title={prim ? "Primary language" : "Set as primary"}
                    style={{
                      padding: "7px 8px", borderRadius: "0 8px 8px 0", fontSize: 14, fontFamily: "inherit",
                      border: "1px solid rgba(99,102,241,0.4)", borderLeft: "none",
                      background: prim ? "rgba(99,102,241,0.2)" : "rgba(99,102,241,0.1)",
                      color: prim ? "#facc15" : "rgba(255,255,255,0.2)", cursor: "pointer",
                      lineHeight: 1, display: "flex", alignItems: "center",
                    }}>
                    {"\u2605"}
                  </button>
                )}
              </div>
            );
          })}
        </div>
        <p style={{ fontSize: 11, color: C.muted, marginTop: 6 }}>Click to toggle. Click the star to set a language as primary.</p>
      </section>

      {/* Custom Prompt — inline right after Languages */}
      <section style={{ marginBottom: 28 }}>
        <div style={label}>Custom Prompt</div>
        <textarea
          value={config.customPrompt}
          onChange={(e) => config.setCustomPrompt(e.target.value)}
          placeholder="Extra instructions for the AI (optional)…"
          rows={2}
          style={{
            width: "100%", padding: "9px 12px", border: `1px solid ${C.border}`, borderRadius: 8,
            fontSize: 13, background: "rgba(255,255,255,0.04)", color: C.text,
            resize: "vertical" as const, fontFamily: "inherit",
          }}
        />
      </section>

      {/* Density */}
      <section style={{ marginBottom: 28 }}>
        <div style={label}>Frame Extraction</div>
        <div style={{ display: "flex", gap: 6 }}>
          {([
            { id: "light" as const, label: "Light", desc: "Key points only" },
            { id: "medium" as const, label: "Medium", desc: "Balanced coverage" },
            { id: "heavy" as const, label: "Heavy", desc: "Detailed commentary" },
          ]).map((d) => (
            <button key={d.id} onClick={() => config.setFrameDensity(d.id)} style={{
              flex: 1, padding: "9px 9px 7px", borderRadius: 8, fontSize: 13, fontFamily: "inherit",
              border: config.frameDensity === d.id ? "1px solid rgba(99,102,241,0.4)" : `1px solid ${C.border}`,
              background: config.frameDensity === d.id ? "rgba(99,102,241,0.1)" : "rgba(255,255,255,0.02)",
              color: config.frameDensity === d.id ? C.accent : C.dim,
              fontWeight: config.frameDensity === d.id ? 600 : 400, cursor: "pointer",
              display: "flex", flexDirection: "column", alignItems: "center", gap: 2,
            }}>
              <span>{d.label}</span>
              <span style={{ fontSize: 10, opacity: 0.6, fontWeight: 400 }}>{d.desc}</span>
            </button>
          ))}
        </div>
      </section>

      {/* Max Frames — inline right after Frame Extraction */}
      <section style={{ marginBottom: 28 }}>
        <div style={label}>Max Frames</div>
        <div style={{ display: "flex", alignItems: "center", gap: 12 }}>
          <input
            type="number" min={5} max={300} value={config.maxFrames}
            onChange={(e) => {
              const n = parseInt(e.target.value);
              config.setMaxFrames(Number.isFinite(n) && n > 0 ? n : recommended);
            }}
            style={{
              width: 90, padding: "7px 10px", border: `1px solid ${C.border}`, borderRadius: 6,
              fontSize: 13, background: "rgba(255,255,255,0.04)", color: C.text, fontFamily: "inherit",
            }}
          />
          {editedDuration > 0 ? (
            <span style={{ fontSize: 12, color: C.dim }}>
              Recommended {recommended} for a {Math.round(editedDuration)}s video at {config.frameDensity} density
              {config.maxFrames !== recommended && (
                <>
                  {" — "}
                  <button
                    onClick={() => config.setMaxFrames(recommended)}
                    style={{ background: "none", border: "none", color: C.accent, cursor: "pointer", fontSize: 12, padding: 0, fontFamily: "inherit" }}
                  >
                    reset
                  </button>
                </>
              )}
            </span>
          ) : (
            <span style={{ fontSize: 12, color: C.muted }}>Load a video to see the recommendation.</span>
          )}
        </div>
      </section>

      {/* AI Summary Card */}
      <section style={{ marginBottom: 28 }}>
        <div style={label}>AI</div>
        <div style={summaryCard}>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={{
              width: 8, height: 8, borderRadius: "50%", flexShrink: 0,
              background: aiHasKey ? "#4ade80" : "#fb923c",
            }} />
            <span style={{ fontSize: 14, color: C.text }}>
              {currentProvider?.label ?? config.aiProvider}
              {currentModel ? ` \u00B7 ${currentModel.label}` : ""}
            </span>
          </div>
          <button style={configureBtn} onClick={() => openSettings("ai")}>Configure</button>
        </div>
        {!aiHasKey && (
          <p style={{ fontSize: 11, color: "#fb923c", marginTop: 6 }}>
            No API key configured for {currentProvider?.label ?? config.aiProvider}. Open Settings to add one.
          </p>
        )}
      </section>

      {/* Temperature — inline right after AI. Disabled for reasoning models. */}
      <section style={{ marginBottom: 28 }}>
        <div style={{ ...label, display: "flex", justifyContent: "space-between", alignItems: "center" }}>
          <span>Temperature</span>
          <span style={{
            fontSize: 12, fontWeight: 600, color: reasoning ? C.muted : C.text,
            fontVariantNumeric: "tabular-nums", letterSpacing: 0,
          }}>
            {displayTemp.toFixed(1)}
          </span>
        </div>
        <input
          type="range" min={0} max={1} step={0.1} value={displayTemp}
          disabled={reasoning}
          onChange={(e) => config.setTemperature(parseFloat(e.target.value))}
          style={{
            width: "100%", accentColor: C.accent,
            opacity: reasoning ? 0.4 : 1,
            cursor: reasoning ? "not-allowed" : "pointer",
          }}
        />
        <div style={{ display: "flex", justifyContent: "space-between", fontSize: 11, color: C.muted, marginTop: 4 }}>
          <span>Precise</span>
          <span>Creative</span>
        </div>
        {reasoning && (
          <p style={{ fontSize: 11, color: C.muted, marginTop: 6, lineHeight: 1.5 }}>
            Reasoning models (o1/o3/o4, GPT-5) don't support a custom temperature — they always run at the default.
          </p>
        )}
      </section>

      {/* Strict mode — extra AI critique pass */}
      <section style={{ marginBottom: 28 }}>
        <label
          style={{ display: "flex", alignItems: "flex-start", gap: 12, cursor: "pointer" }}
        >
          <input
            type="checkbox"
            checked={config.strictMode}
            onChange={(e) => config.setStrictMode(e.target.checked)}
            style={{ marginTop: 3, accentColor: C.accent }}
          />
          <div>
            <div style={{ ...label, marginBottom: 2 }}>Strict mode</div>
            <p style={{ fontSize: 11, color: C.muted, lineHeight: 1.5, margin: 0 }}>
              Re-check the draft against a handful of sampled frames and rewrite any segments whose
              narration doesn't match what's on screen. Adds about 30 seconds and one extra API call
              per iteration.
            </p>
          </div>
        </label>
      </section>

      {/* Voice Summary Card */}
      <section style={{ marginBottom: 28 }}>
        <div style={label}>Voice</div>
        <div style={summaryCard}>
          <div style={{ display: "flex", alignItems: "center", gap: 8 }}>
            <span style={{
              width: 8, height: 8, borderRadius: "50%", flexShrink: 0,
              background: voiceHasKey ? "#4ade80" : "#fb923c",
            }} />
            <span style={{ fontSize: 14, color: C.text }}>
              {currentTts?.label ?? config.ttsProvider}
              {voiceName ? ` \u00B7 ${voiceName}` : ""}
            </span>
          </div>
          <button style={configureBtn} onClick={() => openSettings("voice")}>Configure</button>
        </div>
        {!voiceHasKey && (
          <p style={{ fontSize: 11, color: "#fb923c", marginTop: 6 }}>
            No TTS key configured. Open Settings to set up {currentTts?.label ?? "voice synthesis"}.
          </p>
        )}
      </section>

    </div>
  );
}
