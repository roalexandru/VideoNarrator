import { useState } from "react";
import { WizardLayout } from "./components/layout/WizardLayout";
import { useWizardStore } from "./hooks/useWizardNavigation";
import { useProjectStore } from "./stores/projectStore";
import { useConfigStore } from "./stores/configStore";
import { useScriptStore } from "./stores/scriptStore";
import { useProcessingStore } from "./stores/processingStore";
import { ProjectSetupScreen } from "./features/project-setup/ProjectSetupScreen";
import { ConfigurationScreen } from "./features/configuration/ConfigurationScreen";
import { ProcessingScreen } from "./features/processing/ProcessingScreen";
import { ReviewScreen } from "./features/review/ReviewScreen";
import { ExportScreen } from "./features/export/ExportScreen";
import { SettingsPanel } from "./features/settings/SettingsPanel";
import { ProjectLibrary } from "./features/projects/ProjectLibrary";
import { loadProjectFull, probeVideo } from "./lib/tauri/commands";
import type { FrameDensity, AiProvider, ModelId } from "./types/config";

type AppView = "library" | "editor";

export default function App() {
  const currentStep = useWizardStore((s) => s.currentStep);
  const [view, setView] = useState<AppView>("library");
  const [showSettings, setShowSettings] = useState(false);

  const handleNewProject = () => {
    // Reset all stores for a fresh project with a new ID
    useProjectStore.getState().reset();
    useProjectStore.getState().setProjectId(crypto.randomUUID());
    useConfigStore.getState().reset();
    useScriptStore.getState().reset();
    useProcessingStore.getState().reset();
    useWizardStore.getState().goToStep(0);
    setView("editor");
  };

  const handleOpenProject = async (id: string) => {
    try {
      const loaded = await loadProjectFull(id);
      const cfg = loaded.config;

      // Populate project store
      const ps = useProjectStore.getState();
      ps.setProjectId(cfg.id);
      ps.setTitle(cfg.title);
      ps.setDescription(cfg.description);

      // Try to probe video to get metadata
      try {
        const meta = await probeVideo(cfg.video_path);
        ps.setVideoFile({
          path: meta.path,
          name: meta.path.split("/").pop() || "video",
          size: meta.file_size,
          duration: meta.duration_seconds,
          resolution: { width: meta.width, height: meta.height },
          codec: meta.codec,
          fps: meta.fps,
        });
      } catch {
        // Video file might have moved; still load the project
        ps.setVideoFile({
          path: cfg.video_path,
          name: cfg.video_path.split("/").pop() || "video",
          size: 0, duration: 0,
          resolution: { width: 0, height: 0 },
          codec: "unknown", fps: 0,
        });
      }

      // Populate config store
      const cs = useConfigStore.getState();
      cs.setStyle(cfg.style as any);
      cfg.languages.forEach((l) => {
        if (!cs.languages.includes(l)) cs.toggleLanguage(l);
      });
      cs.setPrimaryLanguage(cfg.primary_language);
      cs.setFrameDensity(cfg.frame_config.density as FrameDensity);
      cs.setSceneThreshold(cfg.frame_config.scene_threshold);
      cs.setMaxFrames(cfg.frame_config.max_frames);
      cs.setAiProvider(cfg.ai_config.provider as AiProvider);
      cs.setModel(cfg.ai_config.model as ModelId);
      cs.setTemperature(cfg.ai_config.temperature);
      cs.setCustomPrompt(cfg.custom_prompt);

      // Populate script store
      const ss = useScriptStore.getState();
      ss.reset();
      for (const [lang, script] of Object.entries(loaded.scripts)) {
        ss.setScript(lang, script);
      }
      if (cfg.primary_language) ss.setActiveLanguage(cfg.primary_language);

      // Navigate to review if has scripts, otherwise project setup
      const hasScripts = Object.keys(loaded.scripts).length > 0;
      useWizardStore.getState().goToStep(hasScripts ? 3 : 0);
      // Mark previous steps as completed
      if (hasScripts) {
        useWizardStore.getState().markCompleted(0);
        useWizardStore.getState().markCompleted(1);
        useWizardStore.getState().markCompleted(2);
      }

      setView("editor");
    } catch (err) {
      console.error("Failed to load project:", err);
    }
  };

  const handleBackToLibrary = () => {
    setView("library");
  };

  if (view === "library") {
    return (
      <>
        <ProjectLibrary
          onNewProject={handleNewProject}
          onOpenProject={handleOpenProject}
          onOpenSettings={() => setShowSettings(true)}
        />
        {showSettings && <SettingsPanel onClose={() => setShowSettings(false)} />}
      </>
    );
  }

  return (
    <>
      <WizardLayout
        onOpenSettings={() => setShowSettings(true)}
        onBackToLibrary={handleBackToLibrary}
      >
        {currentStep === 0 && <ProjectSetupScreen />}
        {currentStep === 1 && <ConfigurationScreen />}
        {currentStep === 2 && <ProcessingScreen />}
        {currentStep === 3 && <ReviewScreen />}
        {currentStep === 4 && <ExportScreen />}
      </WizardLayout>
      {showSettings && <SettingsPanel onClose={() => setShowSettings(false)} />}
    </>
  );
}
