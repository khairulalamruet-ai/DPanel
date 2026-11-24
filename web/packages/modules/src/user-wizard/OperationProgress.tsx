import React from "react";
import { Badge, Button, Card, CardContent } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import {
  CheckCircle2,
  Loader2,
  Server,
  Box,
  HardDrive,
  ShieldCheck,
  ExternalLink,
  Users,
  RotateCcw,
} from "lucide-react";
import { OperationStep, ContainerInfo } from "./types";

export interface OperationProgressProps {
  steps: OperationStep[];
  isComplete: boolean;
  containerInfo?: ContainerInfo | null;
  username: string;
  onReset: () => void;
  onNavigateUsers: () => void;
  onImpersonate?: () => void;
}

export function OperationProgress({
  steps,
  isComplete,
  containerInfo,
  username,
  onReset,
  onNavigateUsers,
  onImpersonate,
}: OperationProgressProps) {
  const { t } = useTranslation();

  return (
    <div className="space-y-6 animate-in fade-in-50">
      {/* Header status */}
      <div className="text-center space-y-2">
        <div className="inline-flex items-center justify-center h-12 w-12 rounded-full bg-primary/10 text-primary mb-1">
          {isComplete ? (
            <CheckCircle2 className="h-7 w-7 text-emerald-500 animate-in zoom-in-75 duration-300" />
          ) : (
            <Loader2 className="h-7 w-7 animate-spin text-primary" />
          )}
        </div>
        <h2 className="text-xl font-bold tracking-tight text-foreground">
          {isComplete
            ? t("userWizard.progress.completedTitle", { username })
            : t("userWizard.progress.inProgressTitle", { username })}
        </h2>
        <p className="text-sm text-muted-foreground max-w-md mx-auto">
          {isComplete
            ? t("userWizard.progress.completedDesc")
            : t("userWizard.progress.inProgressDesc")}
        </p>
      </div>

      {/* Operation Steps Card */}
      <Card className="border bg-card shadow-sm">
        <CardContent className="p-5 space-y-4">
          <div className="space-y-3">
            {steps.map((step, idx) => {
              const isRunning = step.status === "running";
              const isSucceeded = step.status === "succeeded";
              const isPending = step.status === "pending";

              return (
                <div
                  key={step.id}
                  className={`flex items-start gap-3 p-3 rounded-lg border transition-all ${
                    isSucceeded
                      ? "bg-emerald-500/5 border-emerald-500/20"
                      : isRunning
                      ? "bg-primary/5 border-primary/30"
                      : "bg-muted/20 border-muted opacity-60"
                  }`}
                >
                  <div className="mt-0.5 shrink-0">
                    {isSucceeded ? (
                      <CheckCircle2 className="h-5 w-5 text-emerald-500" />
                    ) : isRunning ? (
                      <Loader2 className="h-5 w-5 animate-spin text-primary" />
                    ) : (
                      <div className="h-5 w-5 rounded-full border border-muted-foreground/40 flex items-center justify-center text-[10px] text-muted-foreground font-semibold">
                        {idx + 1}
                      </div>
                    )}
                  </div>

                  <div className="flex-1 min-w-0">
                    <div className="flex items-center justify-between">
                      <p
                        className={`text-sm font-semibold ${
                          isSucceeded
                            ? "text-foreground"
                            : isRunning
                            ? "text-primary"
                            : "text-muted-foreground"
                        }`}
                      >
                        {step.title}
                      </p>
                      {isRunning && (
                        <Badge variant="outline" className="text-[10px] uppercase font-mono animate-pulse">
                          Running
                        </Badge>
                      )}
                      {isSucceeded && (
                        <Badge
                          variant="default"
                          className="text-[10px] bg-emerald-500/20 text-emerald-600 dark:text-emerald-400 border-emerald-500/30"
                        >
                          OK
                        </Badge>
                      )}
                    </div>
                    <p className="text-xs text-muted-foreground mt-0.5">{step.description}</p>
                  </div>
                </div>
              );
            })}
          </div>

          {/* Container Status Showcase (DoD: container চলছে) */}
          {containerInfo && (
            <div className="mt-4 p-4 rounded-xl border bg-card/60 space-y-3 border-emerald-500/30">
              <div className="flex items-center justify-between">
                <div className="flex items-center gap-2">
                  <Box className="h-5 w-5 text-emerald-500" />
                  <span className="text-sm font-semibold text-foreground">
                    {t("userWizard.progress.containerTitle")}
                  </span>
                </div>
                {/* DoD Indicator: container running */}
                <div className="inline-flex items-center gap-2 px-2.5 py-1 rounded-full bg-emerald-500/10 border border-emerald-500/30 text-emerald-600 dark:text-emerald-400 text-xs font-semibold">
                  <span className="relative flex h-2 w-2">
                    <span className="animate-ping absolute inline-flex h-full w-full rounded-full bg-emerald-400 opacity-75" />
                    <span className="relative inline-flex rounded-full h-2 w-2 bg-emerald-500" />
                  </span>
                  <span>{t("userWizard.progress.containerRunning")}</span>
                </div>
              </div>

              <div className="grid grid-cols-2 md:grid-cols-4 gap-2 text-xs font-mono pt-1 border-t">
                <div>
                  <span className="text-muted-foreground block text-[11px]">Container Name</span>
                  <span className="font-semibold text-foreground truncate block">
                    {containerInfo.name}
                  </span>
                </div>
                <div>
                  <span className="text-muted-foreground block text-[11px]">Stack Image</span>
                  <span className="font-semibold text-foreground truncate block">
                    {containerInfo.image}
                  </span>
                </div>
                <div>
                  <span className="text-muted-foreground block text-[11px]">IP Address</span>
                  <span className="font-semibold text-foreground">{containerInfo.ip_address}</span>
                </div>
                <div>
                  <span className="text-muted-foreground block text-[11px]">Security Mode</span>
                  <span className="font-semibold text-emerald-600 dark:text-emerald-400 flex items-center gap-1">
                    <ShieldCheck className="h-3.5 w-3.5" /> Hardened Rootless
                  </span>
                </div>
              </div>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Completion Action Buttons */}
      {isComplete && (
        <div className="flex flex-wrap items-center justify-center gap-3 pt-2">
          {onImpersonate && (
            <Button onClick={onImpersonate} className="gap-2">
              <ExternalLink className="h-4 w-4" />
              {t("userWizard.progress.loginAsUser")}
            </Button>
          )}
          <Button variant="outline" onClick={onNavigateUsers} className="gap-2">
            <Users className="h-4 w-4" />
            {t("userWizard.progress.viewUsersList")}
          </Button>
          <Button variant="ghost" onClick={onReset} className="gap-2 text-muted-foreground">
            <RotateCcw className="h-4 w-4" />
            {t("userWizard.progress.createAnother")}
          </Button>
        </div>
      )}
    </div>
  );
}
