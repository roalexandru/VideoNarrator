import { useEffect, useState, useCallback, useRef } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { listen } from "@tauri-apps/api/event";
import { message } from "@tauri-apps/plugin-dialog";
import { IS_DEV } from "../lib/version";

type Phase = "idle" | "checking" | "available" | "downloading" | "ready";

export function UpdateChecker() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [update, setUpdate] = useState<Update | null>(null);
  const [progress, setProgress] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const checkingRef = useRef(false);

  const doCheck = useCallback(async (manual = false) => {
    if (checkingRef.current) return;
    checkingRef.current = true;
    if (manual) {
      setDismissed(false);
      setPhase("checking");
    }
    try {
      // Timeout after 20s — Windows updater can hang if the endpoint is unreachable
      const result = await Promise.race<Update | null>([
        check(),
        new Promise<null>((_, reject) =>
          setTimeout(() => reject(new Error("Update check timed out")), 20000)
        ),
      ]);
      if (result) {
        setUpdate(result);
        setPhase("available");
        setDismissed(false);
      } else if (manual) {
        setPhase("idle");
        await message("There are currently no updates available.", { title: "Narrator", kind: "info" });
      }
    } catch (e: any) {
      console.debug("Update check:", e?.message || e);
      if (manual) {
        setPhase("idle");
        await message("There are currently no updates available.", { title: "Narrator", kind: "info" });
      }
    } finally {
      checkingRef.current = false;
    }
  }, []);

  // Auto-check on startup (3s delay). Skipped in dev — tauri.conf.json's version
  // is typically behind the latest release, so the updater would offer to replace
  // the dev binary with a production one.
  useEffect(() => {
    if (IS_DEV) return;
    const timer = setTimeout(() => doCheck(false), 3000);
    return () => clearTimeout(timer);
  }, [doCheck]);

  // Listen for "Check for Updates..." menu event
  useEffect(() => {
    const unlisten = listen<string>("menu-event", (event) => {
      if (event.payload === "check_for_updates") {
        doCheck(true);
      }
    });
    return () => { unlisten.then((fn) => fn()); };
  }, [doCheck]);

  const doUpdate = useCallback(async () => {
    if (!update) return;
    setPhase("downloading");
    setProgress(0);
    try {
      let totalBytes = 0;
      let downloadedBytes = 0;
      await update.downloadAndInstall((event) => {
        if (event.event === "Started" && event.data.contentLength) {
          totalBytes = event.data.contentLength;
        } else if (event.event === "Progress") {
          downloadedBytes += event.data.chunkLength;
          if (totalBytes > 0) setProgress(Math.round((downloadedBytes / totalBytes) * 100));
        } else if (event.event === "Finished") {
          setProgress(100);
        }
      });
      setPhase("ready");
    } catch (e: any) {
      console.error("Update failed:", e);
      setPhase("idle");
      await message("Update failed. Please try again later.", { title: "Narrator", kind: "error" });
    }
  }, [update]);

  const doRelaunch = useCallback(async () => {
    await relaunch();
  }, []);

  // Nothing to show
  if (phase === "idle" || dismissed) return null;

  return (
    <div style={{
      position: "fixed", bottom: 0, left: 0, right: 0, zIndex: 9000,
      height: 36, display: "flex", alignItems: "center",
      padding: "0 16px", gap: 12,
      background: "#1e1e2e",
      borderTop: "1px solid rgba(255,255,255,0.07)",
      animation: "slideUpBar 0.2s ease",
    }}>
      <style>{`
        @keyframes slideUpBar { from { transform: translateY(100%); } to { transform: translateY(0); } }
        @keyframes updateSpin { to { transform: rotate(360deg); } }
      `}</style>

      {/* Checking */}
      {phase === "checking" && (
        <>
          <div style={{
            width: 14, height: 14, borderRadius: "50%",
            border: "2px solid rgba(129,140,248,0.25)", borderTopColor: "#818cf8",
            animation: "updateSpin 0.8s linear infinite",
          }} />
          <span style={{ fontSize: 12, color: "#8b8ba0", flex: 1 }}>
            Checking for updates...
          </span>
        </>
      )}

      {phase === "available" && (
        <>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#818cf8" strokeWidth="2" strokeLinecap="round">
            <path d="M21 15v4a2 2 0 01-2 2H5a2 2 0 01-2-2v-4" />
            <polyline points="7 10 12 15 17 10" />
            <line x1="12" y1="15" x2="12" y2="3" />
          </svg>
          <span style={{ fontSize: 12, color: "#e0e0ea", flex: 1 }}>
            New update available{update?.version ? ` (v${update.version})` : ""}
          </span>
          <button onClick={() => setDismissed(true)} style={{
            background: "none", border: "none", color: "#8b8ba0",
            fontSize: 12, cursor: "pointer", fontFamily: "inherit",
          }}>Later</button>
          <button onClick={doUpdate} style={{
            padding: "4px 12px", borderRadius: 6,
            border: "1px solid rgba(255,255,255,0.15)",
            background: "rgba(255,255,255,0.06)", color: "#e0e0ea",
            fontSize: 12, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
          }}>Install Now</button>
        </>
      )}

      {/* Downloading */}
      {phase === "downloading" && (
        <>
          <span style={{ fontSize: 12, color: "#8b8ba0", flex: 1 }}>
            Downloading update... {progress}%
          </span>
          <div style={{
            width: 120, height: 3, borderRadius: 3,
            background: "rgba(255,255,255,0.06)", overflow: "hidden",
          }}>
            <div style={{
              height: "100%", width: `${progress}%`, borderRadius: 3,
              background: "linear-gradient(90deg, #6366f1, #a855f7)",
              transition: "width 0.3s ease",
            }} />
          </div>
        </>
      )}

      {/* Ready to restart */}
      {phase === "ready" && (
        <>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="#4ade80" strokeWidth="2.5" strokeLinecap="round">
            <polyline points="20 6 9 17 4 12" />
          </svg>
          <span style={{ fontSize: 12, color: "#e0e0ea", flex: 1 }}>
            Update installed — restart to apply
          </span>
          <button onClick={() => setDismissed(true)} style={{
            background: "none", border: "none", color: "#8b8ba0",
            fontSize: 12, cursor: "pointer", fontFamily: "inherit",
          }}>Later</button>
          <button onClick={doRelaunch} style={{
            padding: "4px 12px", borderRadius: 6,
            border: "1px solid rgba(255,255,255,0.15)",
            background: "rgba(255,255,255,0.06)", color: "#e0e0ea",
            fontSize: 12, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
          }}>Restart Now</button>
        </>
      )}
    </div>
  );
}
