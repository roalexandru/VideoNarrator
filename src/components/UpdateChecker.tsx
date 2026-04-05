import { useEffect, useState, useCallback } from "react";
import { check, type Update } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";

type Phase = "idle" | "available" | "downloading" | "ready" | "error";

export function UpdateChecker() {
  const [phase, setPhase] = useState<Phase>("idle");
  const [update, setUpdate] = useState<Update | null>(null);
  const [progress, setProgress] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;
    const doCheck = async () => {
      try {
        const result = await check();
        if (!cancelled && result) {
          setUpdate(result);
          setPhase("available");
        }
      } catch (e: any) {
        // Silently ignore update check failures (offline, no releases, dev mode)
        console.debug("Update check:", e?.message || e);
      }
    };
    // Delay check by 3s so it doesn't compete with app startup
    const timer = setTimeout(doCheck, 3000);
    return () => { cancelled = true; clearTimeout(timer); };
  }, []);

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
      setError(e?.message || "Update failed");
      setPhase("error");
    }
  }, [update]);

  const doRelaunch = useCallback(async () => {
    await relaunch();
  }, []);

  // Nothing to show
  if (phase === "idle" || dismissed) return null;

  return (
    <div style={{
      position: "fixed", bottom: 20, right: 20, zIndex: 9000,
      background: "#1e1e2e", border: "1px solid rgba(255,255,255,0.1)",
      borderRadius: 12, padding: "14px 18px", maxWidth: 340,
      boxShadow: "0 8px 32px rgba(0,0,0,0.4)",
      animation: "slideUp 0.3s ease",
    }}>
      <style>{`@keyframes slideUp { from { opacity: 0; transform: translateY(12px); } to { opacity: 1; transform: translateY(0); } }`}</style>

      {/* Close button */}
      {phase !== "downloading" && (
        <button
          onClick={() => setDismissed(true)}
          style={{
            position: "absolute", top: 8, right: 8, background: "none",
            border: "none", color: "#5a5a6e", fontSize: 16, cursor: "pointer",
            width: 20, height: 20, display: "flex", alignItems: "center",
            justifyContent: "center", padding: 0,
          }}
        >x</button>
      )}

      {phase === "available" && (
        <>
          <div style={{ fontSize: 13, fontWeight: 600, color: "#e0e0ea", marginBottom: 4 }}>
            Update available
          </div>
          <div style={{ fontSize: 12, color: "#8b8ba0", marginBottom: 10, lineHeight: 1.4 }}>
            Narrator {update?.version} is ready to download.
            {update?.body && (
              <div style={{ marginTop: 6, fontSize: 11, color: "#6b6b80", maxHeight: 60, overflow: "hidden" }}>
                {update.body.slice(0, 150)}{update.body.length > 150 ? "..." : ""}
              </div>
            )}
          </div>
          <button
            onClick={doUpdate}
            style={{
              width: "100%", padding: "7px 14px", borderRadius: 8, border: "none",
              background: "linear-gradient(135deg, #6366f1, #8b5cf6)", color: "#fff",
              fontSize: 12, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
            }}
          >Update now</button>
        </>
      )}

      {phase === "downloading" && (
        <>
          <div style={{ fontSize: 13, fontWeight: 600, color: "#e0e0ea", marginBottom: 8 }}>
            Downloading update...
          </div>
          <div style={{
            height: 4, width: "100%", borderRadius: 4,
            background: "rgba(255,255,255,0.06)", overflow: "hidden",
          }}>
            <div style={{
              height: "100%", width: `${progress}%`, borderRadius: 4,
              background: "linear-gradient(90deg, #6366f1, #a855f7)",
              transition: "width 0.3s ease",
            }} />
          </div>
          <div style={{ fontSize: 11, color: "#8b8ba0", marginTop: 4 }}>{progress}%</div>
        </>
      )}

      {phase === "ready" && (
        <>
          <div style={{ fontSize: 13, fontWeight: 600, color: "#4ade80", marginBottom: 8 }}>
            Update installed
          </div>
          <div style={{ fontSize: 12, color: "#8b8ba0", marginBottom: 10 }}>
            Restart Narrator to use the new version.
          </div>
          <button
            onClick={doRelaunch}
            style={{
              width: "100%", padding: "7px 14px", borderRadius: 8, border: "none",
              background: "linear-gradient(135deg, #6366f1, #8b5cf6)", color: "#fff",
              fontSize: 12, fontWeight: 600, cursor: "pointer", fontFamily: "inherit",
            }}
          >Restart now</button>
        </>
      )}

      {phase === "error" && (
        <>
          <div style={{ fontSize: 13, fontWeight: 600, color: "#f87171", marginBottom: 4 }}>
            Update failed
          </div>
          <div style={{ fontSize: 11, color: "#8b8ba0" }}>{error}</div>
        </>
      )}
    </div>
  );
}
