import { useState, useEffect, useRef, useCallback, type CSSProperties } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { exit } from "@tauri-apps/plugin-process";
import { listProjects } from "../../lib/tauri/commands";

/** Only rendered on Windows — replaces the native menu bar which doesn't support dark mode. */

interface MenuAction {
  id: string;
  label: string;
  shortcut?: string;
  disabled?: boolean;
  separator?: boolean;
  children?: MenuAction[];
}

interface Props {
  onMenuAction: (id: string) => void;
  hasProject?: boolean;
}

export function AppMenuBar({ onMenuAction, hasProject = false }: Props) {
  const [openMenu, setOpenMenu] = useState<string | null>(null);
  const [recentProjects, setRecentProjects] = useState<{ id: string; title: string }[]>([]);
  const barRef = useRef<HTMLDivElement>(null);

  // Load recent projects when File menu opens
  useEffect(() => {
    if (openMenu === "file") {
      listProjects().then((projects) => {
        setRecentProjects(projects.slice(0, 8).map((p) => ({ id: p.id, title: p.title })));
      }).catch(() => {});
    }
  }, [openMenu]);

  // Close on outside click
  useEffect(() => {
    if (!openMenu) return;
    const handle = (e: MouseEvent) => {
      if (barRef.current && !barRef.current.contains(e.target as Node)) setOpenMenu(null);
    };
    document.addEventListener("mousedown", handle);
    return () => document.removeEventListener("mousedown", handle);
  }, [openMenu]);

  // Close on Escape
  useEffect(() => {
    if (!openMenu) return;
    const handle = (e: KeyboardEvent) => { if (e.key === "Escape") setOpenMenu(null); };
    window.addEventListener("keydown", handle);
    return () => window.removeEventListener("keydown", handle);
  }, [openMenu]);

  const handleAction = useCallback((id: string) => {
    setOpenMenu(null);
    onMenuAction(id);
  }, [onMenuAction]);

  const handleEditAction = useCallback((cmd: string) => {
    setOpenMenu(null);
    document.execCommand(cmd);
  }, []);

  const handleToggleFullscreen = useCallback(async () => {
    setOpenMenu(null);
    const win = getCurrentWindow();
    const isFs = await win.isFullscreen();
    await win.setFullscreen(!isFs);
  }, []);

  const handleExit = useCallback(async () => {
    setOpenMenu(null);
    await exit(0);
  }, []);

  const menus: { id: string; label: string; items: MenuAction[] }[] = [
    {
      id: "file", label: "File", items: [
        { id: "new_project", label: "New Project...", shortcut: "Ctrl+N" },
        { id: "open_project", label: "Open Project...", shortcut: "Ctrl+O" },
        { id: "open_recent", label: "Open Recent", children: recentProjects.length > 0
          ? recentProjects.map((p) => ({ id: `recent:${p.id}`, label: p.title }))
          : [{ id: "_none", label: "No Recent Projects", disabled: true }],
        },
        { id: "_sep1", label: "", separator: true },
        { id: "save_project", label: "Save Project", shortcut: "Ctrl+S", disabled: !hasProject },
        { id: "_sep2", label: "", separator: true },
        { id: "open_settings", label: "Settings", shortcut: "Ctrl+," },
        { id: "_sep3", label: "", separator: true },
        { id: "_exit", label: "Exit" },
      ],
    },
    {
      id: "edit", label: "Edit", items: [
        { id: "_undo", label: "Undo", shortcut: "Ctrl+Z" },
        { id: "_redo", label: "Redo", shortcut: "Ctrl+Y" },
        { id: "_sep1", label: "", separator: true },
        { id: "_cut", label: "Cut", shortcut: "Ctrl+X" },
        { id: "_copy", label: "Copy", shortcut: "Ctrl+C" },
        { id: "_paste", label: "Paste", shortcut: "Ctrl+V" },
        { id: "_selectall", label: "Select All", shortcut: "Ctrl+A" },
      ],
    },
    {
      id: "view", label: "View", items: [
        { id: "_fullscreen", label: "Toggle Full Screen", shortcut: "F11" },
      ],
    },
    {
      id: "help", label: "Help", items: [
        { id: "about_narrator", label: "About Narrator" },
        { id: "check_for_updates", label: "Check for Updates..." },
        { id: "_sep1", label: "", separator: true },
        { id: "narrator_help", label: "Narrator Help", shortcut: "F1" },
        { id: "send_feedback", label: "Send Feedback..." },
      ],
    },
  ];

  return (
    <div ref={barRef} style={bar} data-tauri-drag-region>
      {menus.map((menu) => (
        <div key={menu.id} style={{ position: "relative" }}>
          <button
            style={{
              ...menuBtn,
              background: openMenu === menu.id ? "rgba(255,255,255,0.08)" : "transparent",
              color: openMenu === menu.id ? "#e0e0ea" : "#8b8ba0",
            }}
            onMouseDown={() => setOpenMenu(openMenu === menu.id ? null : menu.id)}
            onMouseEnter={() => { if (openMenu && openMenu !== menu.id) setOpenMenu(menu.id); }}
          >
            {menu.label}
          </button>
          {openMenu === menu.id && (
            <div style={dropdown}>
              {menu.items.map((item, i) =>
                item.separator ? (
                  <div key={`sep-${i}`} style={separator} />
                ) : item.children ? (
                  <SubMenu key={item.id} item={item} onAction={handleAction} />
                ) : (
                  <MenuItem key={item.id} item={item} onAction={(id) => {
                    if (id === "_exit") handleExit();
                    else if (id === "_fullscreen") handleToggleFullscreen();
                    else if (id === "_undo") handleEditAction("undo");
                    else if (id === "_redo") handleEditAction("redo");
                    else if (id === "_cut") handleEditAction("cut");
                    else if (id === "_copy") handleEditAction("copy");
                    else if (id === "_paste") handleEditAction("paste");
                    else if (id === "_selectall") handleEditAction("selectAll");
                    else handleAction(id);
                  }} />
                ),
              )}
            </div>
          )}
        </div>
      ))}
    </div>
  );
}

