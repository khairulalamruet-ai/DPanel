import React, { Component, ErrorInfo, ReactNode } from "react";
import { Button, Card, CardHeader, CardTitle, CardDescription, CardContent } from "@builtpanel/ui";
import { AlertTriangle, RotateCcw } from "lucide-react";

export function RouteErrorFallback({ error, reset }: { error: any; reset?: () => void }) {
  const errorMessage = error instanceof Error ? error.message : String(error || "Unknown error");

  return (
    <div className="flex min-h-[60vh] items-center justify-center p-4">
      <Card className="w-full max-w-lg border-destructive/50">
        <CardHeader className="flex flex-col items-center space-y-3 pb-4 text-center">
          <div className="flex h-14 w-14 items-center justify-center rounded-full bg-destructive/10 text-destructive">
            <AlertTriangle className="h-7 w-7" />
          </div>
          <CardTitle className="text-xl font-bold">Something went wrong</CardTitle>
          <CardDescription>
            An unexpected error occurred while rendering this route.
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="rounded-md bg-muted p-3 font-mono text-xs text-muted-foreground overflow-auto max-h-32">
            {errorMessage}
          </div>
          <div className="flex justify-center gap-3">
            <Button variant="outline" onClick={() => window.location.reload()}>
              Reload page
            </Button>
            {reset && (
              <Button onClick={reset} className="gap-2">
                <RotateCcw className="h-4 w-4" />
                Try again
              </Button>
            )}
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

export interface ErrorBoundaryProps {
  children: ReactNode;
  fallback?: ReactNode;
  onReset?: () => void;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export class ErrorBoundary extends Component<ErrorBoundaryProps, State> {
  public state: State = {
    hasError: false,
    error: null,
  };

  public static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  public componentDidCatch(error: Error, errorInfo: ErrorInfo) {
    console.error("ErrorBoundary caught an error:", error, errorInfo);
  }

  public reset = () => {
    this.setState({ hasError: false, error: null });
    if (this.props.onReset) {
      this.props.onReset();
    }
  };

  public render() {
    if (this.state.hasError) {
      if (this.props.fallback) {
        return this.props.fallback;
      }

      return (
        <RouteErrorFallback
          error={this.state.error || new Error("Unknown error")}
          reset={this.reset}
        />
      );
    }

    return this.props.children;
  }
}
