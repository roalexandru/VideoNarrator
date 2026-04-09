import { useState, useEffect, useCallback, useRef } from "react";
import { listen } from "@tauri-apps/api/event";
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { WizardLayout } from "./components/layout/WizardLayout";
import { useWizardStore } from "./hooks/useWizardNavigation";
import { useProjectStore } from "./stores/projectStore";
import { useConfigStore } from "./stores/configStore";
import { useScriptStore } from "./stores/scriptStore";
import { useProcessingStore } from "./stores/processingStore";
import { useEditStore } from "./stores/editStore";
import { useExportStore } from "./stores/exportStore";
import { ProjectSetupScreen } from "./features/project-setup/ProjectSetupScreen";
import { ConfigurationScreen } from "./features/configuration/ConfigurationScreen";
import { ProcessingScreen } from "./features/processing/ProcessingScreen";
import { EditVideoScreen } from "./features/edit-video/EditVideoScreen";
import { ReviewScreen } from "./features/review/ReviewScreen";
import { ExportScreen } from "./features/export/ExportScreen";
import { SettingsPanel } from "./features/settings/SettingsPanel";
import { HelpPanel } from "./features/help/HelpPanel";
import { PrivacyPolicy } from "./features/legal/PrivacyPolicy";
import { TermsOfService } from "./features/legal/TermsOfService";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { ToastContainer, showToast } from "./components/ui/Toast";
import { UpdateChecker } from "./components/UpdateChecker";
import { ProjectLibrary } from "./features/projects/ProjectLibrary";
import { loadProjectFull, probeVideo, saveProject, getTelemetryEnabled } from "./lib/tauri/commands";
import { initTelemetry, trackEvent, trackError } from "./features/telemetry/analytics";
import { SettingsProvider, type SettingsTab } from "./contexts/SettingsContext";
import type { FrameDensity, AiProvider, ModelId, NarrationStyleId } from "./types/config";

type AppView = "library" | "editor";

function NewProjectDialog({ onSaveAndNew, onDiscard, onCancel }: {
  onSaveAndNew: () => void; onDiscard: () => void; onCancel: () => void;
}) {
  return (
    <div style={{
      position: "fixed", inset: 0, zIndex: 9500, background: "rgba(0,0,0,0.5)", backdropFilter: "blur(4px)",
      display: "flex", alignItems: "center", justifyContent: "center",
    }} onClick={(e) => { if (e.target === e.currentTarget) onCancel(); }}>
      <div style={{
        background: "#16161e", borderRadius: 14, border: "1px solid rgba(255,255,255,0.07)",
        boxShadow: "0 24px 64px rgba(0,0,0,0.5)", padding: "24px 28px", maxWidth: 400, width: "100%",
      }}>
        <h3 style={{ fontSize: 17, fontWeight: 700, color: "#e0e0ea", marginBottom: 8 }}>Save before closing?</h3>
        <p style={{ fontSize: 14, color: "#8b8ba0", lineHeight: 1.5, marginBottom: 24 }}>
          Your current project has unsaved changes. Would you like to save before starting a new project?
        </p>
        <div style={{ display: "flex", gap: 10, justifyContent: "flex-end" }}>
          <button onClick={onCancel} style={{
            padding: "8px 18px", borderRadius: 8, border: "1px solid rgba(255,255,255,0.07)",
            background: "transparent", color: "#8b8ba0", fontSize: 14, cursor: "pointer", fontFamily: "inherit",
          }}>Cancel</button>
          <button onClick={onDiscard} style={{
            padding: "8px 18px", borderRadius: 8, border: "1px solid rgba(239,68,68,0.3)",
            background: "rgba(239,68,68,0.08)", color: "#f87171", fontSize: 14, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
          }}>Don't Save</button>
          <button onClick={onSaveAndNew} style={{
            padding: "8px 18px", borderRadius: 8, border: "none",
            background: "linear-gradient(135deg, #6366f1, #8b5cf6)",
            color: "#fff", fontSize: 14, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
          }}>Save & New</button>
        </div>
      </div>
    </div>
  );
}

