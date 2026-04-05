import { useState, useEffect, useCallback } from "react";
import { setApiKey, getProviderStatus, validateApiKey, getElevenLabsConfig, saveElevenLabsConfig, getTelemetryEnabled, setTelemetryEnabled as setTelemetrySetting } from "../../lib/tauri/commands";
import { PROVIDERS } from "../../lib/constants";
import { Button } from "../../components/ui/Button";
import { setTelemetryEnabled as setAnalyticsEnabled } from "../telemetry/analytics";
import type { AiProvider, ProviderKeyStatus } from "../../types/config";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)" };
const input = { width: "100%", padding: "8px 12px", border: `1px solid rgba(255,255,255,0.07)`, borderRadius: 8, fontSize: 13, background: "rgba(255,255,255,0.04)", color: "#e0e0ea", outline: "none", fontFamily: "inherit" };

export function SettingsPanel({ onClose, onShowPrivacyPolicy, onShowTerms }: { onClose: () => void; onShowPrivacyPolicy?: () => void; onShowTerms?: () => void }) {
  const [statuses, setStatuses] = useState<ProviderKeyStatus[]>([]);
  const [keys, setKeys] = useState<Record<string, string>>({ claude: "", openai: "" });
  const [elKeyInput, setElKeyInput] = useState("");
  const [elHasKey, setElHasKey] = useState(false);
  const [saving, setSaving] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [telemetryOn, setTelemetryOn] = useState(true);

  useEffect(() => {
    getProviderStatus().then(setStatuses).catch(() => {});
    getElevenLabsConfig().then((cfg) => { if (cfg) setElHasKey(!!cfg.api_key); }).catch(() => {});
    getTelemetryEnabled().then(setTelemetryOn).catch(() => {});
  }, []);

  const handleTelemetryToggle = async () => {
    const next = !telemetryOn;
    setTelemetryOn(next);
    setAnalyticsEnabled(next);
    await setTelemetrySetting(next).catch(() => {});
  };

  const handleEscKey = useCallback((e: KeyboardEvent) => {
    if (e.key === "Escape") onClose();
  }, [onClose]);

  useEffect(() => {
    window.addEventListener("keydown", handleEscKey);
    return () => window.removeEventListener("keydown", handleEscKey);
  }, [handleEscKey]);

  const handleSaveAiKey = async (provider: AiProvider) => {
    const key = keys[provider]; if (!key.trim()) return;
    setSaving(provider); setError(null); setSaved(null);
    try {
      const valid = await validateApiKey(provider, key.trim());
      if (!valid) { setError(`Invalid ${provider} API key`); setSaving(null); return; }
      await setApiKey(provider, key.trim());
      setSaved(provider); setKeys((k) => ({ ...k, [provider]: "" }));
      getProviderStatus().then(setStatuses);
      setTimeout(() => setSaved(null), 2000);
    } catch (err: any) { setError(String(err)); }
    finally { setSaving(null); }
  };

  const handleSaveElKey = async () => {
    if (!elKeyInput.trim()) return;
    setSaving("elevenlabs"); setError(null);
    try {
      const current = await getElevenLabsConfig();
      const cfg = current || { api_key: "", voice_id: "JBFqnCBsd6RMkjVDRZzb", model_id: "eleven_multilingual_v2", stability: 0.5, similarity_boost: 0.75, style: 0, speed: 1.0 };
      cfg.api_key = elKeyInput.trim();
      await saveElevenLabsConfig(cfg);
      setElHasKey(true); setElKeyInput(""); setSaved("elevenlabs");
      setTimeout(() => setSaved(null), 2000);
    } catch (err: any) { setError(String(err)); }
    finally { setSaving(null); }
  };

  return (
    <div style={{ position: "fixed", inset: 0, zIndex: 100, background: "rgba(0,0,0,0.6)", backdropFilter: "blur(8px)", display: "flex", alignItems: "center", justifyContent: "center" }}>
      <div style={{ background: "#16161e", borderRadius: 14, border: "1px solid rgba(255,255,255,0.08)", boxShadow: "0 32px 64px rgba(0,0,0,0.5)", padding: "28px 32px", width: 480, maxHeight: "85vh", overflowY: "auto" }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 24 }}>
          <h2 style={{ fontSize: 20, fontWeight: 700, color: C.text }}>API Keys</h2>
          <button onClick={onClose} style={{ width: 28, height: 28, borderRadius: 6, border: "none", background: "rgba(255,255,255,0.06)", color: C.muted, cursor: "pointer", fontSize: 16, display: "flex", alignItems: "center", justifyContent: "center" }}>&times;</button>
        </div>

        <p style={{ fontSize: 13, color: C.muted, marginBottom: 20 }}>Configure API keys for AI providers and text-to-speech.</p>

        <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
          {/* AI Providers */}
          {PROVIDERS.map((p) => {
            const status = statuses.find((s) => s.provider === p.id);
            const hasKey = status?.has_key ?? false;
            return (
              <div key={p.id} style={{ padding: "14px 16px", borderRadius: 10, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)" }}>
                <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
                  <span style={{ fontWeight: 600, color: C.text, fontSize: 14 }}>{p.label}</span>
                  <span style={{ fontSize: 11, fontWeight: 600, padding: "2px 8px", borderRadius: 4, background: hasKey ? "rgba(34,197,94,0.12)" : "rgba(255,255,255,0.04)", color: hasKey ? "#4ade80" : C.muted }}>{hasKey ? "Configured" : "Not set"}</span>
                </div>
                <div style={{ display: "flex", gap: 8 }}>
                  <input type="password" value={keys[p.id] || ""} onChange={(e) => setKeys((k) => ({ ...k, [p.id]: e.target.value }))} onKeyDown={(e) => e.key === "Enter" && handleSaveAiKey(p.id)}
                    placeholder={hasKey ? "Enter new key to update..." : `Enter API key...`} style={input} />
                  <Button variant="primary" size="sm" disabled={!keys[p.id]?.trim() || saving === p.id} onClick={() => handleSaveAiKey(p.id)}>{saving === p.id ? "..." : "Save"}</Button>
                </div>
                {saved === p.id && <p style={{ color: "#4ade80", fontSize: 12, marginTop: 6 }}>Saved!</p>}
              </div>
            );
          })}

          {/* ElevenLabs */}
          <div style={{ padding: "14px 16px", borderRadius: 10, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)" }}>
            <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 10 }}>
              <span style={{ fontWeight: 600, color: C.text, fontSize: 14 }}>ElevenLabs TTS</span>
              <span style={{ fontSize: 11, fontWeight: 600, padding: "2px 8px", borderRadius: 4, background: elHasKey ? "rgba(34,197,94,0.12)" : "rgba(255,255,255,0.04)", color: elHasKey ? "#4ade80" : C.muted }}>{elHasKey ? "Configured" : "Not set"}</span>
            </div>
            <div style={{ display: "flex", gap: 8 }}>
              <input type="password" value={elKeyInput} onChange={(e) => setElKeyInput(e.target.value)} onKeyDown={(e) => e.key === "Enter" && handleSaveElKey()}
                placeholder={elHasKey ? "Enter new key to update..." : "Enter ElevenLabs API key..."} style={input} />
              <Button variant="primary" size="sm" disabled={!elKeyInput.trim() || saving === "elevenlabs"} onClick={handleSaveElKey}>{saving === "elevenlabs" ? "..." : "Save"}</Button>
            </div>
            {saved === "elevenlabs" && <p style={{ color: "#4ade80", fontSize: 12, marginTop: 6 }}>Saved!</p>}
          </div>
        </div>

        {/* Privacy & Analytics */}
        <div style={{ marginTop: 20, padding: "14px 16px", borderRadius: 10, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.02)" }}>
          <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
            <div>
              <span style={{ fontWeight: 600, color: C.text, fontSize: 14 }}>Anonymous Usage Analytics</span>
              <p style={{ fontSize: 12, color: C.muted, marginTop: 4, lineHeight: 1.4 }}>
                Help improve Narrator by sharing anonymous usage data. No personal information is collected.
              </p>
            </div>
            <button
              onClick={handleTelemetryToggle}
              style={{
                width: 44, height: 24, borderRadius: 12, border: "none", cursor: "pointer",
                background: telemetryOn ? "rgba(99,102,241,0.6)" : "rgba(255,255,255,0.1)",
                position: "relative", transition: "background 0.2s ease", flexShrink: 0, marginLeft: 16,
              }}
            >
              <span style={{
                position: "absolute", top: 2, left: telemetryOn ? 22 : 2,
                width: 20, height: 20, borderRadius: 10,
                background: telemetryOn ? "#818cf8" : "#5a5a6e",
                transition: "left 0.2s ease, background 0.2s ease",
              }} />
            </button>
          </div>
        </div>

        {/* Legal links */}
        <div style={{ marginTop: 12, display: "flex", gap: 16, justifyContent: "center" }}>
          {onShowPrivacyPolicy && (
            <button onClick={() => { onShowPrivacyPolicy(); }} style={{ background: "none", border: "none", color: C.muted, fontSize: 12, cursor: "pointer", fontFamily: "inherit", textDecoration: "underline", textUnderlineOffset: 2 }}>
              Privacy Policy
            </button>
          )}
          {onShowTerms && (
            <button onClick={() => { onShowTerms(); }} style={{ background: "none", border: "none", color: C.muted, fontSize: 12, cursor: "pointer", fontFamily: "inherit", textDecoration: "underline", textUnderlineOffset: 2 }}>
              Terms of Service
            </button>
          )}
        </div>

        {error && <div style={{ marginTop: 14, padding: "10px 14px", background: "rgba(239,68,68,0.08)", border: "1px solid rgba(239,68,68,0.2)", borderRadius: 8, fontSize: 13, color: "#f87171" }}>{error}</div>}

        <div style={{ marginTop: 20, display: "flex", justifyContent: "flex-end", alignItems: "center", gap: 12 }}>
          <span style={{ fontSize: 11, color: C.muted }}>Press Esc to close</span>
          <Button variant="secondary" onClick={onClose}>Done</Button>
        </div>
      </div>
    </div>
  );
}
