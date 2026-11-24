import React from "react";
import { Button, Card, CardHeader, CardTitle, CardDescription, CardContent } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Lock, Home, Sparkles } from "lucide-react";

export interface FeatureRequiredPageProps {
  feature?: string;
  onNavigateHome?: () => void;
}

export function FeatureRequiredPage({ feature, onNavigateHome }: FeatureRequiredPageProps) {
  const { t } = useTranslation();

  const handleHomeClick = () => {
    if (onNavigateHome) {
      onNavigateHome();
    } else {
      window.location.href = "/";
    }
  };

  return (
    <div className="flex min-h-[60vh] items-center justify-center p-4">
      <Card className="w-full max-w-md text-center border-dashed">
        <CardHeader className="flex flex-col items-center space-y-3 pb-4">
          <div className="flex h-16 w-16 items-center justify-center rounded-full bg-amber-500/10 text-amber-500">
            <Lock className="h-8 w-8" />
          </div>
          <CardTitle className="text-xl font-bold tracking-tight">
            {t("common.featureDisabled")}
          </CardTitle>
          <CardDescription className="text-sm text-muted-foreground">
            {feature
              ? `The feature "${feature}" is not enabled on your current subscription plan.`
              : t("common.featureDisabledDesc") || "Upgrade your hosting plan to gain access to this feature and unlock higher limits."}
          </CardDescription>
        </CardHeader>
        <CardContent className="pt-2 flex flex-col sm:flex-row gap-2 justify-center">
          <Button onClick={handleHomeClick} variant="outline" className="gap-2 text-xs">
            <Home className="h-4 w-4" />
            {t("common.backHome")}
          </Button>
          <Button
            onClick={() => {
              window.open("https://builtpanel.com/pricing", "_blank");
            }}
            className="gap-2 text-xs"
          >
            <Sparkles className="h-4 w-4 text-amber-300" />
            {t("common.upgradePlan") || "Upgrade Plan"}
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
