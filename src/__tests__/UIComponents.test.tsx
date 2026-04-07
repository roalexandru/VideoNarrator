import { describe, it, expect, beforeEach, afterEach, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { clearMocks } from "@tauri-apps/api/mocks";
import { setupDefaultMocks, resetAllStores } from "./setup";
import { Button } from "../components/ui/Button";
import { ConfirmDialog } from "../components/ui/ConfirmDialog";
import { ProgressBar } from "../components/ui/ProgressBar";
import { ErrorCard } from "../components/ui/ErrorCard";
import { Card } from "../components/ui/Card";

describe("Button", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });

  afterEach(() => {
    clearMocks();
  });

  it("renders with label", () => {
    render(<Button>Click Me</Button>);
    expect(screen.getByText("Click Me")).toBeInTheDocument();
  });

  it("handles click", async () => {
    const onClick = vi.fn();
    const user = userEvent.setup();

    render(<Button onClick={onClick}>Click Me</Button>);
    await user.click(screen.getByText("Click Me"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("can be disabled", async () => {
    const onClick = vi.fn();
    const user = userEvent.setup();

    render(<Button onClick={onClick} disabled>Disabled</Button>);

    const button = screen.getByText("Disabled");
    expect(button).toBeDisabled();

    await user.click(button);
    expect(onClick).not.toHaveBeenCalled();
  });

  it("renders with different variants", () => {
    const { rerender } = render(<Button variant="primary">Primary</Button>);
    expect(screen.getByText("Primary")).toBeInTheDocument();

    rerender(<Button variant="secondary">Secondary</Button>);
    expect(screen.getByText("Secondary")).toBeInTheDocument();

    rerender(<Button variant="ghost">Ghost</Button>);
    expect(screen.getByText("Ghost")).toBeInTheDocument();

    rerender(<Button variant="danger">Danger</Button>);
    expect(screen.getByText("Danger")).toBeInTheDocument();
  });

  it("renders with different sizes", () => {
    const { rerender } = render(<Button size="sm">Small</Button>);
    expect(screen.getByText("Small")).toBeInTheDocument();

    rerender(<Button size="md">Medium</Button>);
    expect(screen.getByText("Medium")).toBeInTheDocument();

    rerender(<Button size="lg">Large</Button>);
    expect(screen.getByText("Large")).toBeInTheDocument();
  });
});

describe("ConfirmDialog", () => {
  beforeEach(() => {
    resetAllStores();
    setupDefaultMocks();
  });

  afterEach(() => {
    clearMocks();
  });

  it("shows title and message", () => {
    render(
      <ConfirmDialog
        title="Confirm Action"
        message="Are you sure you want to proceed?"
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    );

    expect(screen.getByText("Confirm Action")).toBeInTheDocument();
    expect(screen.getByText("Are you sure you want to proceed?")).toBeInTheDocument();
  });

  it("confirm callback works", async () => {
    const onConfirm = vi.fn();
    const user = userEvent.setup();

    render(
      <ConfirmDialog
        title="Confirm"
        message="Proceed?"
        confirmLabel="Yes"
        onConfirm={onConfirm}
        onCancel={vi.fn()}
      />
    );

    await user.click(screen.getByText("Yes"));
    expect(onConfirm).toHaveBeenCalledTimes(1);
  });

  it("cancel callback works", async () => {
    const onCancel = vi.fn();
    const user = userEvent.setup();

    render(
      <ConfirmDialog
        title="Confirm"
        message="Proceed?"
        onConfirm={vi.fn()}
        onCancel={onCancel}
      />
    );

    await user.click(screen.getByText("Cancel"));
    expect(onCancel).toHaveBeenCalledTimes(1);
  });

  it("uses default confirm label 'Delete'", () => {
    render(
      <ConfirmDialog
        title="Delete Item"
        message="This cannot be undone."
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    );

    expect(screen.getByText("Delete")).toBeInTheDocument();
  });

  it("uses custom confirm and cancel labels", () => {
    render(
      <ConfirmDialog
        title="Custom Labels"
        message="Test custom labels"
        confirmLabel="Proceed"
        cancelLabel="Go Back"
        onConfirm={vi.fn()}
        onCancel={vi.fn()}
      />
    );

    expect(screen.getByText("Proceed")).toBeInTheDocument();
    expect(screen.getByText("Go Back")).toBeInTheDocument();
  });
});

describe("ProgressBar", () => {
  function getInnerBar(container: HTMLElement): HTMLDivElement {
    // The ProgressBar renders: <div (outer)><div (inner fill)/></div>
    const outer = container.firstChild as HTMLDivElement;
    return outer.firstChild as HTMLDivElement;
  }

  it("renders with 0%", () => {
    const { container } = render(<ProgressBar value={0} />);
    const inner = getInnerBar(container);
    expect(inner.style.width).toBe("0%");
  });

  it("renders with 50%", () => {
    const { container } = render(<ProgressBar value={50} />);
    const inner = getInnerBar(container);
    expect(inner.style.width).toBe("50%");
  });

  it("renders with 100%", () => {
    const { container } = render(<ProgressBar value={100} />);
    const inner = getInnerBar(container);
    expect(inner.style.width).toBe("100%");
  });

  it("clamps values above 100 to 100%", () => {
    const { container } = render(<ProgressBar value={150} />);
    const inner = getInnerBar(container);
    expect(inner.style.width).toBe("100%");
  });

  it("clamps negative values to 0%", () => {
    const { container } = render(<ProgressBar value={-20} />);
    const inner = getInnerBar(container);
    expect(inner.style.width).toBe("0%");
  });

  it("accepts custom height", () => {
    const { container } = render(<ProgressBar value={50} height={10} />);
    const outer = container.firstChild as HTMLDivElement;
    expect(outer.style.height).toBe("10px");
  });
});

describe("ErrorCard", () => {
  it("renders error message", () => {
    render(<ErrorCard error="Something failed" />);
    expect(screen.getByText("Something failed")).toBeInTheDocument();
  });

  it("renders suggestion when provided", () => {
    render(<ErrorCard error="Failed" suggestion="Try again later" />);
    expect(screen.getByText("Try again later")).toBeInTheDocument();
  });

  it("renders action button when actionLabel and onAction provided", async () => {
    const onAction = vi.fn();
    const user = userEvent.setup();

    render(<ErrorCard error="Failed" actionLabel="Retry" onAction={onAction} />);

    const retryButton = screen.getByText("Retry");
    expect(retryButton).toBeInTheDocument();

    await user.click(retryButton);
    expect(onAction).toHaveBeenCalledTimes(1);
  });

  it("renders dismiss button when onDismiss provided", async () => {
    const onDismiss = vi.fn();
    const user = userEvent.setup();

    render(<ErrorCard error="Failed" onDismiss={onDismiss} />);

    // The dismiss button uses the × character
    const dismissButton = screen.getByText("\u00d7");
    await user.click(dismissButton);
    expect(onDismiss).toHaveBeenCalledTimes(1);
  });
});

describe("Card", () => {
  it("renders children", () => {
    render(<Card>Card content</Card>);
    expect(screen.getByText("Card content")).toBeInTheDocument();
  });

  it("handles click when onClick provided", async () => {
    const onClick = vi.fn();
    const user = userEvent.setup();

    render(<Card onClick={onClick}>Clickable Card</Card>);

    await user.click(screen.getByText("Clickable Card"));
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it("renders with selected styling", () => {
    const { container } = render(<Card selected>Selected Card</Card>);
    const card = container.firstChild as HTMLDivElement;
    // jsdom normalizes color values with spaces: rgba(99, 102, 241, 0.5)
    expect(card.style.border).toContain("rgba(99, 102, 241, 0.5)");
  });
});
