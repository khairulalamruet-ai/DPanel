import React, { useState } from "react";
import { Card, CardHeader, CardTitle, CardDescription, CardContent, Button, Badge } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { useSession } from "@builtpanel/auth";
import { FeatureRequiredPage } from "@builtpanel/shell";
import { Puzzle, CheckCircle2, Zap } from "lucide-react";

export function DummyModuleContent() {
  const { t } = useTranslation();
  const [clickCount, setClickCount] = useState(0);

  return (
    <div className="space-y-6">
      <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4">
        <div>
          <div className="flex items-center gap-2">
            <h1 className="text-2xl font-bold tracking-tight">{t("dummy.title")}</h1>
            <Badge variant="outline" className="text-xs text-primary border-primary">
              {t("dummy.badge")}
            </Badge>
          </div>
          <p className="text-sm text-muted-foreground">{t("dummy.desc")}</p>
        </div>
      </div>

      <Card>
        <CardHeader>
          <CardTitle className="text-base flex items-center gap-2">
            <Puzzle className="h-4 w-4 text-primary" />
            {t("dummy.status")}
          </CardTitle>
          <CardDescription>
            {t("dummy.activeNotice")}
          </CardDescription>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="flex items-center gap-2 text-sm text-emerald-500 font-medium">
            <CheckCircle2 className="h-5 w-5" />
            <span>Module registry verification: Active and Verified</span>
          </div>

          <div className="pt-2">
            <Button
              size="sm"
              onClick={() => setClickCount((prev) => prev + 1)}
              className="gap-2 text-xs"
            >
              <Zap className="h-4 w-4" />
              Interactive Action ({clickCount})
            </Button>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}

export function DummyModulePage() {
  const { data: session } = useSession();
  const isModuleEnabled = session?.modules?.includes("dummy");
  const isFeatureEnabled = session?.features?.includes("all")
    ? isModuleEnabled
    : (session?.features?.includes("dummy_feature") && isModuleEnabled);

  if (!isFeatureEnabled) {
    return <FeatureRequiredPage feature="dummy_feature" />;
  }

  return <DummyModuleContent />;
}
