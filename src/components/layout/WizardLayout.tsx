import { useWizardStore, STEP_LABELS } from "../../hooks/useWizardNavigation";
import { NarratorLogo } from "../NarratorLogo";
import type { ReactNode } from "react";

const STEP_DESCRIPTIONS = [
  "Select your video",
  "Trim & adjust",
  "Style & language",
  "Generate narration",
  "Refine script",
  "Save & share",
];

// ── Step icons (SVG) ──
const Icons = [
  // 0: Project Setup (upload)
  <svg key="0" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="17 8 12 3 7 8"/><line x1="12" y1="3" x2="12" y2="15"/></svg>,
  // 1: Edit Video (scissors)
  <svg key="1" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><circle cx="6" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><line x1="20" y1="4" x2="8.12" y2="15.88"/><line x1="14.47" y1="14.48" x2="20" y2="20"/><line x1="8.12" y1="8.12" x2="12" y2="12"/></svg>,
  // 2: Configuration (gear)
  <svg key="2" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/></svg>,
  // 3: Processing (play)
  <svg key="3" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>,
  // 4: Review & Edit (pen)
  <svg key="4" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M11 4H4a2 2 0 00-2 2v14a2 2 0 002 2h14a2 2 0 002-2v-7"/><path d="M18.5 2.5a2.121 2.121 0 013 3L12 15l-4 1 1-4 9.5-9.5z"/></svg>,
  // 5: Export (download)
  <svg key="5" width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4"/><polyline points="7 10 12 15 17 10"/><line x1="12" y1="15" x2="12" y2="3"/></svg>,
];

export function WizardLayout({ children, onOpenSettings, onBackToLibrary }: { children: ReactNode; onOpenSettings?: () => void; onBackToLibrary?: () => void }) {
  const { currentStep, completedSteps, goToStep } = useWizardStore();

  return (
    <div style={{ display: "flex", height: "100%", background: "#0c0c10" }}>
      {/* Sidebar */}
      <aside style={{
        width: 220, background: "#111118",
        borderRight: "1px solid rgba(255,255,255,0.06)",
        display: "flex", flexDirection: "column", flexShrink: 0, userSelect: "none",
        paddingTop: 12,
      }}>
        {/* Back to library */}
        {onBackToLibrary && (
          <div style={{ padding: "4px 10px 0" }}>
            <button onClick={onBackToLibrary} style={{
              display: "flex", alignItems: "center", gap: 6, padding: "6px 8px",
              borderRadius: 6, border: "none", background: "transparent",
              color: "#4a4a5a", fontSize: 12, cursor: "pointer", fontFamily: "inherit",
            }}
              onMouseEnter={(e) => { e.currentTarget.style.color = "#8b8ba0"; e.currentTarget.style.background = "rgba(255,255,255,0.04)"; }}
              onMouseLeave={(e) => { e.currentTarget.style.color = "#4a4a5a"; e.currentTarget.style.background = "transparent"; }}
            >
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><polyline points="15 18 9 12 15 6"/></svg>
              Projects
            </button>
          </div>
        )}

        {/* Logo */}
        <div style={{ padding: "8px 16px 20px" }}>
          <NarratorLogo size={30} />
        </div>

        {/* Nav steps */}
        <nav style={{ flex: 1, display: "flex", flexDirection: "column", gap: 2, padding: "0 10px" }}>
          {STEP_LABELS.map((lbl, i) => {
            const active = i === currentStep;
            const done = completedSteps.has(i);
            return (
              <button key={i} onClick={() => goToStep(i)} style={{
                display: "flex", alignItems: "center", gap: 10,
                padding: "9px 10px", borderRadius: 8, border: "none",
                background: active ? "rgba(99,102,241,0.12)" : "transparent",
                color: active ? "#a5b4fc" : done ? "#8b8ba0" : "#5a5a6e",
                cursor: "pointer",
                fontSize: 13, fontWeight: active ? 600 : 400,
                textAlign: "left", fontFamily: "inherit", transition: "all 0.12s",
              }}
                onMouseEnter={(e) => { if (!active) e.currentTarget.style.background = "rgba(255,255,255,0.04)"; }}
                onMouseLeave={(e) => { if (!active) e.currentTarget.style.background = "transparent"; }}
              >
                <span style={{
                  width: 26, height: 26, borderRadius: 7,
                  display: "flex", alignItems: "center", justifyContent: "center", flexShrink: 0,
                  background: active ? "rgba(99,102,241,0.2)" : done ? "rgba(34,197,94,0.15)" : "rgba(255,255,255,0.04)",
                  color: active ? "#a5b4fc" : done ? "#4ade80" : "#4a4a5a",
                }}>
                  {done && !active
                    ? <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="3" strokeLinecap="round"><polyline points="20 6 9 17 4 12"/></svg>
                    : Icons[i]}
                </span>
                <div>
                  {lbl}
                  {active && (
                    <div style={{ fontSize: 10, color: "#5a5a6e", fontWeight: 400, marginTop: 1 }}>
                      {STEP_DESCRIPTIONS[i]}
                    </div>
                  )}
                </div>
              </button>
            );
          })}
        </nav>

        {/* Settings */}
        {onOpenSettings && (
          <div style={{ padding: "0 10px 8px" }}>
            <button onClick={onOpenSettings} style={{
              display: "flex", alignItems: "center", gap: 8, width: "100%",
              padding: "9px 10px", borderRadius: 8, border: "none",
              background: "transparent", color: "#4a4a5a", fontSize: 13,
              cursor: "pointer", textAlign: "left", fontFamily: "inherit",
            }}
              onMouseEnter={(e) => { e.currentTarget.style.background = "rgba(255,255,255,0.04)"; e.currentTarget.style.color = "#8b8ba0"; }}
              onMouseLeave={(e) => { e.currentTarget.style.background = "transparent"; e.currentTarget.style.color = "#4a4a5a"; }}
            >
              <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/></svg>
              Settings
            </button>
          </div>
        )}
        <div style={{ padding: "8px 20px 12px", fontSize: 11, color: "#2a2a3a" }}>v0.1.0</div>
      </aside>

      {/* Content */}
      <main style={{ flex: 1, overflowY: "auto", padding: "24px 44px 36px" }}>
        {children}
      </main>
    </div>
  );
}
