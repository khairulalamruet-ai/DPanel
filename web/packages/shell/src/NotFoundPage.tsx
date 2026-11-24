import React from "react";
import { Button, Card, CardHeader, CardTitle, CardDescription, CardContent } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { FileQuestion, Home } from "lucide-react";

export interface NotFoundPageProps {
  onNavigateHome?: () => void;
}

export function NotFoundPage({ onNavigateHome }: NotFoundPageProps) {
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
          <div className="flex h-16 w-16 items-center justify-center rounded-full bg-muted">
            <FileQuestion className="h-8 w-8 text-muted-foreground" />
          </div>
          <CardTitle className="text-2xl font-bold tracking-tight">
            {t("common.notFoundTitle")}
          </CardTitle>
          <CardDescription className="text-sm text-muted-foreground">
            {t("common.notFoundDesc")}
          </CardDescription>
        </CardHeader>
        <CardContent className="pt-2">
          <Button onClick={handleHomeClick} className="gap-2">
            <Home className="h-4 w-4" />
            {t("common.backHome")}
          </Button>
        </CardContent>
      </Card>
    </div>
  );
}
