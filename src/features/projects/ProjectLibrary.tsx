import { useEffect, useState } from "react";
import { listProjects, deleteProject, type ProjectSummary } from "../../lib/tauri/commands";
import { Button } from "../../components/ui/Button";
import { ConfirmDialog } from "../../components/ui/ConfirmDialog";
import { convertFileSrc } from "@tauri-apps/api/core";
import { trackEvent } from "../telemetry/analytics";

const C = { text: "#e0e0ea", dim: "#8b8ba0", muted: "#5a5a6e", border: "rgba(255,255,255,0.07)", accent: "#818cf8" };

function formatDate(iso: string): string {
  try {
    const d = new Date(iso);
    const now = new Date();
    const diff = now.getTime() - d.getTime();
    const mins = Math.floor(diff / 60000);
    if (mins < 1) return "Just now";
    if (mins < 60) return `${mins}m ago`;
    const hrs = Math.floor(mins / 60);
    if (hrs < 24) return `${hrs}h ago`;
    const days = Math.floor(hrs / 24);
    if (days < 7) return `${days}d ago`;
    return d.toLocaleDateString();
  } catch { return ""; }
}

const STYLE_LABELS: Record<string, string> = {
  executive: "Executive", product_demo: "Product Demo", technical: "Technical",
  teaser: "Teaser", training: "Training",
};

