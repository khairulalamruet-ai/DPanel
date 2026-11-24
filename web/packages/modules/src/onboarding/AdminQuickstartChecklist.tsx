import React, { useState, useEffect } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  CheckCircle2,
  Circle,
  X,
  Server,
  Layers,
  Globe,
  Mail,
  ArrowRight,
  Sparkles,
} from "lucide-react";
import { AdminQuickstartStep, AdminQuickstartStepId } from "./types";

const STORAGE_DISMISS_KEY = "builtpanel-admin-onboarding-dismissed";
const STORAGE_COMPLETED_KEY = "builtpanel-admin-onboarding-completed-steps";

const INITIAL_STEPS: AdminQuickstartStep[] = [
  {
    id: "add_node",
    titleKey: "onboarding.admin.steps.addNode.title",
    defaultTitle: "Add First Managed Node",
    descKey: "onboarding.admin.steps.addNode.desc",
    defaultDesc: "Enroll a Linux worker node using the secure one-line node-agent bootstrap token.",
    actionUrl: "/nodes",
    actionKey: "onboarding.admin.steps.addNode.action",
    defaultAction: "Enroll Node",
    completed: true, // Default 1 node already enrolled in fresh install
  },
  {
    id: "create_plan",
    titleKey: "onboarding.admin.steps.createPlan.title",
    defaultTitle: "Create Hosting Plan",
    descKey: "onboarding.admin.steps.createPlan.desc",
    defaultDesc: "Define resource quotas, disk limits, and feature sets for your client accounts.",
    actionUrl: "/plans",
    actionKey: "onboarding.admin.steps.createPlan.action",
    defaultAction: "Manage Plans",
    completed: false,
  },
  {
    id: "configure_dns",
    titleKey: "onboarding.admin.steps.configureDns.title",
    defaultTitle: "Configure Primary Nameservers",
    descKey: "onboarding.admin.steps.configureDns.desc",
    defaultDesc: "Set your cluster nameservers (ns1.example.com, ns2.example.com) for automated zone delegation.",
    actionUrl: "/settings",
    actionKey: "onboarding.admin.steps.configureDns.action",
    defaultAction: "Configure DNS",
    completed: false,
  },
  {
    id: "setup_smtp",
    titleKey: "onboarding.admin.steps.setupSmtp.title",
    defaultTitle: "Configure SMTP & Notifications",
    descKey: "onboarding.admin.steps.setupSmtp.desc",
    defaultDesc: "Verify outgoing mail delivery for password resets, SSL warnings, and server health alerts.",
    actionUrl: "/settings",
    actionKey: "onboarding.admin.steps.setupSmtp.action",
    defaultAction: "Setup SMTP",
    completed: false,
  },
];

const STEP_ICONS: Record<AdminQuickstartStepId, React.ReactNode> = {
  add_node: <Server className="h-5 w-5 text-primary" />,
  create_plan: <Layers className="h-5 w-5 text-indigo-500" />,
  configure_dns: <Globe className="h-5 w-5 text-emerald-500" />,
  setup_smtp: <Mail className="h-5 w-5 text-amber-500" />,
};

interface AdminQuickstartChecklistProps {
  onDismissCallback?: () => void;
}