function TelemetryNotice({ onClose }: { onClose: () => void }) {
  return (
    <div
      style={{
        position: "fixed", bottom: 20, left: "50%", transform: "translateX(-50%)",
        zIndex: 9100, maxWidth: 480, padding: "14px 20px",
        background: "#1e1e28", border: "1px solid rgba(99,102,241,0.25)",
        borderRadius: 12, boxShadow: "0 8px 32px rgba(0,0,0,0.4)",
        display: "flex", alignItems: "center", gap: 14,
        animation: "toastIn 0.3s ease-out",
      }}
    >
      <div style={{ flex: 1, fontSize: 13, color: "#8b8ba0", lineHeight: 1.5 }}>
        <span style={{ color: "#e0e0ea", fontWeight: 600 }}>Anonymous analytics enabled.</span>{" "}
        Narrator collects anonymous usage data to improve the app. No personal data is collected. You can disable this in Settings.
      </div>
      <button
        onClick={onClose}
        style={{
          width: 24, height: 24, borderRadius: 6, border: "none",
          background: "rgba(255,255,255,0.06)", color: "#5a5a6e",
          cursor: "pointer", fontSize: 14, flexShrink: 0,
          display: "flex", alignItems: "center", justifyContent: "center",
        }}
      >&times;</button>
    </div>
  );
}

