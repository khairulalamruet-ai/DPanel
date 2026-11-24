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
  Globe,
  Folder,
  Database,
  ShieldCheck,
  ChevronRight,
  ChevronLeft,
  X,
  Compass,
  Check,
} from "lucide-react";
import { ClientTourStep } from "./types";

const STORAGE_DISMISS_KEY = "builtpanel-client-tour-dismissed";

const CLIENT_TOUR_STEPS: ClientTourStep[] = [
  {
    id: "tour_domains",
    titleKey: "onboarding.client.tour.domains.title",
    defaultTitle: "1. Websites & Domain Management",
    descKey: "onboarding.client.tour.domains.desc",
    defaultDesc: "Deploy new websites, map subdomains, customize PHP runtimes, and inspect document root paths.",
    actionUrl: "/domains",
    actionKey: "onboarding.client.tour.domains.action",
    defaultAction: "Manage Domains",
  },
  {
    id: "tour_files",
    titleKey: "onboarding.client.tour.files.title",
    defaultTitle: "2. Web File Manager & FTP Access",
    descKey: "onboarding.client.tour.files.desc",
    defaultDesc: "Upload archive packages, edit code in browser, extract archives, or configure secure FTP credentials.",
    actionUrl: "/files",
    actionKey: "onboarding.client.tour.files.action",
    defaultAction: "Open File Manager",
  },
  {
    id: "tour_databases",
    titleKey: "onboarding.client.tour.databases.title",
    defaultTitle: "3. MariaDB Databases & phpMyAdmin",
    descKey: "onboarding.client.tour.databases.desc",
    defaultDesc: "Create database schemas, grant user privileges, and access phpMyAdmin with single-sign-on.",
    actionUrl: "/databases",
    actionKey: "onboarding.client.tour.databases.action",
    defaultAction: "View Databases",
  },
  {
    id: "tour_security",
    titleKey: "onboarding.client.tour.security.title",
    defaultTitle: "4. SSL Certificates & WAF Protection",
    descKey: "onboarding.client.tour.security.desc",
    defaultDesc: "Issue free auto-renewing Let's Encrypt certificates, enforce HTTPS redirects, and monitor WAF blocks.",
    actionUrl: "/ssl",
    actionKey: "onboarding.client.tour.security.action",
    defaultAction: "Security & SSL",
  },
];

const TOUR_ICONS: Record<string, React.ReactNode> = {
  tour_domains: <Globe className="h-6 w-6 text-primary" />,
  tour_files: <Folder className="h-6 w-6 text-amber-500" />,
  tour_databases: <Database className="h-6 w-6 text-emerald-500" />,
  tour_security: <ShieldCheck className="h-6 w-6 text-indigo-500" />,
};

interface ClientWelcomeTourProps {
  onDismissCallback?: () => void;
}

