import { Component, type ReactNode } from "react";

interface Props { children: ReactNode; fallback?: ReactNode; }
interface State { hasError: boolean; error: string; }

export class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: "" };

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error: error.message };
  }

  componentDidCatch(error: Error, info: React.ErrorInfo) {
    console.error("ErrorBoundary caught:", error, info.componentStack);
  }

  render() {
    if (this.state.hasError) {
      return this.props.fallback || (
        <div style={{ padding: 40, textAlign: "center", color: "#e0e0ea" }}>
          <h3 style={{ fontSize: 18, fontWeight: 600, marginBottom: 8 }}>Something went wrong</h3>
          <p style={{ color: "#8b8ba0", fontSize: 14, marginBottom: 16 }}>{this.state.error}</p>
          <button onClick={() => { this.setState({ hasError: false, error: "" }); window.location.reload(); }}
            style={{ padding: "8px 20px", borderRadius: 8, border: "none", background: "rgba(99,102,241,0.15)", color: "#818cf8", fontSize: 14, fontWeight: 600, cursor: "pointer" }}>
            Try Again
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
