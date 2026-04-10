import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { ProjectLibrary } from "../features/projects/ProjectLibrary";

describe("ProjectLibrary", () => {
  const onNewProject = vi.fn();
  const onOpenProject = vi.fn();
  const onOpenSettings = vi.fn();

  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
    onNewProject.mockClear();
    onOpenProject.mockClear();
    onOpenSettings.mockClear();
  });

  afterEach(() => {
    clearMocks();
  });

  it("renders project list from IPC", async () => {
    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("Demo Project")).toBeInTheDocument();
    });
  });

  it("shows project title and metadata", async () => {
    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("Demo Project")).toBeInTheDocument();
    });

    // Style badge
    expect(screen.getByText("Product Demo")).toBeInTheDocument();
    // Language badge
    expect(screen.getByText("EN")).toBeInTheDocument();
    // Script status badge
    expect(screen.getByText("READY")).toBeInTheDocument();
  });

  it("shows project count", async () => {
    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("1 project")).toBeInTheDocument();
    });
  });

  it("delete button shows confirmation dialog", async () => {
    const user = userEvent.setup();

    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("Demo Project")).toBeInTheDocument();
    });

    const deleteButton = screen.getByText("Delete");
    await user.click(deleteButton);

    await waitFor(() => {
      expect(screen.getByText("Delete Project")).toBeInTheDocument();
      expect(screen.getByText(/Are you sure you want to delete "Demo Project"/)).toBeInTheDocument();
    });
  });

  it("confirmation dialog cancel closes it", async () => {
    const user = userEvent.setup();

    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("Demo Project")).toBeInTheDocument();
    });

    await user.click(screen.getByText("Delete"));

    await waitFor(() => {
      expect(screen.getByText("Delete Project")).toBeInTheDocument();
    });

    await user.click(screen.getByText("Cancel"));

    await waitFor(() => {
      expect(screen.queryByText("Delete Project")).not.toBeInTheDocument();
    });
  });

  it("empty state when no projects", async () => {
    clearMocks();
    mockIPC((cmd) => {
      switch (cmd) {
        case "list_projects":
          return [];
        default:
          return null;
      }
    });

    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("No projects yet")).toBeInTheDocument();
    });

    expect(screen.getByText("Create First Project")).toBeInTheDocument();
    expect(screen.getByText(/Create a new project to start generating/)).toBeInTheDocument();
  });

  it("handles loading state", () => {
    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    // While the IPC call is pending, the loading indicator shows
    expect(screen.getByText("Loading...")).toBeInTheDocument();
  });

  it("clicking New Project button calls onNewProject", async () => {
    const user = userEvent.setup();

    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("Demo Project")).toBeInTheDocument();
    });

    await user.click(screen.getByText("New Project"));
    expect(onNewProject).toHaveBeenCalledTimes(1);
  });

  it("clicking a project card calls onOpenProject with the project id", async () => {
    const user = userEvent.setup();

    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("Demo Project")).toBeInTheDocument();
    });

    await user.click(screen.getByText("Demo Project"));
    expect(onOpenProject).toHaveBeenCalledWith("proj-1");
  });

  it("renders Import button in the New Project card", async () => {
    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("Demo Project")).toBeInTheDocument();
    });

    expect(screen.getByText("Import .narrator")).toBeInTheDocument();
  });

  it("shows Export button on project card hover", async () => {
    render(
      <ProjectLibrary
        onNewProject={onNewProject}
        onOpenProject={onOpenProject}
        onOpenSettings={onOpenSettings}
      />
    );

    await waitFor(() => {
      expect(screen.getByText("Demo Project")).toBeInTheDocument();
    });

    // Export button exists in DOM but is hidden (opacity: 0)
    expect(screen.getByText("Export")).toBeInTheDocument();
  });
});