function MenuItem({ item, onAction }: { item: MenuAction; onAction: (id: string) => void }) {
  const [hover, setHover] = useState(false);
  return (
    <button
      style={{
        ...menuItem,
        background: hover && !item.disabled ? "rgba(255,255,255,0.06)" : "transparent",
        color: item.disabled ? "#3a3a4a" : "#e0e0ea",
        cursor: item.disabled ? "default" : "pointer",
      }}
      disabled={item.disabled}
      onClick={() => !item.disabled && onAction(item.id)}
      onMouseEnter={() => setHover(true)}
      onMouseLeave={() => setHover(false)}
    >
      <span>{item.label}</span>
      {item.shortcut && <span style={shortcutStyle}>{item.shortcut}</span>}
    </button>
  );
}

function SubMenu({ item, onAction }: { item: MenuAction; onAction: (id: string) => void }) {
  const [hover, setHover] = useState(false);
  const [showSub, setShowSub] = useState(false);

  return (
    <div
      style={{ position: "relative" }}
      onMouseEnter={() => { setHover(true); setShowSub(true); }}
      onMouseLeave={() => { setHover(false); setShowSub(false); }}
    >
      <div style={{
        ...menuItem,
        background: hover ? "rgba(255,255,255,0.06)" : "transparent",
        color: "#e0e0ea",
        cursor: "pointer",
      }}>
        <span>{item.label}</span>
        <span style={{ fontSize: 10, color: "#5a5a6e" }}>&#9654;</span>
      </div>
      {showSub && item.children && (
        <div style={{ ...dropdown, position: "absolute", left: "100%", top: 0 }}>
          {item.children.map((child) => (
            <MenuItem key={child.id} item={child} onAction={onAction} />
          ))}
        </div>
      )}
    </div>
  );
}

// ── Styles ──

const bar: CSSProperties = {
  height: 32,
  display: "flex",
  alignItems: "center",
  paddingLeft: 8,
  background: "#111118",
  borderBottom: "1px solid rgba(255,255,255,0.06)",
  flexShrink: 0,
};

const menuBtn: CSSProperties = {
  height: 28,
  padding: "0 10px",
  border: "none",
  borderRadius: 4,
  fontSize: 12,
  fontWeight: 500,
  fontFamily: "inherit",
  cursor: "pointer",
  transition: "background 0.1s, color 0.1s",
};

const dropdown: CSSProperties = {
  position: "absolute",
  top: "100%",
  left: 0,
  minWidth: 220,
  background: "#16161e",
  border: "1px solid rgba(255,255,255,0.08)",
  borderRadius: 8,
  boxShadow: "0 8px 32px rgba(0,0,0,0.5)",
  padding: "4px 0",
  zIndex: 9999,
};

const menuItem: CSSProperties = {
  display: "flex",
  alignItems: "center",
  justifyContent: "space-between",
  width: "100%",
  padding: "6px 12px",
  border: "none",
  background: "transparent",
  fontSize: 12,
  fontFamily: "inherit",
  textAlign: "left",
};

const shortcutStyle: CSSProperties = {
  fontSize: 11,
  color: "#5a5a6e",
  marginLeft: 24,
};

const separator: CSSProperties = {
  height: 1,
  margin: "4px 8px",
  background: "rgba(255,255,255,0.06)",
};