export function ProjectLibrary({ onNewProject, onOpenProject, onOpenSettings }: {
  onNewProject: () => void;
  onOpenProject: (id: string) => void;
  onOpenSettings: () => void;
}) {
  const [projects, setProjects] = useState<ProjectSummary[]>([]);
  const [loading, setLoading] = useState(true);
  const [confirmDelete, setConfirmDelete] = useState<{id:string,title:string}|null>(null);

  const refresh = () => {
    setLoading(true);
    listProjects().then((p) => { setProjects(p); trackEvent("library_loaded", { project_count: p.length }); }).catch(() => {}).finally(() => setLoading(false));
  };
  useEffect(() => { refresh(); }, []);

  const handleDelete = (e: React.MouseEvent, id: string, title: string) => {
    e.stopPropagation();
    setConfirmDelete({ id, title });
  };

  return (
    <div style={{ height: "100%", display: "flex", flexDirection: "column", background: "#0c0c10" }}>
      {/* Top bar — settings only, title is in native title bar */}
      <div style={{ display: "flex", justifyContent: "flex-end", padding: "8px 24px", flexShrink: 0 }}>
        <button onClick={onOpenSettings} style={{ background: "rgba(255,255,255,0.05)", border: "none", borderRadius: 7, width: 28, height: 28, display: "flex", alignItems: "center", justifyContent: "center", cursor: "pointer", color: C.muted }}>
          <svg width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth="2" strokeLinecap="round"><circle cx="12" cy="12" r="3"/><path d="M19.4 15a1.65 1.65 0 00.33 1.82l.06.06a2 2 0 010 2.83 2 2 0 01-2.83 0l-.06-.06a1.65 1.65 0 00-1.82-.33 1.65 1.65 0 00-1 1.51V21a2 2 0 01-4 0v-.09A1.65 1.65 0 009 19.4a1.65 1.65 0 00-1.82.33l-.06.06a2 2 0 01-2.83-2.83l.06-.06A1.65 1.65 0 004.68 15a1.65 1.65 0 00-1.51-1H3a2 2 0 010-4h.09A1.65 1.65 0 004.6 9a1.65 1.65 0 00-.33-1.82l-.06-.06a2 2 0 012.83-2.83l.06.06A1.65 1.65 0 009 4.68a1.65 1.65 0 001-1.51V3a2 2 0 014 0v.09a1.65 1.65 0 001 1.51 1.65 1.65 0 001.82-.33l.06-.06a2 2 0 012.83 2.83l-.06.06A1.65 1.65 0 0019.4 9a1.65 1.65 0 001.51 1H21a2 2 0 010 4h-.09a1.65 1.65 0 00-1.51 1z"/></svg>
        </button>
      </div>

      <div style={{ flex: 1, overflowY: "auto", padding: "20px 40px 40px" }}>
        <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center", marginBottom: 28 }}>
          <div>
            <h1 style={{ fontSize: 28, fontWeight: 700, color: C.text, letterSpacing: -0.5 }}>Projects</h1>
            {projects.length === 0 && <p style={{ color: C.muted, fontSize: 13, marginTop: 4 }}>Create your first narration project</p>}
          </div>
          <span style={{ fontSize: 11, color: C.muted }}>{projects.length} project{projects.length !== 1 ? "s" : ""}</span>
        </div>

        {loading ? (
          <div style={{ textAlign: "center", padding: 60, color: C.muted }}>Loading...</div>
        ) : projects.length === 0 ? (
          <div style={{ display: "flex", flexDirection: "column", alignItems: "center", padding: "80px 0", textAlign: "center" }}>
            <div style={{ width: 80, height: 80, borderRadius: 20, background: "rgba(99,102,241,0.08)", border: "1px solid rgba(99,102,241,0.15)", display: "flex", alignItems: "center", justifyContent: "center", marginBottom: 20 }}>
              <svg width="36" height="36" viewBox="0 0 24 24" fill="none" stroke="#6366f1" strokeWidth="1.5" strokeLinecap="round"><rect x="2" y="2" width="20" height="20" rx="2.18"/><line x1="7" y1="2" x2="7" y2="22"/><line x1="17" y1="2" x2="17" y2="22"/><line x1="2" y1="12" x2="22" y2="12"/></svg>
            </div>
            <h3 style={{ fontSize: 18, fontWeight: 600, color: C.text, marginBottom: 8 }}>No projects yet</h3>
            <p style={{ color: C.muted, fontSize: 14, maxWidth: 320, lineHeight: 1.5, marginBottom: 24 }}>Create a new project to start generating AI narrations for your videos.</p>
            <Button onClick={onNewProject} size="lg">Create First Project</Button>
          </div>
        ) : (
          <div style={{ display: "grid", gridTemplateColumns: "repeat(auto-fill, minmax(280px, 1fr))", gap: 16 }}>
            {/* New project card */}
            <button onClick={onNewProject} style={{
              padding: 0, borderRadius: 12, border: "2px dashed rgba(255,255,255,0.08)",
              background: "rgba(255,255,255,0.02)", cursor: "pointer",
              display: "flex", flexDirection: "column", alignItems: "center", justifyContent: "center",
              minHeight: 200, fontFamily: "inherit", transition: "all 0.15s",
            }}
              onMouseEnter={(e) => { e.currentTarget.style.borderColor = "rgba(99,102,241,0.3)"; e.currentTarget.style.background = "rgba(99,102,241,0.04)"; }}
              onMouseLeave={(e) => { e.currentTarget.style.borderColor = "rgba(255,255,255,0.08)"; e.currentTarget.style.background = "rgba(255,255,255,0.02)"; }}
            >
              <svg width="24" height="24" viewBox="0 0 24 24" fill="none" stroke="#6366f1" strokeWidth="2" strokeLinecap="round"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
              <span style={{ color: C.accent, fontSize: 14, fontWeight: 600, marginTop: 10 }}>New Project</span>
            </button>

            {projects.map((p) => (
              <div key={p.id} onClick={() => onOpenProject(p.id)} style={{
                borderRadius: 12, border: `1px solid ${C.border}`, background: "rgba(255,255,255,0.03)",
                cursor: "pointer", transition: "all 0.15s", overflow: "hidden", display: "flex", flexDirection: "column",
              }}
                onMouseEnter={(e) => { e.currentTarget.style.borderColor = "rgba(99,102,241,0.3)"; e.currentTarget.style.background = "rgba(255,255,255,0.05)"; }}
                onMouseLeave={(e) => { e.currentTarget.style.borderColor = C.border; e.currentTarget.style.background = "rgba(255,255,255,0.03)"; }}
              >
                {/* Thumbnail */}
                <div style={{ height: 120, background: "#0a0a0e", display: "flex", alignItems: "center", justifyContent: "center", position: "relative", overflow: "hidden" }}>
                  {p.thumbnail_path ? (
                    <img src={convertFileSrc(p.thumbnail_path)} alt="" style={{ width: "100%", height: "100%", objectFit: "cover" }} />
                  ) : (
                    <svg width="32" height="32" viewBox="0 0 24 24" fill="none" stroke="#2a2a3a" strokeWidth="1.5" strokeLinecap="round"><polygon points="5 3 19 12 5 21 5 3"/></svg>
                  )}
                  {/* Status badge */}
                  {p.has_script && (
                    <div style={{ position: "absolute", top: 8, right: 8, background: "rgba(34,197,94,0.9)", borderRadius: 5, padding: "2px 8px", fontSize: 10, fontWeight: 700, color: "#fff" }}>
                      READY
                    </div>
                  )}
                </div>

                {/* Info */}
                <div style={{ padding: "14px 16px", flex: 1 }}>
                  <div style={{ fontWeight: 600, color: C.text, fontSize: 14, marginBottom: 6, overflow: "hidden", textOverflow: "ellipsis", whiteSpace: "nowrap" }}>
                    {p.title}
                  </div>
                  <div style={{ display: "flex", gap: 6, flexWrap: "wrap", marginBottom: 8 }}>
                    <span style={{ fontSize: 10, padding: "2px 7px", borderRadius: 4, background: "rgba(99,102,241,0.1)", color: C.accent, fontWeight: 600 }}>
                      {STYLE_LABELS[p.style] || p.style}
                    </span>
                    {p.script_languages.map((l) => (
                      <span key={l} style={{ fontSize: 10, padding: "2px 7px", borderRadius: 4, background: "rgba(255,255,255,0.05)", color: C.muted, fontWeight: 600 }}>
                        {l.toUpperCase()}
                      </span>
                    ))}
                  </div>
                  <div style={{ display: "flex", justifyContent: "space-between", alignItems: "center" }}>
                    <span style={{ fontSize: 11, color: C.muted }}>{formatDate(p.updated_at)}</span>
                    <button onClick={(e) => handleDelete(e, p.id, p.title)} style={{
                      background: "none", border: "none", color: C.muted, cursor: "pointer",
                      fontSize: 11, padding: "3px 6px", borderRadius: 4, fontFamily: "inherit",
                    }}
                      onMouseEnter={(e) => { e.currentTarget.style.color = "#f87171"; e.currentTarget.style.background = "rgba(239,68,68,0.08)"; }}
                      onMouseLeave={(e) => { e.currentTarget.style.color = C.muted; e.currentTarget.style.background = "none"; }}
                    >Delete</button>
                  </div>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      {confirmDelete && (
        <ConfirmDialog
          title="Delete Project"
          message={`Are you sure you want to delete "${confirmDelete.title}"? This cannot be undone.`}
          confirmLabel="Delete"
          danger
          onConfirm={async () => {
            await deleteProject(confirmDelete.id);
            trackEvent("project_deleted");
            setConfirmDelete(null);
            refresh();
          }}
          onCancel={() => setConfirmDelete(null)}
        />
      )}
    </div>
  );
}
