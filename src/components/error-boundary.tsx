import { Component, type ErrorInfo, type ReactNode } from "react";
import { Button } from "@/components/ui/button";

type Props = { children: ReactNode };
type State = { error: Error | null };

/** Catch render crashes so the window is never a silent white screen. */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null };

  static getDerivedStateFromError(error: Error): State {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error("UI crash:", error, info.componentStack);
  }

  render() {
    if (this.state.error) {
      return (
        <div className="flex h-full min-h-[240px] flex-col items-center justify-center gap-3 p-6 text-center">
          <div className="text-base font-semibold">界面出错了</div>
          <pre className="max-w-lg whitespace-pre-wrap break-all rounded-md border border-red-200 bg-red-50 p-3 text-left text-xs text-red-800">
            {this.state.error.message}
          </pre>
          <Button
            onClick={() => {
              this.setState({ error: null });
              window.location.hash = "";
              window.location.reload();
            }}
          >
            重新加载
          </Button>
        </div>
      );
    }
    return this.props.children;
  }
}