export function ClientWelcomeTour({ onDismissCallback }: ClientWelcomeTourProps) {
  const { t } = useTranslation();

  const [isDismissed, setIsDismissed] = useState<boolean>(() => {
    try {
      return localStorage.getItem(STORAGE_DISMISS_KEY) === "true";
    } catch {
      return false;
    }
  });

  const [currentStepIndex, setCurrentStepIndex] = useState(0);

  useEffect(() => {
    if (isDismissed) return;
    let isMounted = true;
    apiFetch<{ dismissed: boolean }>("/api/v1/onboarding/client")
      .then((res) => {
        if (!isMounted) return;
        if (res.dismissed) {
          setIsDismissed(true);
          try {
            localStorage.setItem(STORAGE_DISMISS_KEY, "true");
          } catch {
            // Ignore
          }
        }
      })
      .catch(() => {});
    return () => {
      isMounted = false;
    };
  }, [isDismissed]);

  const handleDismiss = async () => {
    setIsDismissed(true);
    try {
      localStorage.setItem(STORAGE_DISMISS_KEY, "true");
    } catch {
      // Ignore
    }
    if (onDismissCallback) onDismissCallback();

    try {
      await apiFetch("/api/v1/onboarding/client/dismiss", { method: "POST" });
    } catch {
      // Ignore
    }
  };

  if (isDismissed) {
    return null;
  }

  const currentStep = CLIENT_TOUR_STEPS[currentStepIndex];
  const isLast = currentStepIndex === CLIENT_TOUR_STEPS.length - 1;

  return (
    <Card
      className="border-primary/30 bg-gradient-to-r from-primary/10 via-background to-background relative shadow-sm overflow-hidden"
      data-testid="client-welcome-tour-card"
    >
      <CardHeader className="pb-3 flex flex-row items-start justify-between space-y-0">
        <div className="space-y-1">
          <div className="flex items-center gap-2">
            <Compass className="h-4 w-4 text-primary animate-pulse" />
            <CardTitle className="text-base font-semibold" data-testid="client-tour-title">
              {t("onboarding.client.tour.title", "Welcome to Your Built Panel Account")}
            </CardTitle>
            <Badge variant="outline" className="text-xs font-mono ml-2">
              {currentStepIndex + 1} / {CLIENT_TOUR_STEPS.length}
            </Badge>
          </div>
          <CardDescription className="text-xs">
            {t(
              "onboarding.client.tour.subtitle",
              "Take a quick 1-minute walkthrough of the key features of your hosting account."
            )}
          </CardDescription>
        </div>

        <Button
          variant="ghost"
          size="sm"
          onClick={handleDismiss}
          className="h-8 w-8 p-0 text-muted-foreground hover:text-foreground"
          data-testid="btn-dismiss-client-tour"
          title={t("onboarding.client.tour.dismiss", "Dismiss Tour")}
          aria-label={t("onboarding.client.tour.dismiss", "Dismiss Tour")}
        >
          <X className="h-4 w-4" />
        </Button>
      </CardHeader>

      <CardContent className="space-y-4">
        {/* Active Step Showcase */}
        <div className="flex flex-col sm:flex-row items-start sm:items-center gap-4 p-4 rounded-lg border bg-card/60 shadow-xs">
          <div className="p-3 rounded-xl bg-primary/10 border border-primary/20 shrink-0">
            {TOUR_ICONS[currentStep.id] || <Compass className="h-6 w-6 text-primary" />}
          </div>

          <div className="space-y-1 flex-1 min-w-0">
            <h4 className="text-sm font-semibold text-foreground">
              {t(currentStep.titleKey, currentStep.defaultTitle)}
            </h4>
            <p className="text-xs text-muted-foreground leading-relaxed">
              {t(currentStep.descKey, currentStep.defaultDesc)}
            </p>
          </div>

          {currentStep.actionUrl && (
            <a
              href={currentStep.actionUrl}
              className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md text-xs font-medium bg-secondary text-secondary-foreground hover:bg-secondary/80 transition-colors shrink-0"
              data-testid={`btn-tour-goto-${currentStep.id}`}
            >
              <span>{t(currentStep.actionKey || "", currentStep.defaultAction || "Explore")}</span>
              <ChevronRight className="h-3.5 w-3.5" />
            </a>
          )}
        </div>

        {/* Step dots and navigation buttons */}
        <div className="flex items-center justify-between pt-1">
          {/* Dots */}
          <div className="flex items-center gap-1.5">
            {CLIENT_TOUR_STEPS.map((step, idx) => (
              <button
                key={step.id}
                type="button"
                onClick={() => setCurrentStepIndex(idx)}
                className={`h-2 rounded-full transition-all cursor-pointer ${
                  idx === currentStepIndex
                    ? "w-6 bg-primary"
                    : "w-2 bg-muted-foreground/30 hover:bg-muted-foreground/50"
                }`}
                aria-label={`Jump to step ${idx + 1}`}
              />
            ))}
          </div>

          {/* Navigation controls */}
          <div className="flex items-center gap-2">
            <Button
              variant="outline"
              size="sm"
              disabled={currentStepIndex === 0}
              onClick={() => setCurrentStepIndex((prev) => Math.max(0, prev - 1))}
              className="h-8 px-2.5 text-xs gap-1"
              data-testid="btn-tour-prev"
            >
              <ChevronLeft className="h-3.5 w-3.5" />
              <span>{t("common.prev", "Previous")}</span>
            </Button>

            {isLast ? (
              <Button
                variant="default"
                size="sm"
                onClick={handleDismiss}
                className="h-8 px-3 text-xs gap-1"
                data-testid="btn-tour-finish"
              >
                <Check className="h-3.5 w-3.5" />
                <span>{t("onboarding.client.tour.finish", "Finish Tour")}</span>
              </Button>
            ) : (
              <Button
                variant="default"
                size="sm"
                onClick={() =>
                  setCurrentStepIndex((prev) =>
                    Math.min(CLIENT_TOUR_STEPS.length - 1, prev + 1)
                  )
                }
                className="h-8 px-3 text-xs gap-1"
                data-testid="btn-tour-next"
              >
                <span>{t("common.next", "Next")}</span>
                <ChevronRight className="h-3.5 w-3.5" />
              </Button>
            )}
          </div>
        </div>
      </CardContent>
    </Card>
  );
}