export default function App() {
  const currentStep = useWizardStore((s) => s.currentStep);
  const sessionStart = useRef(Date.now());
  const [view, setView] = useState<AppView>("library");
  const [settingsState, setSettingsState] = useState<{ open: boolean; tab?: SettingsTab }>({ open: false });
  const openSettings = useCallback((tab?: SettingsTab) => {
    setSettingsState({ open: true, tab });
  }, []);
  const closeSettings = useCallback(() => {
    setSettingsState({ open: false });
  }, []);
  const [showHelp, setShowHelp] = useState(false);
  const [showPrivacyPolicy, setShowPrivacyPolicy] = useState(false);
  const [showTerms, setShowTerms] = useState(false);
  const [showNewConfirm, setShowNewConfirm] = useState(false);
  const [showTelemetryNotice, setShowTelemetryNotice] = useState(false);
  // Tracks what triggered the save dialog: "new" (⌘N) or "open" (⌘O)
  const pendingAction = useRef<"new" | "open">("new");
  const isLoadingProject = useRef(false);

  // ── Sync menu enabled states whenever the view changes ──
  useEffect(() => {
    invoke("set_menu_context", { hasProject: view === "editor" }).catch(() => {});
  }, [view]);

  // ── Init telemetry on mount ──
  useEffect(() => {
    initTelemetry().then(() => {
      trackEvent("app_launched", {
        os: navigator.platform,
        locale: navigator.language,
        screen_width: window.screen.width,
        screen_height: window.screen.height,
      });
    });
    // Show first-launch notice if telemetry_enabled has never been set
    getTelemetryEnabled()
      .then(() => {
        // Telemetry is enabled by default; can be disabled in Settings.
        // Notice popup is disabled — no user prompt needed.
      })
      .catch(() => {});
  }, []);

  // ── Track session duration on unload ──
  useEffect(() => {
    const handleUnload = () => {
      const duration = Math.round((Date.now() - sessionStart.current) / 1000);
      trackEvent("session_end", { duration_seconds: duration });
    };
    window.addEventListener("beforeunload", handleUnload);
    return () => window.removeEventListener("beforeunload", handleUnload);
  }, []);

  const doNewProject = useCallback(() => {
    useProjectStore.getState().reset();
    useProjectStore.getState().setProjectId(crypto.randomUUID());
    useConfigStore.getState().reset();
    useScriptStore.getState().reset();
    useProcessingStore.getState().reset();
    useEditStore.getState().reset();
    useExportStore.getState().reset();
    useWizardStore.getState().reset();
    setView("editor");
    trackEvent("project_created", { source: "new" });
  }, []);

  const handleNewProject = useCallback(() => {
    if (view === "editor") {
      pendingAction.current = "new";
      setShowNewConfirm(true);
    } else {
      doNewProject();
    }
  }, [view, doNewProject]);

  const buildSavePayload = useCallback(() => {
    const ps = useProjectStore.getState();
    const cs = useConfigStore.getState();
    const es = useEditStore.getState();
    const now = new Date().toISOString();
    const editClips = es.clips.length > 0 ? es.clips.map((c) => ({
      source_start: c.sourceStart,
      source_end: c.sourceEnd,
      speed: c.speed,
      skip_frames: c.skipFrames,
      fps_override: c.fpsOverride,
    })) : null;
    return {
      id: ps.projectId || crypto.randomUUID(),
      title: ps.title || "Untitled Project",
      description: ps.description || "",
      video_path: ps.videoFile!.path,
      style: cs.style,
      languages: cs.languages,
      primary_language: cs.primaryLanguage,
      frame_config: {
        density: cs.frameDensity,
        scene_threshold: cs.sceneThreshold,
        max_frames: cs.maxFrames,
      },
      ai_config: {
        provider: cs.aiProvider,
        model: cs.model,
        temperature: cs.temperature,
      },
      custom_prompt: cs.customPrompt,
      created_at: ps.createdAt || now,
      updated_at: now,
      edit_clips: editClips,
    };
  }, []);

  const handleSaveProject = useCallback(async () => {
    const ps = useProjectStore.getState();
    if (!ps.videoFile) {
      showToast("Add a video before saving", "error");
      return;
    }
    try {
      await saveProject(buildSavePayload());
      showToast("Project saved", "success");
    } catch {
      showToast("Failed to save project", "error");
    }
  }, [buildSavePayload]);

  // ── Auto-save project when state changes (debounced) ──
  const autoSaveTimer = useRef<ReturnType<typeof setTimeout> | null>(null);
  const projectId = useProjectStore((s) => s.projectId);
  const videoFile = useProjectStore((s) => s.videoFile);
  const title = useProjectStore((s) => s.title);
  const description = useProjectStore((s) => s.description);
  const contextDocuments = useProjectStore((s) => s.contextDocuments);
  const configStyle = useConfigStore((s) => s.style);
  const configLanguages = useConfigStore((s) => s.languages);
  const configProvider = useConfigStore((s) => s.aiProvider);
  const configModel = useConfigStore((s) => s.model);
  const editClips = useEditStore((s) => s.clips);

  useEffect(() => {
    // Only auto-save when in editor view with a video file (same guard as manual save)
    if (view !== "editor" || !projectId || !videoFile) return;

    if (autoSaveTimer.current) clearTimeout(autoSaveTimer.current);
    autoSaveTimer.current = setTimeout(async () => {
      try {
        await saveProject(buildSavePayload());
      } catch (err: unknown) {
        console.error("Auto-save failed:", err);
        trackError("auto_save", err);
      }
    }, 2000);

    return () => {
      if (autoSaveTimer.current) clearTimeout(autoSaveTimer.current);
    };
  }, [view, projectId, videoFile, title, description, contextDocuments, configStyle, configLanguages, configProvider, configModel, editClips, buildSavePayload]);

  // ── Listen for native menu events from the Rust backend ──
  useEffect(() => {
    const unlisten = listen<string>("menu-event", async (event) => {
      switch (event.payload) {
        case "new_project":
          handleNewProject();
          break;
        case "open_project":
          if (view === "editor") {
            // Same save guard as ⌘N — but navigate to library after
            pendingAction.current = "open";
            setShowNewConfirm(true);
          } else {
            setView("library");
          }
          break;
        case "save_project":
          await handleSaveProject();
          break;
        case "open_settings":
          openSettings();
          break;
        case "narrator_help":
          setShowHelp(true);
          break;
        case "toggle_fullscreen": {
          const win = getCurrentWindow();
          const isFs = await win.isFullscreen();
          await win.setFullscreen(!isFs);
          break;
        }
        default:
          // Handle "recent:<project_id>" events
          if (event.payload.startsWith("recent:")) {
            const projectId = event.payload.slice(7);
            if (projectId) handleOpenProject(projectId);
          }
          break;
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [handleNewProject, handleSaveProject]);

  const handleOpenProject = async (id: string) => {
    if (isLoadingProject.current) return;
    isLoadingProject.current = true;
    try {
      const loaded = await loadProjectFull(id);
      const cfg = loaded.config;

      const ps = useProjectStore.getState();
      ps.setProjectId(cfg.id);
      ps.setTitle(cfg.title);
      ps.setDescription(cfg.description);
      ps.setCreatedAt(cfg.created_at);

      try {
        const meta = await probeVideo(cfg.video_path);
        ps.setVideoFile({
          path: meta.path, name: meta.path.split("/").pop() || "video",
          size: meta.file_size, duration: meta.duration_seconds,
          resolution: { width: meta.width, height: meta.height },
          codec: meta.codec, fps: meta.fps,
        });
      } catch {
        ps.setVideoFile({
          path: cfg.video_path, name: cfg.video_path.split("/").pop() || "video",
          size: 0, duration: 0, resolution: { width: 0, height: 0 }, codec: "unknown", fps: 0,
        });
      }

      const cs = useConfigStore.getState();
      const validStyles: NarrationStyleId[] = ["executive", "product_demo", "technical", "teaser", "training", "critique"];
      if (validStyles.includes(cfg.style as NarrationStyleId)) cs.setStyle(cfg.style as NarrationStyleId);
      cfg.languages.forEach((l) => { if (!cs.languages.includes(l)) cs.toggleLanguage(l); });
      cs.setPrimaryLanguage(cfg.primary_language);
      cs.setFrameDensity(cfg.frame_config.density as FrameDensity);
      cs.setSceneThreshold(cfg.frame_config.scene_threshold);
      cs.setMaxFrames(cfg.frame_config.max_frames);
      cs.setAiProvider(cfg.ai_config.provider as AiProvider);
      cs.setModel(cfg.ai_config.model as ModelId);
      cs.setTemperature(cfg.ai_config.temperature);
      cs.setCustomPrompt(cfg.custom_prompt);

      // Restore video edit clips if saved
      const es = useEditStore.getState();
      es.reset();
      if (cfg.edit_clips && cfg.edit_clips.length > 0) {
        const duration = useProjectStore.getState().videoFile?.duration || 0;
        es.initFromVideo(duration);
        // Replace the default single clip with saved clips
        const restored = cfg.edit_clips.map((c: { source_start: number; source_end: number; speed: number; skip_frames: boolean; fps_override: number | null }) => ({
          id: crypto.randomUUID(),
          sourceStart: c.source_start,
          sourceEnd: c.source_end,
          speed: c.speed,
          skipFrames: c.skip_frames,
          fpsOverride: c.fps_override,
        }));
        // Directly set clips via store — initFromVideo already set sourceDuration
        useEditStore.setState({ clips: restored, selectedClipIndex: 0 });
      }

      const ss = useScriptStore.getState();
      ss.reset();
      for (const [lang, script] of Object.entries(loaded.scripts)) ss.setScript(lang, script);
      if (cfg.primary_language) ss.setActiveLanguage(cfg.primary_language);

      const hasScripts = Object.keys(loaded.scripts).length > 0;
      const ws = useWizardStore.getState();
      ws.reset();
      if (hasScripts) {
        for (let i = 0; i <= 3; i++) ws.markCompleted(i);
        ws.goToStep(4);
      }

      setView("editor");
      trackEvent("project_opened", {
        has_scripts: hasScripts,
        languages: Object.keys(loaded.scripts).length,
      });
    } catch (err) {
      console.error("Failed to load project:", err);
      trackError("load_project", err);
    } finally {
      isLoadingProject.current = false;
    }
  };

  const settingsEl = settingsState.open && (
    <SettingsPanel
      onClose={closeSettings}
      initialTab={settingsState.tab}
      onShowPrivacyPolicy={() => { closeSettings(); setShowPrivacyPolicy(true); }}
      onShowTerms={() => { closeSettings(); setShowTerms(true); }}
    />
  );

  return (
    <SettingsProvider value={openSettings}>
      {view === "library" ? (
        <>
          <ProjectLibrary onNewProject={handleNewProject} onOpenProject={handleOpenProject} onOpenSettings={() => openSettings()} />
          {settingsEl}
          {showHelp && <HelpPanel onClose={() => setShowHelp(false)} onShowPrivacyPolicy={() => { setShowHelp(false); setShowPrivacyPolicy(true); }} onShowTerms={() => { setShowHelp(false); setShowTerms(true); }} />}
        </>
      ) : (
        <>
          <WizardLayout onOpenSettings={() => openSettings()} onBackToLibrary={() => setView("library")}>
            <ErrorBoundary>
              {currentStep === 0 && <ProjectSetupScreen />}
              {currentStep === 1 && <EditVideoScreen />}
              {currentStep === 2 && <ConfigurationScreen />}
              {currentStep === 3 && <ProcessingScreen />}
              {currentStep === 4 && <ReviewScreen />}
              {currentStep === 5 && <ExportScreen />}
            </ErrorBoundary>
          </WizardLayout>
          {settingsEl}
          {showHelp && <HelpPanel onClose={() => setShowHelp(false)} onShowPrivacyPolicy={() => { setShowHelp(false); setShowPrivacyPolicy(true); }} onShowTerms={() => { setShowHelp(false); setShowTerms(true); }} />}
          {showNewConfirm && (
            <NewProjectDialog
              onSaveAndNew={async () => {
                setShowNewConfirm(false);
                await handleSaveProject();
                if (pendingAction.current === "new") doNewProject();
                else setView("library");
              }}
              onDiscard={() => {
                setShowNewConfirm(false);
                if (pendingAction.current === "new") doNewProject();
                else setView("library");
              }}
              onCancel={() => setShowNewConfirm(false)}
            />
          )}
        </>
      )}
      {showPrivacyPolicy && <PrivacyPolicy onClose={() => setShowPrivacyPolicy(false)} />}
      {showTerms && <TermsOfService onClose={() => setShowTerms(false)} />}
      {showTelemetryNotice && <TelemetryNotice onClose={() => setShowTelemetryNotice(false)} />}
      <UpdateChecker />
      <ToastContainer />
    </SettingsProvider>
  );
}
