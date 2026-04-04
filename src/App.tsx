import { useState } from "react";
import { WizardLayout } from "./components/layout/WizardLayout";
import { useWizardStore } from "./hooks/useWizardNavigation";
import { useProjectStore } from "./stores/projectStore";
import { useConfigStore } from "./stores/configStore";
import { useScriptStore } from "./stores/scriptStore";
import { useProcessingStore } from "./stores/processingStore";
import { useEditStore } from "./stores/editStore";
import { ProjectSetupScreen } from "./features/project-setup/ProjectSetupScreen";
import { ConfigurationScreen } from "./features/configuration/ConfigurationScreen";
import { ProcessingScreen } from "./features/processing/ProcessingScreen";
import { EditVideoScreen } from "./features/edit-video/EditVideoScreen";
import { ReviewScreen } from "./features/review/ReviewScreen";
import { ExportScreen } from "./features/export/ExportScreen";
import { SettingsPanel } from "./features/settings/SettingsPanel";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { ToastContainer } from "./components/ui/Toast";
import { ProjectLibrary } from "./features/projects/ProjectLibrary";
import { loadProjectFull, probeVideo } from "./lib/tauri/commands";
import type { FrameDensity, AiProvider, ModelId, NarrationStyleId } from "./types/config";

type AppView = "library" | "editor";

export default function App() {
  const currentStep = useWizardStore((s) => s.currentStep);
  const [view, setView] = useState<AppView>("library");
  const [showSettings, setShowSettings] = useState(false);

  const handleNewProject = () => {
    useProjectStore.getState().reset();
    useProjectStore.getState().setProjectId(crypto.randomUUID());
    useConfigStore.getState().reset();
    useScriptStore.getState().reset();
    useProcessingStore.getState().reset();
    useEditStore.getState().reset();
    useWizardStore.getState().reset();
    setView("editor");
  };

  const handleOpenProject = async (id: string) => {
    try {
      const loaded = await loadProjectFull(id);
      const cfg = loaded.config;

      const ps = useProjectStore.getState();
      ps.setProjectId(cfg.id);
      ps.setTitle(cfg.title);
      ps.setDescription(cfg.description);

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

      // Navigate: has scripts → Review (step 4), otherwise → Project Setup (step 0)
      const hasScripts = Object.keys(loaded.scripts).length > 0;
      const ws = useWizardStore.getState();
      ws.reset();
      if (hasScripts) {
        for (let i = 0; i <= 3; i++) ws.markCompleted(i);
        ws.goToStep(4); // Review
      }

      setView("editor");
    } catch (err) { console.error("Failed to load project:", err); }
  };

  if (view === "library") {
    return (
      <>
        <ProjectLibrary onNewProject={handleNewProject} onOpenProject={handleOpenProject} onOpenSettings={() => setShowSettings(true)} />
        {showSettings && <SettingsPanel onClose={() => setShowSettings(false)} />}
        <ToastContainer />
      </>
    );
  }

  return (
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
      <ToastContainer />
    </>
  );
}
