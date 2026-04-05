import { useState, useEffect, useCallback } from "react";
import {
  setApiKey,
  getProviderStatus,
  validateApiKey,
  getElevenLabsConfig,
  saveElevenLabsConfig,
  listElevenLabsVoices,
  validateElevenLabsKey,
  getAzureTtsConfig,
  saveAzureTtsConfig,
  listAzureTtsVoices,
  validateAzureTtsKey,
  getTelemetryEnabled,
  setTelemetryEnabled as setTelemetrySetting,
} from "../../lib/tauri/commands";
import type {
  ElevenLabsConfig,
  ElevenLabsVoice,
  AzureTtsConfig,
  AzureTtsVoice,
} from "../../lib/tauri/commands";
import { PROVIDERS, TTS_PROVIDERS, ELEVEN_MODELS } from "../../lib/constants";
import { useConfigStore } from "../../stores/configStore";
import { Button } from "../../components/ui/Button";
import { setTelemetryEnabled as setAnalyticsEnabled } from "../telemetry/analytics";
import type { AiProvider, ProviderKeyStatus } from "../../types/config";

/* ------------------------------------------------------------------ */
/*  Design tokens                                                      */
/* ------------------------------------------------------------------ */

const C = {
  text: "#e0e0ea",
  dim: "#8b8ba0",
  muted: "#5a5a6e",
  border: "rgba(255,255,255,0.07)",
  borderSubtle: "rgba(255,255,255,0.04)",
  accent: "#818cf8",
  accentDim: "rgba(99,102,241,0.15)",
  bg: "rgba(255,255,255,0.02)",
  bgHover: "rgba(255,255,255,0.04)",
  success: "#4ade80",
  warning: "#f59e0b",
  error: "#f87171",
  dotOff: "#3a3a4a",
};

/* ------------------------------------------------------------------ */
/*  Shared inline styles                                               */
/* ------------------------------------------------------------------ */

const selectStyle: React.CSSProperties = {
  padding: "4px 24px 4px 8px",
  border: `1px solid ${C.border}`,
  borderRadius: 6,
  fontSize: 12,
  background: "rgba(255,255,255,0.04)",
  color: C.text,
  outline: "none",
  fontFamily: "inherit",
  appearance: "none" as const,
  backgroundImage: `url("data:image/svg+xml,%3Csvg width='10' height='6' viewBox='0 0 10 6' xmlns='http://www.w3.org/2000/svg'%3E%3Cpath d='M1 1l4 4 4-4' stroke='%238b8ba0' stroke-width='1.5' fill='none'/%3E%3C/svg%3E")`,
  backgroundRepeat: "no-repeat",
  backgroundPosition: "right 6px center",
  cursor: "pointer",
};

const sectionLabel: React.CSSProperties = {
  fontSize: 11,
  fontWeight: 700,
  color: C.muted,
  textTransform: "uppercase",
  letterSpacing: 1.2,
  marginBottom: 6,
};

/* ------------------------------------------------------------------ */
/*  Constants                                                          */
/* ------------------------------------------------------------------ */

type TabId = "providers" | "ai" | "voice";

interface SettingsPanelProps {
  onClose: () => void;
  onShowPrivacyPolicy?: () => void;
  onShowTerms?: () => void;
  initialTab?: "providers" | "ai" | "voice" | string;
}

const TABS: { id: TabId; label: string }[] = [
  { id: "providers", label: "Providers" },
  { id: "ai", label: "AI" },
  { id: "voice", label: "Voice" },
];

const AZURE_REGIONS = [
  "eastus", "eastus2", "westus", "westus2", "westus3",
  "centralus", "northcentralus", "southcentralus",
  "westeurope", "northeurope", "uksouth",
  "southeastasia", "eastasia", "japaneast", "japanwest",
  "australiaeast", "canadacentral", "brazilsouth",
  "koreacentral", "centralindia",
];

const AZURE_SPEAKING_STYLES = [
  "general", "narration-professional", "narration-relaxed",
  "newscast", "newscast-casual", "chat", "customerservice",
  "friendly", "cheerful", "empathetic", "assistant", "calm", "hopeful",
];

/* ================================================================== */
/*  Main component                                                     */
/* ================================================================== */