export function AdminQuickstartChecklist({ onDismissCallback }: AdminQuickstartChecklistProps) {
  const { t } = useTranslation();

  const [isDismissed, setIsDismissed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(STORAGE_DISMISS_KEY) === "true";
    } catch {
      return false;
    }
  });

  const [steps, setSteps] = useState<AdminQuickstartStep[]>(() => {
    try {
      const saved = localStorage.getItem(STORAGE_COMPLETED_KEY);
      if (saved) {
        const completedMap = JSON.parse(saved) as Record<string, boolean>;
        return INITIAL_STEPS.map((s) => ({
          ...s,
          completed: completedMap[s.id] ?? s.completed,
        }));
      }
    } catch {
      // Ignore
    }
    return INITIAL_STEPS;
  });

  // Fetch status from API if available
  useEffect(() => {
    if (isDismissed) return;
    let isMounted = true;
    apiFetch<{ dismissed: boolean; completed_steps?: string[] }>("/api/v1/onboarding/admin")
      .then((res) => {
        if (!isMounted) return;
        if (res.dismissed) {
          setIsDismissed(true);
          try {
            localStorage.setItem(STORAGE_DISMISS_KEY, "true");
          } catch {
            // Ignore
          }
        } else if (res.completed_steps) {
          setSteps((prev) =>
            prev.map((s) => ({
              ...s,
              completed: res.completed_steps!.includes(s.id),
            }))
          );
        }
      })
      .catch(() => {
        // Ignore API failure, fallback to localStorage state
      });
    return () => {
      isMounted = false;
    };
  }, [isDismissed]);

  const handleToggleStep = (id: AdminQuickstartStepId) => {
    setSteps((prev) => {
      const updated = prev.map((s) =>
        s.id === id ? { ...s, completed: !s.completed } : s
      );
      try {
        const map: Record<string, boolean> = {};
        updated.forEach((s) => {
          map[s.id] = s.completed;
        });
        localStorage.setItem(STORAGE_COMPLETED_KEY, JSON.stringify(map));
      } catch {
        // Ignore
      }
      return updated;
    });

    // Notify backend
    apiFetch(`/api/v1/onboarding/admin/step/${id}/toggle`, {
      method: "POST",
    }).catch(() => {});
  };

  const handleDismiss = async () => {
    setIsDismissed(true);
    try {
      localStorage.setItem(STORAGE_DISMISS_KEY, "true");
    } catch {
      // Ignore
    }
    if (onDismissCallback) onDismissCallback();

    try {
      await apiFetch("/api/v1/onboarding/admin/dismiss", { method: "POST" });
    } catch {
      // Fallback
    }
  };

  if (isDismissed) {
    return null;
  }

  const completedCount = steps.filter((s) => s.completed).length;
  const progressPercent = Math.round((completedCount / steps.length) * 100);

  return (
    <Card
      className="border-primary/20 bg-gradient-to-r from-primary/5 via-background to-background relative overflow-hidden shadow-sm"
      data-testid="admin-quickstart-card"
    >
      <CardHeader className="pb-3 flex flex-row items-start justify-between space-y-0">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <Sparkles className="h-4 w-4 text-primary" />
            <CardTitle className="text-base font-semibold" data-testid="quickstart-title">
              {t("onboarding.admin.title", "Getting Started with Built Panel")}
            </CardTitle>
            <Badge variant="outline" className="text-xs font-mono ml-2">
              {completedCount} / {steps.length} {t("onboarding.admin.stepsDone", "completed")} ({progressPercent}%)
            </Badge>
          </div>
          <CardDescription className="text-xs">
            {t(
              "onboarding.admin.description",
              "Complete these initial configuration steps to prepare your cluster for tenants and domains."
            )}
          </CardDescription>
        </div>

        <Button
          variant="ghost"
          size="sm"
          onClick={handleDismiss}
          className="h-8 w-8 p-0 text-muted-foreground hover:text-foreground"
          data-testid="btn-dismiss-quickstart"
          title={t("onboarding.admin.dismiss", "Dismiss Quickstart Checklist")}
          aria-label={t("onboarding.admin.dismiss", "Dismiss Quickstart Checklist")}
        >
          <X className="h-4 w-4" />
        </Button>
      </CardHeader>

      <CardContent className="space-y-4">
        {/* Progress Bar */}
        <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
          <div
            className="bg-primary h-1.5 rounded-full transition-all duration-300"
            style={{ width: `${progressPercent}%` }}
            data-testid="quickstart-progress-bar"
          />
        </div>

        {/* Steps Grid */}
        <div className="grid grid-cols-1 md:grid-cols-2 gap-3 pt-1">
          {steps.map((step) => {
            return (
              <div
                key={step.id}
                data-testid={`quickstart-step-${step.id}`}
                className={`flex items-start gap-3 p-3 rounded-lg border transition-all ${
                  step.completed
                    ? "bg-muted/30 border-muted text-muted-foreground opacity-90"
                    : "bg-card border-border hover:border-primary/40 shadow-xs"
                }`}
              >
                <button
                  type="button"
                  onClick={() => handleToggleStep(step.id)}
                  className="mt-0.5 shrink-0 text-muted-foreground hover:text-primary transition-colors cursor-pointer"
                  data-testid={`btn-toggle-step-${step.id}`}
                  aria-label={`Toggle ${step.defaultTitle}`}
                >
                  {step.completed ? (
                    <CheckCircle2 className="h-5 w-5 text-emerald-500 fill-emerald-500/20" />
                  ) : (
                    <Circle className="h-5 w-5" />
                  )}
                </button>

                <div className="flex-1 min-w-0 space-y-1">
                  <div className="flex items-center justify-between gap-2">
                    <span
                      className={`text-xs font-semibold ${
                        step.completed ? "line-through text-muted-foreground" : "text-foreground"
                      }`}
                    >
                      {t(step.titleKey, step.defaultTitle)}
                    </span>
                    <a
                      href={step.actionUrl}
                      className="inline-flex items-center gap-1 text-[11px] font-medium text-primary hover:underline shrink-0"
                      data-testid={`link-step-${step.id}`}
                    >
                      <span>{t(step.actionKey, step.defaultAction)}</span>
                      <ArrowRight className="h-3 w-3" />
                    </a>
                  </div>
                  <p className="text-[11px] text-muted-foreground line-clamp-2 leading-relaxed">
                    {t(step.descKey, step.defaultDesc)}
                  </p>
                </div>
              </div>
            );
          })}
        </div>
      </CardContent>
    </Card>
  );
}
