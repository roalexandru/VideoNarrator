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
import { loadProjectFull, probeVideo, saveProject, getTelemetryEnabled, getTtsProvider } from "./lib/tauri/commands";
import { initTelemetry, trackEvent, trackError } from "./features/telemetry/analytics";
import { SettingsProvider, type SettingsTab } from "./contexts/SettingsContext";
import { AppMenuBar } from "./components/layout/AppMenuBar";
import { FeedbackPanel } from "./features/help/FeedbackPanel";
import { AboutDialog } from "./features/help/AboutDialog";
import type { FrameDensity, AiProvider, ModelId, NarrationStyleId } from "./types/config";

const IS_WINDOWS = navigator.userAgent.includes("Windows");

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
  const [showFeedback, setShowFeedback] = useState(false);
  const [showAbout, setShowAbout] = useState(false);
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

  // ── Init telemetry + TTS provider preference on mount ──
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
    // Restore persisted TTS provider preference
    getTtsProvider()
      .then((provider) => {
        if (provider === "azure" || provider === "elevenlabs") {
          useConfigStore.getState().setTtsProvider(provider);
        }
      })
      .catch(() => {});
  }, []);

  // ── Track session duration on unload ──
  // beforeunload is unreliable for async IPC; visibilitychange fires more reliably on app close
  useEffect(() => {
    let sent = false;
    const sendSessionEnd = () => {
      if (sent) return;
      sent = true;
      const duration = Math.round((Date.now() - sessionStart.current) / 1000);
      trackEvent("session_end", { duration_seconds: duration });
    };
    const handleVisibility = () => {
      if (document.visibilityState === "hidden") sendSessionEnd();
    };
    const handleUnload = () => sendSessionEnd();
    document.addEventListener("visibilitychange", handleVisibility);
    window.addEventListener("beforeunload", handleUnload);
    return () => {
      document.removeEventListener("visibilitychange", handleVisibility);
      window.removeEventListener("beforeunload", handleUnload);
    };
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
      clip_type: c.type ?? 'normal',
      freeze_source_time: c.freezeSourceTime,
      freeze_duration: c.freezeDuration,
      zoom_pan: c.zoomPan ? {
        startRegion: c.zoomPan.startRegion,
        endRegion: c.zoomPan.endRegion,
        easing: c.zoomPan.easing,
      } : null,
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
      timeline_effects: es.effects.length > 0 ? es.effects : null,
      video_metadata: ps.videoFile ? {
        path: ps.videoFile.path,
        duration_seconds: ps.videoFile.duration,
        width: ps.videoFile.resolution.width,
        height: ps.videoFile.resolution.height,
        codec: ps.videoFile.codec,
        fps: ps.videoFile.fps,
        file_size: ps.videoFile.size,
      } : null,
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
  const editEffects = useEditStore((s) => s.effects);

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
  }, [view, projectId, videoFile, title, description, contextDocuments, configStyle, configLanguages, configProvider, configModel, editClips, editEffects, buildSavePayload]);

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
        case "send_feedback":
          setShowFeedback(true);
          break;
        case "about_narrator":
          setShowAbout(true);
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

      // Use cached video metadata if available; only probe as a fallback
      const cachedMeta = cfg.video_metadata;
      if (cachedMeta) {
        const { cleanPath, fileNameFromPath } = await import("./lib/formatters");
        const cleanedPath = cleanPath(cachedMeta.path);
        ps.setVideoFile({
          path: cleanedPath, name: fileNameFromPath(cachedMeta.path),
          size: cachedMeta.file_size, duration: cachedMeta.duration_seconds,
          resolution: { width: cachedMeta.width, height: cachedMeta.height },
          codec: cachedMeta.codec, fps: cachedMeta.fps,
        });
      } else {
        try {
          const meta = await probeVideo(cfg.video_path);
          const { fileNameFromPath: fname } = await import("./lib/formatters");
          ps.setVideoFile({
            path: meta.path, name: fname(meta.path),
            size: meta.file_size, duration: meta.duration_seconds,
            resolution: { width: meta.width, height: meta.height },
            codec: meta.codec, fps: meta.fps,
          });
          // Cache the probed metadata so subsequent loads are fast
          try {
            await saveProject({ ...cfg, video_metadata: meta });
          } catch {
            // Non-critical: metadata will be probed again next load
          }
        } catch {
          const { fileNameFromPath: fn2 } = await import("./lib/formatters");
          ps.setVideoFile({
            path: cfg.video_path, name: fn2(cfg.video_path),
            size: 0, duration: 0, resolution: { width: 0, height: 0 }, codec: "unknown", fps: 0,
          });
        }
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
        const restored = cfg.edit_clips.map((c: { source_start: number; source_end: number; speed: number; skip_frames: boolean; fps_override: number | null; clip_type?: string; freeze_source_time?: number; freeze_duration?: number; zoom_pan?: { startRegion: { x: number; y: number; width: number; height: number }; endRegion: { x: number; y: number; width: number; height: number }; easing: string } | null }) => ({
          id: crypto.randomUUID(),
          sourceStart: c.source_start,
          sourceEnd: c.source_end,
          speed: c.speed,
          skipFrames: c.skip_frames,
          fpsOverride: c.fps_override,
          type: (c.clip_type === 'freeze' ? 'freeze' : undefined) as 'normal' | 'freeze' | undefined,
          freezeSourceTime: c.freeze_source_time,
          freezeDuration: c.freeze_duration,
          zoomPan: c.zoom_pan ? {
            startRegion: c.zoom_pan.startRegion,
            endRegion: c.zoom_pan.endRegion,
            easing: c.zoom_pan.easing as 'linear' | 'ease-in' | 'ease-out' | 'ease-in-out',
          } : undefined,
        }));
        // Directly set clips via store — initFromVideo already set sourceDuration
        useEditStore.setState({ clips: restored, selectedClipIndex: 0 });
      }

      // Restore timeline effects if saved
      if (cfg.timeline_effects && Array.isArray(cfg.timeline_effects) && cfg.timeline_effects.length > 0) {
        useEditStore.setState({ effects: cfg.timeline_effects as import("./stores/editStore").TimelineEffect[] });
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

  // Handler for the custom Windows menu bar — mirrors the native menu event handler
  const handleMenuAction = useCallback(async (id: string) => {
    switch (id) {
      case "new_project": handleNewProject(); break;
      case "open_project":
        if (view === "editor") { pendingAction.current = "open"; setShowNewConfirm(true); }
        else setView("library");
        break;
      case "save_project": await handleSaveProject(); break;
      case "open_settings": openSettings(); break;
      case "narrator_help": setShowHelp(true); break;
      case "send_feedback": setShowFeedback(true); break;
      case "about_narrator": setShowAbout(true); break;
      case "check_for_updates": { const { check } = await import("@tauri-apps/plugin-updater"); check().catch(() => {}); break; }
      case "toggle_fullscreen": {
        const win = getCurrentWindow();
        const isFs = await win.isFullscreen();
        await win.setFullscreen(!isFs);
        break;
      }
      default:
        if (id.startsWith("recent:")) {
          const pid = id.slice(7);
          if (pid) handleOpenProject(pid);
        }
    }
  }, [view, handleNewProject, handleSaveProject, openSettings, handleOpenProject]);

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
      {IS_WINDOWS && <AppMenuBar onMenuAction={handleMenuAction} hasProject={view === "editor" && !!videoFile} />}
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
      {showFeedback && <FeedbackPanel onClose={() => setShowFeedback(false)} />}
      {showAbout && <AboutDialog onClose={() => setShowAbout(false)} />}
      {showPrivacyPolicy && <PrivacyPolicy onClose={() => setShowPrivacyPolicy(false)} />}
      {showTerms && <TermsOfService onClose={() => setShowTerms(false)} />}
      {showTelemetryNotice && <TelemetryNotice onClose={() => setShowTelemetryNotice(false)} />}
      <UpdateChecker />
      <ToastContainer />
    </SettingsProvider>
  );
}
