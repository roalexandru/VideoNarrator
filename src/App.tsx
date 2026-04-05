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
import { ErrorBoundary } from "./components/ErrorBoundary";
import { ToastContainer, showToast } from "./components/ui/Toast";
import { UpdateChecker } from "./components/UpdateChecker";
import { ProjectLibrary } from "./features/projects/ProjectLibrary";
import { loadProjectFull, probeVideo, saveProject } from "./lib/tauri/commands";
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

export default function App() {
  const currentStep = useWizardStore((s) => s.currentStep);
  const [view, setView] = useState<AppView>("library");
  const [showSettings, setShowSettings] = useState(false);
  const [showHelp, setShowHelp] = useState(false);
  const [showNewConfirm, setShowNewConfirm] = useState(false);
  // Tracks what triggered the save dialog: "new" (⌘N) or "open" (⌘O)
  const pendingAction = useRef<"new" | "open">("new");

  // ── Sync menu enabled states whenever the view changes ──
  useEffect(() => {
    invoke("set_menu_context", { hasProject: view === "editor" }).catch(() => {});
  }, [view]);

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
  }, []);

  const handleNewProject = useCallback(() => {
    if (view === "editor") {
      pendingAction.current = "new";
      setShowNewConfirm(true);
    } else {
      doNewProject();
    }
  }, [view, doNewProject]);

  const handleSaveProject = useCallback(async () => {
    const ps = useProjectStore.getState();
    if (!ps.videoFile) {
      showToast("Add a video before saving", "error");
      return;
    }
    const cs = useConfigStore.getState();
    const now = new Date().toISOString();
    try {
      await saveProject({
        id: ps.projectId || crypto.randomUUID(),
        title: ps.title || "Untitled Project",
        description: ps.description || "",
        video_path: ps.videoFile.path,
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
      });
      showToast("Project saved", "success");
    } catch {
      showToast("Failed to save project", "error");
    }
  }, []);

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
          setShowSettings(true);
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
    } catch (err) { console.error("Failed to load project:", err); }
  };

  return (
    <>
      {view === "library" ? (
        <>
          <ProjectLibrary onNewProject={handleNewProject} onOpenProject={handleOpenProject} onOpenSettings={() => setShowSettings(true)} />
          {showSettings && <SettingsPanel onClose={() => setShowSettings(false)} />}
          {showHelp && <HelpPanel onClose={() => setShowHelp(false)} />}
        </>
      ) : (
        <>
          <WizardLayout onOpenSettings={() => setShowSettings(true)} onBackToLibrary={() => setView("library")}>
            <ErrorBoundary>
              {currentStep === 0 && <ProjectSetupScreen />}
              {currentStep === 1 && <EditVideoScreen />}
              {currentStep === 2 && <ConfigurationScreen />}
              {currentStep === 3 && <ProcessingScreen />}
              {currentStep === 4 && <ReviewScreen />}
              {currentStep === 5 && <ExportScreen />}
            </ErrorBoundary>
          </WizardLayout>
          {showSettings && <SettingsPanel onClose={() => setShowSettings(false)} />}
          {showHelp && <HelpPanel onClose={() => setShowHelp(false)} />}
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
      <UpdateChecker />
      <ToastContainer />
    </>
  );
}
