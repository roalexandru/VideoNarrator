import { useEffect, useCallback } from "react";
import { colors, typography } from "../../lib/theme";
import { Button } from "../../components/ui/Button";

export function PrivacyPolicy({ onClose }: { onClose: () => void }) {
  const handleKeyDown = useCallback((e: KeyboardEvent) => {
    if (e.key === "Escape") onClose();
  }, [onClose]);

  useEffect(() => {
    document.addEventListener("keydown", handleKeyDown);
    return () => document.removeEventListener("keydown", handleKeyDown);
  }, [handleKeyDown]);

  return (
    <div
      style={{
        position: "fixed", inset: 0, zIndex: 110,
        background: "rgba(0,0,0,0.6)", backdropFilter: "blur(8px)",
        display: "flex", alignItems: "center", justifyContent: "center",
      }}
      onClick={(e) => { if (e.target === e.currentTarget) onClose(); }}
    >
      <div style={{
        background: colors.bg.card, borderRadius: 14,
        border: `1px solid ${colors.border.default}`,
        boxShadow: "0 32px 64px rgba(0,0,0,0.5)",
        padding: "28px 32px", width: 560, maxHeight: "85vh",
        display: "flex", flexDirection: "column",
      }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 20, flexShrink: 0 }}>
          <h2 style={{ ...typography.pageTitle, color: colors.text.primary }}>Privacy Policy</h2>
          <button onClick={onClose} style={{
            width: 28, height: 28, borderRadius: 6, border: "none",
            background: colors.bg.hover, color: colors.text.muted,
            cursor: "pointer", fontSize: 16,
            display: "flex", alignItems: "center", justifyContent: "center",
          }}>&times;</button>
        </div>

        <div style={{ overflowY: "auto", flex: 1, paddingRight: 4, fontSize: 13, lineHeight: 1.7, color: colors.text.secondary }}>
          <p style={{ color: colors.text.muted, marginBottom: 16 }}>Last updated: April 2026</p>

          <Section title="What We Collect">
            <p>
              When anonymous analytics is enabled, Narrator collects aggregated usage events to help us understand how the app is used and improve the experience. This includes:
            </p>
            <ul style={{ paddingLeft: 20, marginTop: 8 }}>
              <li>Feature usage (e.g., which export formats are used, narration styles selected)</li>
              <li>App version and operating system type</li>
              <li>Session counts (app launches)</li>
              <li>Processing durations (aggregated, not tied to content)</li>
            </ul>
          </Section>

          <Section title="What We Do NOT Collect">
            <ul style={{ paddingLeft: 20 }}>
              <li>No personally identifiable information (PII)</li>
              <li>No IP addresses</li>
              <li>No device identifiers or fingerprints</li>
              <li>No video content, filenames, or file paths</li>
              <li>No API keys or credentials</li>
              <li>No narration scripts or document content</li>
              <li>No location data</li>
            </ul>
          </Section>

          <Section title="How Data Is Processed">
            <p>
              Anonymous analytics are processed by <strong style={{ color: colors.text.primary }}>Aptabase</strong>, a privacy-first analytics service hosted in the European Union. Aptabase does not collect IP addresses, does not use cookies, and does not create user profiles. Data is stored as anonymous, aggregated metrics only.
            </p>
          </Section>

          <Section title="Opt-Out">
            <p>
              You can disable anonymous analytics at any time in <strong style={{ color: colors.text.primary }}>Settings &gt; Privacy &amp; Analytics</strong>. When disabled, no usage data is transmitted.
            </p>
          </Section>

          <Section title="Third-Party Services">
            <p>
              Narrator integrates with third-party AI and speech services. You provide your own API keys, and data is sent directly from your device to these services:
            </p>
            <ul style={{ paddingLeft: 20, marginTop: 8 }}>
              <li><strong style={{ color: colors.text.primary }}>Anthropic (Claude)</strong> — video frame analysis and narration generation</li>
              <li><strong style={{ color: colors.text.primary }}>OpenAI</strong> — alternative AI provider for narration generation</li>
              <li><strong style={{ color: colors.text.primary }}>ElevenLabs</strong> — text-to-speech audio generation</li>
            </ul>
            <p style={{ marginTop: 8 }}>
              Usage of these services is governed by their respective privacy policies. Narrator does not proxy, store, or log any data sent to these providers.
            </p>
          </Section>

          <Section title="Local Data Storage">
            <p>
              All project data, configuration, and API keys are stored locally on your device at <code style={{ fontSize: 12, background: colors.bg.hover, padding: "1px 5px", borderRadius: 4, color: colors.text.primary }}>~/.narrator/</code>. No project data is ever transmitted to our servers.
            </p>
          </Section>

          <Section title="Contact">
            <p>
              If you have questions about this privacy policy, please open an issue on our{" "}
              <span style={{ color: colors.accent.primary }}>GitHub repository</span>.
            </p>
          </Section>
        </div>

        <div style={{ marginTop: 16, display: "flex", justifyContent: "flex-end", flexShrink: 0 }}>
          <Button variant="secondary" onClick={onClose}>Done</Button>
        </div>
      </div>
    </div>
  );
}

function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div style={{ marginBottom: 20 }}>
      <h3 style={{ ...typography.sectionLabel, color: colors.accent.primary, marginBottom: 8 }}>{title}</h3>
      {children}
    </div>
  );
}