export function SettingsPanel({
  onClose,
  onShowPrivacyPolicy,
  onShowTerms,
  initialTab = "providers",
}: SettingsPanelProps) {
  // Gracefully handle legacy "general" tab
  const resolvedInitial: TabId =
    initialTab === "providers" || initialTab === "ai" || initialTab === "voice"
      ? initialTab
      : "providers";

  const [tab, setTab] = useState<TabId>(resolvedInitial);

  /* ------------- shared state ------------- */
  const [statuses, setStatuses] = useState<ProviderKeyStatus[]>([]);
  const [keys, setKeys] = useState<Record<string, string>>({
    claude: "",
    openai: "",
    gemini: "",
  });
  const [saving, setSaving] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);

  /* ------------- ElevenLabs ------------- */
  const [elConfig, setElConfig] = useState<ElevenLabsConfig | null>(null);
  const [elKeyInput, setElKeyInput] = useState("");
  const [elHasKey, setElHasKey] = useState(false);
  const [elVoices, setElVoices] = useState<ElevenLabsVoice[]>([]);
  const [elVoicesLoading, setElVoicesLoading] = useState(false);

  /* ------------- Azure TTS ------------- */
  const [azConfig, setAzConfig] = useState<AzureTtsConfig | null>(null);
  const [azKeyInput, setAzKeyInput] = useState("");
  const [azHasKey, setAzHasKey] = useState(false);
  const [azRegionInput, setAzRegionInput] = useState("eastus");
  const [azVoices, setAzVoices] = useState<AzureTtsVoice[]>([]);
  const [azVoicesLoading, setAzVoicesLoading] = useState(false);

  /* ------------- Telemetry ------------- */
  const [telemetryOn, setTelemetryOn] = useState(true);

  /* ------------- Config store ------------- */
  const {
    aiProvider,
    model,
    temperature,
    ttsProvider,
    setAiProvider,
    setModel,
    setTemperature,
    setTtsProvider,
  } = useConfigStore();

  /* ---------------------------------------------------------------- */
  /*  Load on mount                                                    */
  /* ---------------------------------------------------------------- */

  useEffect(() => {
    getProviderStatus().then(setStatuses).catch(() => {});

    getElevenLabsConfig()
      .then((cfg) => {
        if (cfg) {
          setElConfig(cfg);
          setElHasKey(!!cfg.api_key);
        }
      })
      .catch(() => {});

    getAzureTtsConfig()
      .then((cfg) => {
        if (cfg) {
          setAzConfig(cfg);
          setAzHasKey(!!cfg.api_key);
          setAzRegionInput(cfg.region || "eastus");
        }
      })
      .catch(() => {});

    getTelemetryEnabled().then(setTelemetryOn).catch(() => {});
  }, []);

  /* ---------------------------------------------------------------- */
  /*  Load voices when switching to Voice tab                          */
  /* ---------------------------------------------------------------- */

  useEffect(() => {
    if (tab !== "voice") return;
    if (ttsProvider === "elevenlabs" && elHasKey && elConfig?.api_key) {
      setElVoicesLoading(true);
      listElevenLabsVoices(elConfig.api_key)
        .then(setElVoices)
        .catch(() => {})
        .finally(() => setElVoicesLoading(false));
    }
    if (ttsProvider === "azure" && azHasKey && azConfig?.api_key) {
      setAzVoicesLoading(true);
      listAzureTtsVoices(azConfig.api_key, azConfig.region || "eastus")
        .then(setAzVoices)
        .catch(() => {})
        .finally(() => setAzVoicesLoading(false));
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, ttsProvider]);

  /* ---------------------------------------------------------------- */
  /*  Esc key                                                          */
  /* ---------------------------------------------------------------- */

  const handleEsc = useCallback(
    (e: KeyboardEvent) => {
      if (e.key === "Escape") onClose();
    },
    [onClose],
  );

  useEffect(() => {
    window.addEventListener("keydown", handleEsc);
    return () => window.removeEventListener("keydown", handleEsc);
  }, [handleEsc]);

  /* ---------------------------------------------------------------- */
  /*  Helpers                                                          */
  /* ---------------------------------------------------------------- */

  const providerHasKey = (id: AiProvider) =>
    statuses.find((s) => s.provider === id)?.has_key ?? false;

  const clearFeedback = () => {
    setError(null);
    setSaved(null);
  };

  /* ---------------------------------------------------------------- */
  /*  Save AI key                                                      */
  /* ---------------------------------------------------------------- */

  const handleSaveAiKey = async (provider: AiProvider) => {
    const key = keys[provider];
    if (!key?.trim()) return;
    setSaving(provider);
    clearFeedback();
    try {
      const valid = await validateApiKey(provider, key.trim());
      if (!valid) {
        setError(`Invalid ${provider} API key`);
        setSaving(null);
        return;
      }
      await setApiKey(provider, key.trim());
      setSaved(provider);
      setKeys((k) => ({ ...k, [provider]: "" }));
      getProviderStatus().then(setStatuses);
      setTimeout(() => setSaved(null), 2000);
    } catch (err: unknown) {
      setError(String(err));
    } finally {
      setSaving(null);
    }
  };

  /* ---------------------------------------------------------------- */
  /*  Save ElevenLabs key                                              */
  /* ---------------------------------------------------------------- */

  const handleSaveElKey = async () => {
    if (!elKeyInput.trim()) return;
    setSaving("elevenlabs");
    clearFeedback();
    try {
      const valid = await validateElevenLabsKey(elKeyInput.trim());
      if (!valid) {
        setError("Invalid ElevenLabs API key");
        setSaving(null);
        return;
      }
      const current = await getElevenLabsConfig();
      const cfg: ElevenLabsConfig = current || {
        api_key: "",
        voice_id: "JBFqnCBsd6RMkjVDRZzb",
        model_id: "eleven_multilingual_v2",
        stability: 0.5,
        similarity_boost: 0.75,
        style: 0,
        speed: 1.0,
      };
      cfg.api_key = elKeyInput.trim();
      await saveElevenLabsConfig(cfg);
      setElConfig(cfg);
      setElHasKey(true);
      setElKeyInput("");
      setSaved("elevenlabs");
      setTimeout(() => setSaved(null), 2000);
    } catch (err: unknown) {
      setError(String(err));
    } finally {
      setSaving(null);
    }
  };

  /* ---------------------------------------------------------------- */
  /*  Save Azure TTS key                                               */
  /* ---------------------------------------------------------------- */

  const handleSaveAzKey = async () => {
    if (!azKeyInput.trim()) return;
    setSaving("azure");
    clearFeedback();
    try {
      const valid = await validateAzureTtsKey(azKeyInput.trim(), azRegionInput);
      if (!valid) {
        setError("Invalid Azure TTS API key");
        setSaving(null);
        return;
      }
      const current = await getAzureTtsConfig();
      const cfg: AzureTtsConfig = current || {
        api_key: "",
        region: "eastus",
        voice_name: "en-US-JennyNeural",
        speaking_style: "general",
        speed: 1.0,
      };
      cfg.api_key = azKeyInput.trim();
      cfg.region = azRegionInput;
      await saveAzureTtsConfig(cfg);
      setAzConfig(cfg);
      setAzHasKey(true);
      setAzKeyInput("");
      setSaved("azure");
      setTimeout(() => setSaved(null), 2000);
    } catch (err: unknown) {
      setError(String(err));
    } finally {
      setSaving(null);
    }
  };

  /* ---------------------------------------------------------------- */
  /*  Telemetry                                                        */
  /* ---------------------------------------------------------------- */

  const handleTelemetryToggle = async () => {
    const next = !telemetryOn;
    setTelemetryOn(next);
    setAnalyticsEnabled(next);
    await setTelemetrySetting(next).catch(() => {});
  };

  /* ---------------------------------------------------------------- */
  /*  Save ElevenLabs voice config                                     */
  /* ---------------------------------------------------------------- */

  const saveElVoiceConfig = async (patch: Partial<ElevenLabsConfig>) => {
    if (!elConfig) return;
    const updated = { ...elConfig, ...patch };
    setElConfig(updated);
    try {
      await saveElevenLabsConfig(updated);
    } catch (err: unknown) {
      setError(String(err));
    }
  };

  /* ---------------------------------------------------------------- */
  /*  Save Azure TTS voice config                                      */
  /* ---------------------------------------------------------------- */

  const saveAzVoiceConfig = async (patch: Partial<AzureTtsConfig>) => {
    if (!azConfig) return;
    const updated = { ...azConfig, ...patch };
    setAzConfig(updated);
    try {
      await saveAzureTtsConfig(updated);
    } catch (err: unknown) {
      setError(String(err));
    }
  };

  /* ================================================================ */
  /*  StatusDot (6px)                                                  */
  /* ================================================================ */

  const Dot = ({ on }: { on: boolean }) => (
    <span
      style={{
        width: 6,
        height: 6,
        borderRadius: 3,
        background: on ? C.success : C.dotOff,
        display: "inline-block",
        flexShrink: 0,
      }}
    />
  );

  /* ================================================================ */
  /*  TAB: Providers (merged with General)                             */
  /* ================================================================ */

  const renderProviders = () => (
    <div style={{ display: "flex", flexDirection: "column", gap: 0 }}>
      {/* AI PROVIDERS label */}
      <div style={sectionLabel}>AI Providers</div>

      {PROVIDERS.map((p, i) => {
        const hasKey = providerHasKey(p.id);
        return (
          <div key={p.id}>
            <div
              style={{
                display: "flex",
                alignItems: "center",
                gap: 10,
                height: 40,
              }}
            >
              <Dot on={hasKey} />
              <span
                style={{
                  width: 130,
                  flexShrink: 0,
                  fontSize: 13,
                  fontWeight: 500,
                  color: C.text,
                }}
              >
                {p.label}
              </span>
              <input
                type="password"
                value={keys[p.id] || ""}
                onChange={(e) =>
                  setKeys((k) => ({ ...k, [p.id]: e.target.value }))
                }
                onKeyDown={(e) => e.key === "Enter" && handleSaveAiKey(p.id)}
                placeholder={hasKey ? "Update key..." : "Enter API key..."}
                style={{
                  flex: 1,
                  height: 30,
                  padding: "0 10px",
                  border: `1px solid ${C.border}`,
                  borderRadius: 6,
                  fontSize: 12,
                  fontFamily: "monospace",
                  background: "rgba(255,255,255,0.04)",
                  color: C.text,
                  outline: "none",
                }}
              />
              <Button
                variant="primary"
                size="sm"
                disabled={!keys[p.id]?.trim() || saving === p.id}
                onClick={() => handleSaveAiKey(p.id)}
                style={{ minWidth: 52, height: 30 }}
              >
                {saving === p.id ? "..." : "Save"}
              </Button>
            </div>
            {saved === p.id && (
              <span style={{ fontSize: 11, color: C.success, marginLeft: 16 }}>
                Saved!
              </span>
            )}
            {i < PROVIDERS.length - 1 && (
              <div
                style={{
                  height: 1,
                  background: C.borderSubtle,
                  margin: "0 0 0 16px",
                }}
              />
            )}
          </div>
        );
      })}

      {/* Separator between AI and TTS */}
      <div style={{ height: 1, background: C.border, margin: "12px 0" }} />

      {/* TTS PROVIDERS label */}
      <div style={sectionLabel}>TTS Providers</div>

      {/* ElevenLabs row */}
      <div>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            height: 40,
          }}
        >
          <Dot on={elHasKey} />
          <span
            style={{
              width: 130,
              flexShrink: 0,
              fontSize: 13,
              fontWeight: 500,
              color: C.text,
            }}
          >
            ElevenLabs
          </span>
          <input
            type="password"
            value={elKeyInput}
            onChange={(e) => setElKeyInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSaveElKey()}
            placeholder={elHasKey ? "Update key..." : "Enter API key..."}
            style={{
              flex: 1,
              height: 30,
              padding: "0 10px",
              border: `1px solid ${C.border}`,
              borderRadius: 6,
              fontSize: 12,
              fontFamily: "monospace",
              background: "rgba(255,255,255,0.04)",
              color: C.text,
              outline: "none",
            }}
          />
          <Button
            variant="primary"
            size="sm"
            disabled={!elKeyInput.trim() || saving === "elevenlabs"}
            onClick={handleSaveElKey}
            style={{ minWidth: 52, height: 30 }}
          >
            {saving === "elevenlabs" ? "..." : "Save"}
          </Button>
        </div>
        {saved === "elevenlabs" && (
          <span style={{ fontSize: 11, color: C.success, marginLeft: 16 }}>
            Saved!
          </span>
        )}
      </div>

      <div
        style={{
          height: 1,
          background: C.borderSubtle,
          margin: "0 0 0 16px",
        }}
      />

      {/* Azure TTS row */}
      <div>
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 10,
            height: 40,
          }}
        >
          <Dot on={azHasKey} />
          <span
            style={{
              width: 130,
              flexShrink: 0,
              fontSize: 13,
              fontWeight: 500,
              color: C.text,
            }}
          >
            Azure TTS
          </span>
          <input
            type="password"
            value={azKeyInput}
            onChange={(e) => setAzKeyInput(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && handleSaveAzKey()}
            placeholder={azHasKey ? "Update key..." : "Enter API key..."}
            style={{
              flex: 1,
              height: 30,
              padding: "0 10px",
              border: `1px solid ${C.border}`,
              borderRadius: 6,
              fontSize: 12,
              fontFamily: "monospace",
              background: "rgba(255,255,255,0.04)",
              color: C.text,
              outline: "none",
            }}
          />
          <Button
            variant="primary"
            size="sm"
            disabled={!azKeyInput.trim() || saving === "azure"}
            onClick={handleSaveAzKey}
            style={{ minWidth: 52, height: 30 }}
          >
            {saving === "azure" ? "..." : "Save"}
          </Button>
        </div>
        {saved === "azure" && (
          <span style={{ fontSize: 11, color: C.success, marginLeft: 16 }}>
            Saved!
          </span>
        )}
        {/* Azure region selector inline */}
        <div
          style={{
            display: "flex",
            alignItems: "center",
            gap: 8,
            marginLeft: 16,
            marginTop: 4,
            marginBottom: 4,
          }}
        >
          <label style={{ fontSize: 11, color: C.dim }}>Region</label>
          <select
            value={azRegionInput}
            onChange={(e) => setAzRegionInput(e.target.value)}
            style={{ ...selectStyle, width: 140, height: 26 }}
          >
            {AZURE_REGIONS.map((r) => (
              <option key={r} value={r}>
                {r}
              </option>
            ))}
          </select>
        </div>
      </div>

      {/* ---- PREFERENCES section (merged from General) ---- */}
      <div style={{ height: 1, background: C.border, margin: "12px 0" }} />
      <div style={sectionLabel}>Preferences</div>

      {/* Telemetry toggle row */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          height: 40,
          gap: 10,
        }}
      >
        <div style={{ flex: 1 }}>
          <span style={{ fontSize: 13, fontWeight: 500, color: C.text }}>
            Anonymous Usage Analytics
          </span>
          <span
            style={{
              fontSize: 11,
              color: C.muted,
              marginLeft: 8,
            }}
          >
            Help improve Narrator
          </span>
        </div>
        <button
          onClick={handleTelemetryToggle}
          style={{
            width: 40,
            height: 22,
            borderRadius: 11,
            border: "none",
            cursor: "pointer",
            background: telemetryOn
              ? "rgba(99,102,241,0.6)"
              : "rgba(255,255,255,0.1)",
            position: "relative",
            transition: "background 0.2s ease",
            flexShrink: 0,
          }}
        >
          <span
            style={{
              position: "absolute",
              top: 2,
              left: telemetryOn ? 20 : 2,
              width: 18,
              height: 18,
              borderRadius: 9,
              background: telemetryOn ? C.accent : C.muted,
              transition: "left 0.2s ease, background 0.2s ease",
            }}
          />
        </button>
      </div>

      {/* Legal + version row */}
      <div
        style={{
          display: "flex",
          alignItems: "center",
          height: 32,
          marginTop: 4,
        }}
      >
        <div style={{ display: "flex", gap: 12 }}>
          {onShowPrivacyPolicy && (
            <button
              onClick={onShowPrivacyPolicy}
              style={{
                background: "none",
                border: "none",
                color: C.dim,
                fontSize: 12,
                cursor: "pointer",
                fontFamily: "inherit",
                textDecoration: "underline",
                textUnderlineOffset: 2,
                padding: 0,
              }}
            >
              Privacy Policy
            </button>
          )}
          {onShowPrivacyPolicy && onShowTerms && (
            <span style={{ color: C.muted, fontSize: 12 }}>|</span>
          )}
          {onShowTerms && (
            <button
              onClick={onShowTerms}
              style={{
                background: "none",
                border: "none",
                color: C.dim,
                fontSize: 12,
                cursor: "pointer",
                fontFamily: "inherit",
                textDecoration: "underline",
                textUnderlineOffset: 2,
                padding: 0,
              }}
            >
              Terms of Service
            </button>
          )}
        </div>
        <span
          style={{
            marginLeft: "auto",
            fontSize: 11,
            color: C.muted,
          }}
        >
          Narrator v0.2.0
        </span>
      </div>
    </div>
  );

  /* ================================================================ */
  /*  TAB: AI                                                          */
  /* ================================================================ */

  const renderAi = () => {
    const selectedProviderDef = PROVIDERS.find((p) => p.id === aiProvider);
    const selectedHasKey = providerHasKey(aiProvider);

    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
        <div style={sectionLabel}>AI Provider & Model</div>

        {/* Provider rows in bordered container */}
        <div
          style={{
            border: `1px solid ${C.border}`,
            borderRadius: 10,
            overflow: "hidden",
          }}
        >
          {PROVIDERS.map((p, i) => {
            const isSelected = p.id === aiProvider;
            const hasKey = providerHasKey(p.id);
            return (
              <div
                key={p.id}
                style={{
                  display: "flex",
                  alignItems: "center",
                  gap: 10,
                  height: 42,
                  padding: "0 14px",
                  background: isSelected ? "rgba(99,102,241,0.1)" : "transparent",
                  borderBottom:
                    i < PROVIDERS.length - 1
                      ? `1px solid ${C.border}`
                      : "none",
                  cursor: "pointer",
                  transition: "background 0.15s",
                }}
                onClick={() => {
                  setAiProvider(p.id);
                  setModel(p.models[0].id);
                }}
              >
                {/* Radio 14px */}
                <span
                  style={{
                    width: 14,
                    height: 14,
                    borderRadius: 7,
                    border: `2px solid ${isSelected ? C.accent : C.muted}`,
                    display: "flex",
                    alignItems: "center",
                    justifyContent: "center",
                    flexShrink: 0,
                  }}
                >
                  {isSelected && (
                    <span
                      style={{
                        width: 6,
                        height: 6,
                        borderRadius: 3,
                        background: C.accent,
                      }}
                    />
                  )}
                </span>

                {/* Provider name */}
                <span
                  style={{
                    fontWeight: isSelected ? 600 : 400,
                    color: isSelected ? C.text : C.dim,
                    fontSize: 13,
                    flex: 1,
                  }}
                >
                  {p.label}
                </span>

                {/* Model select w:170 h:28 */}
                <select
                  value={isSelected ? model : p.models[0].id}
                  onChange={(e) => {
                    setAiProvider(p.id);
                    setModel(e.target.value as typeof model);
                  }}
                  onClick={(e) => e.stopPropagation()}
                  style={{
                    ...selectStyle,
                    width: 170,
                    height: 28,
                    fontSize: 12,
                    opacity: isSelected ? 1 : 0.5,
                  }}
                >
                  {p.models.map((m) => (
                    <option key={m.id} value={m.id}>
                      {m.label}
                    </option>
                  ))}
                </select>

                {/* Status dot */}
                <Dot on={hasKey} />
              </div>
            );
          })}
        </div>

        {/* Missing key warning */}
        {!selectedHasKey && selectedProviderDef && (
          <div
            style={{
              padding: "8px 12px",
              background: "rgba(245,158,11,0.08)",
              border: "1px solid rgba(245,158,11,0.2)",
              borderRadius: 8,
              fontSize: 12,
              color: C.warning,
              display: "flex",
              alignItems: "center",
              gap: 6,
            }}
          >
            <span>
              No API key for {selectedProviderDef.label}.{" "}
              <button
                onClick={() => setTab("providers")}
                style={{
                  background: "none",
                  border: "none",
                  color: C.accent,
                  cursor: "pointer",
                  fontFamily: "inherit",
                  fontSize: 12,
                  textDecoration: "underline",
                  textUnderlineOffset: 2,
                  padding: 0,
                }}
              >
                Add key in Providers
              </button>
            </span>
          </div>
        )}

        {/* Temperature section -- no card border */}
        <div style={{ ...sectionLabel, marginTop: 4 }}>Default Temperature</div>
        <div>
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              marginBottom: 6,
            }}
          >
            <span style={{ fontSize: 13, color: C.dim }}>Creativity</span>
            <span
              style={{
                fontSize: 13,
                fontWeight: 600,
                color: C.text,
                fontVariantNumeric: "tabular-nums",
              }}
            >
              {temperature.toFixed(1)}
            </span>
          </div>
          <input
            type="range"
            min={0}
            max={1}
            step={0.1}
            value={temperature}
            onChange={(e) => setTemperature(parseFloat(e.target.value))}
            style={{ width: "100%", accentColor: C.accent }}
          />
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              fontSize: 11,
              color: C.muted,
              marginTop: 4,
            }}
          >
            <span>Precise</span>
            <span>Creative</span>
          </div>
        </div>
      </div>
    );
  };

  /* ================================================================ */
  /*  TAB: Voice                                                       */
  /* ================================================================ */

  const renderVoice = () => {
    const loadElVoices = () => {
      if (!elConfig?.api_key) return;
      setElVoicesLoading(true);
      listElevenLabsVoices(elConfig.api_key)
        .then(setElVoices)
        .catch(() => {})
        .finally(() => setElVoicesLoading(false));
    };

    const loadAzVoices = () => {
      if (!azConfig?.api_key) return;
      setAzVoicesLoading(true);
      listAzureTtsVoices(azConfig.api_key, azConfig.region || "eastus")
        .then(setAzVoices)
        .catch(() => {})
        .finally(() => setAzVoicesLoading(false));
    };

    const handleTtsSwitch = (id: "elevenlabs" | "azure") => {
      setTtsProvider(id);
      if (id === "elevenlabs" && elVoices.length === 0 && elHasKey)
        loadElVoices();
      if (id === "azure" && azVoices.length === 0 && azHasKey) loadAzVoices();
    };

    return (
      <div style={{ display: "flex", flexDirection: "column", gap: 14 }}>
        <div style={sectionLabel}>TTS Provider</div>

        {/* Provider cards (2-col, compact 52px) */}
        <div
          style={{ display: "grid", gridTemplateColumns: "1fr 1fr", gap: 8 }}
        >
          {TTS_PROVIDERS.map((tp) => {
            const isActive = ttsProvider === tp.id;
            const hasKey = tp.id === "elevenlabs" ? elHasKey : azHasKey;
            return (
              <div
                key={tp.id}
                onClick={() => handleTtsSwitch(tp.id)}
                style={{
                  height: 52,
                  padding: "8px 12px",
                  borderRadius: 8,
                  border: `1px solid ${isActive ? "rgba(99,102,241,0.4)" : C.border}`,
                  background: isActive ? C.accentDim : C.bg,
                  cursor: "pointer",
                  transition: "all 0.15s",
                  display: "flex",
                  flexDirection: "column",
                  justifyContent: "center",
                }}
              >
                <div
                  style={{
                    display: "flex",
                    alignItems: "center",
                    gap: 6,
                    marginBottom: 2,
                  }}
                >
                  <span
                    style={{
                      fontWeight: 600,
                      fontSize: 13,
                      color: isActive ? C.text : C.dim,
                    }}
                  >
                    {tp.label}
                  </span>
                  <Dot on={hasKey} />
                </div>
                <span style={{ fontSize: 11, color: C.muted, lineHeight: 1.2 }}>
                  {tp.description}
                </span>
              </div>
            );
          })}
        </div>

        {/* ElevenLabs settings */}
        {ttsProvider === "elevenlabs" && (
          <>
            {!elHasKey ? (
              <div
                style={{
                  padding: "8px 12px",
                  background: "rgba(245,158,11,0.08)",
                  border: "1px solid rgba(245,158,11,0.2)",
                  borderRadius: 8,
                  fontSize: 12,
                  color: C.warning,
                }}
              >
                No API key set.{" "}
                <button
                  onClick={() => setTab("providers")}
                  style={{
                    background: "none",
                    border: "none",
                    color: C.accent,
                    cursor: "pointer",
                    fontFamily: "inherit",
                    fontSize: 12,
                    textDecoration: "underline",
                    textUnderlineOffset: 2,
                    padding: 0,
                  }}
                >
                  Add key in Providers
                </button>
              </div>
            ) : (
              <>
                {/* 2-col: Voice | Model */}
                <div
                  style={{
                    display: "grid",
                    gridTemplateColumns: "1fr 1fr",
                    gap: 10,
                  }}
                >
                  <div>
                    <label
                      style={{
                        fontSize: 11,
                        color: C.dim,
                        display: "block",
                        marginBottom: 4,
                      }}
                    >
                      Voice
                    </label>
                    <select
                      value={elConfig?.voice_id || ""}
                      onChange={(e) =>
                        saveElVoiceConfig({ voice_id: e.target.value })
                      }
                      style={{ ...selectStyle, width: "100%", height: 30 }}
                    >
                      {elVoicesLoading && <option>Loading...</option>}
                      {!elVoicesLoading && elVoices.length === 0 && (
                        <option value={elConfig?.voice_id || ""}>
                          {elConfig?.voice_id || "Default"}
                        </option>
                      )}
                      {elVoices.map((v) => (
                        <option key={v.voice_id} value={v.voice_id}>
                          {v.name} ({v.category})
                        </option>
                      ))}
                    </select>
                  </div>
                  <div>
                    <label
                      style={{
                        fontSize: 11,
                        color: C.dim,
                        display: "block",
                        marginBottom: 4,
                      }}
                    >
                      Model
                    </label>
                    <select
                      value={elConfig?.model_id || "eleven_multilingual_v2"}
                      onChange={(e) =>
                        saveElVoiceConfig({ model_id: e.target.value })
                      }
                      style={{ ...selectStyle, width: "100%", height: 30 }}
                    >
                      {ELEVEN_MODELS.map((m) => (
                        <option key={m.id} value={m.id}>
                          {m.label}
                        </option>
                      ))}
                    </select>
                  </div>
                </div>

                {/* 2-col: Stability | Clarity */}
                <div
                  style={{
                    display: "grid",
                    gridTemplateColumns: "1fr 1fr",
                    gap: 10,
                  }}
                >
                  <div>
                    <div
                      style={{
                        display: "flex",
                        justifyContent: "space-between",
                        marginBottom: 4,
                      }}
                    >
                      <label style={{ fontSize: 11, color: C.dim }}>
                        Stability
                      </label>
                      <span
                        style={{
                          fontSize: 11,
                          color: C.text,
                          fontVariantNumeric: "tabular-nums",
                        }}
                      >
                        {(elConfig?.stability ?? 0.5).toFixed(1)}
                      </span>
                    </div>
                    <input
                      type="range"
                      min={0}
                      max={1}
                      step={0.1}
                      value={elConfig?.stability ?? 0.5}
                      onChange={(e) =>
                        saveElVoiceConfig({
                          stability: parseFloat(e.target.value),
                        })
                      }
                      style={{ width: "100%", accentColor: C.accent }}
                    />
                  </div>
                  <div>
                    <div
                      style={{
                        display: "flex",
                        justifyContent: "space-between",
                        marginBottom: 4,
                      }}
                    >
                      <label style={{ fontSize: 11, color: C.dim }}>
                        Clarity
                      </label>
                      <span
                        style={{
                          fontSize: 11,
                          color: C.text,
                          fontVariantNumeric: "tabular-nums",
                        }}
                      >
                        {(elConfig?.similarity_boost ?? 0.75).toFixed(1)}
                      </span>
                    </div>
                    <input
                      type="range"
                      min={0}
                      max={1}
                      step={0.1}
                      value={elConfig?.similarity_boost ?? 0.75}
                      onChange={(e) =>
                        saveElVoiceConfig({
                          similarity_boost: parseFloat(e.target.value),
                        })
                      }
                      style={{ width: "100%", accentColor: C.accent }}
                    />
                  </div>
                </div>

                {/* Full-width: Custom Voice ID */}
                <div>
                  <label
                    style={{
                      fontSize: 11,
                      color: C.dim,
                      display: "block",
                      marginBottom: 4,
                    }}
                  >
                    Custom Voice ID
                  </label>
                  <input
                    type="text"
                    value={elConfig?.voice_id || ""}
                    onChange={(e) =>
                      saveElVoiceConfig({ voice_id: e.target.value })
                    }
                    placeholder="Paste a custom voice ID..."
                    style={{
                      width: "100%",
                      height: 30,
                      padding: "0 10px",
                      border: `1px solid ${C.border}`,
                      borderRadius: 6,
                      fontSize: 12,
                      fontFamily: "monospace",
                      background: "rgba(255,255,255,0.04)",
                      color: C.text,
                      outline: "none",
                      boxSizing: "border-box",
                    }}
                  />
                </div>
              </>
            )}
          </>
        )}

        {/* Azure settings */}
        {ttsProvider === "azure" && (
          <>
            {!azHasKey ? (
              <div
                style={{
                  padding: "8px 12px",
                  background: "rgba(245,158,11,0.08)",
                  border: "1px solid rgba(245,158,11,0.2)",
                  borderRadius: 8,
                  fontSize: 12,
                  color: C.warning,
                }}
              >
                No API key set.{" "}
                <button
                  onClick={() => setTab("providers")}
                  style={{
                    background: "none",
                    border: "none",
                    color: C.accent,
                    cursor: "pointer",
                    fontFamily: "inherit",
                    fontSize: 12,
                    textDecoration: "underline",
                    textUnderlineOffset: 2,
                    padding: 0,
                  }}
                >
                  Add key in Providers
                </button>
              </div>
            ) : (
              <>
                {/* 2-col: Voice | Speaking Style */}
                <div
                  style={{
                    display: "grid",
                    gridTemplateColumns: "1fr 1fr",
                    gap: 10,
                  }}
                >
                  <div>
                    <label
                      style={{
                        fontSize: 11,
                        color: C.dim,
                        display: "block",
                        marginBottom: 4,
                      }}
                    >
                      Voice
                    </label>
                    <select
                      value={azConfig?.voice_name || ""}
                      onChange={(e) =>
                        saveAzVoiceConfig({ voice_name: e.target.value })
                      }
                      style={{ ...selectStyle, width: "100%", height: 30 }}
                    >
                      {azVoicesLoading && <option>Loading...</option>}
                      {!azVoicesLoading && azVoices.length === 0 && (
                        <option value={azConfig?.voice_name || ""}>
                          {azConfig?.voice_name || "en-US-JennyNeural"}
                        </option>
                      )}
                      {azVoices.map((v) => (
                        <option key={v.short_name} value={v.short_name}>
                          {v.short_name}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div>
                    <label
                      style={{
                        fontSize: 11,
                        color: C.dim,
                        display: "block",
                        marginBottom: 4,
                      }}
                    >
                      Speaking Style
                    </label>
                    <select
                      value={azConfig?.speaking_style || "general"}
                      onChange={(e) =>
                        saveAzVoiceConfig({ speaking_style: e.target.value })
                      }
                      style={{ ...selectStyle, width: "100%", height: 30 }}
                    >
                      {AZURE_SPEAKING_STYLES.map((s) => (
                        <option key={s} value={s}>
                          {s}
                        </option>
                      ))}
                    </select>
                  </div>
                </div>

                {/* Full-width: Speed slider */}
                <div>
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "space-between",
                      marginBottom: 4,
                    }}
                  >
                    <label style={{ fontSize: 11, color: C.dim }}>Speed</label>
                    <span
                      style={{
                        fontSize: 11,
                        color: C.text,
                        fontVariantNumeric: "tabular-nums",
                      }}
                    >
                      {(azConfig?.speed ?? 1.0).toFixed(1)}x
                    </span>
                  </div>
                  <input
                    type="range"
                    min={0.5}
                    max={2.0}
                    step={0.1}
                    value={azConfig?.speed ?? 1.0}
                    onChange={(e) =>
                      saveAzVoiceConfig({
                        speed: parseFloat(e.target.value),
                      })
                    }
                    style={{ width: "100%", accentColor: C.accent }}
                  />
                  <div
                    style={{
                      display: "flex",
                      justifyContent: "space-between",
                      fontSize: 11,
                      color: C.muted,
                      marginTop: 4,
                    }}
                  >
                    <span>0.5x</span>
                    <span>2.0x</span>
                  </div>
                </div>
              </>
            )}
          </>
        )}
      </div>
    );
  };

  /* ================================================================ */
  /*  Render tab content                                               */
  /* ================================================================ */

  const renderContent = () => {
    switch (tab) {
      case "providers":
        return renderProviders();
      case "ai":
        return renderAi();
      case "voice":
        return renderVoice();
    }
  };

  /* ================================================================ */
  /*  Shell                                                            */
  /* ================================================================ */

  return (
    <div
      style={{
        position: "fixed",
        inset: 0,
        zIndex: 100,
        background: "rgba(0,0,0,0.6)",
        backdropFilter: "blur(8px)",
        display: "flex",
        alignItems: "center",
        justifyContent: "center",
      }}
    >
      <div
        style={{
          background: "#16161e",
          borderRadius: 14,
          border: "1px solid rgba(255,255,255,0.08)",
          boxShadow: "0 32px 64px rgba(0,0,0,0.5)",
          width: 600,
          height: 520,
          overflow: "hidden",
          display: "flex",
          flexDirection: "column",
        }}
      >
        {/* Header */}
        <div
          style={{
            display: "flex",
            justifyContent: "space-between",
            alignItems: "center",
            padding: "14px 20px 12px",
            borderBottom: "1px solid rgba(255,255,255,0.07)",
            flexShrink: 0,
          }}
        >
          <h2
            style={{
              fontSize: 16,
              fontWeight: 700,
              color: C.text,
              margin: 0,
            }}
          >
            Settings
          </h2>
          <button
            onClick={onClose}
            style={{
              width: 26,
              height: 26,
              borderRadius: 6,
              border: "none",
              background: "rgba(255,255,255,0.06)",
              color: C.muted,
              cursor: "pointer",
              fontSize: 14,
              display: "flex",
              alignItems: "center",
              justifyContent: "center",
            }}
          >
            &times;
          </button>
        </div>

        {/* Body: tab strip + content */}
        <div
          style={{
            display: "flex",
            flex: 1,
            minHeight: 0,
          }}
        >
          {/* Tab strip: 110px */}
          <div
            style={{
              width: 110,
              flexShrink: 0,
              background: "rgba(255,255,255,0.015)",
              borderRight: `1px solid ${C.border}`,
              padding: "10px 0",
              display: "flex",
              flexDirection: "column",
              gap: 2,
            }}
          >
            {TABS.map((t) => {
              const isActive = t.id === tab;
              return (
                <button
                  key={t.id}
                  onClick={() => {
                    setTab(t.id);
                    clearFeedback();
                  }}
                  style={{
                    display: "block",
                    width: "100%",
                    textAlign: "left",
                    padding: "8px 12px",
                    border: "none",
                    borderLeft: `2px solid ${isActive ? "#818cf8" : "transparent"}`,
                    borderRadius: 0,
                    background: isActive
                      ? "rgba(99,102,241,0.1)"
                      : "transparent",
                    color: isActive ? "#a5b4fc" : "#5a5a6e",
                    fontWeight: isActive ? 600 : 400,
                    fontSize: 13,
                    cursor: "pointer",
                    fontFamily: "inherit",
                    transition: "all 0.12s",
                  }}
                >
                  {t.label}
                </button>
              );
            })}
          </div>

          {/* Scrollable content */}
          <div
            style={{
              flex: 1,
              minHeight: 0,
              overflowY: "auto",
              padding: "16px 20px",
            }}
          >
            {renderContent()}

            {/* Error display */}
            {error && (
              <div
                style={{
                  marginTop: 10,
                  padding: "8px 12px",
                  background: "rgba(239,68,68,0.08)",
                  border: "1px solid rgba(239,68,68,0.2)",
                  borderRadius: 8,
                  fontSize: 12,
                  color: C.error,
                }}
              >
                {error}
              </div>
            )}
          </div>
        </div>

        {/* Footer */}
        <div
          style={{
            display: "flex",
            justifyContent: "flex-end",
            alignItems: "center",
            gap: 12,
            padding: "10px 20px",
            borderTop: "1px solid rgba(255,255,255,0.07)",
            flexShrink: 0,
          }}
        >
          <span style={{ fontSize: 11, color: C.muted }}>
            Press Esc to close
          </span>
          <Button variant="secondary" onClick={onClose}>
            Done
          </Button>
        </div>
      </div>
    </div>
  );
}
