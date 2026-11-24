import React, { useState } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Badge,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import { setAccountVersion, setDomainVersion } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import { Check, Globe, Cpu } from "lucide-react";
import type { PhpVersion, PhpDomainConfig } from "./types";

interface PhpVersionSelectorProps {
  defaultVersion: string;
  supportedVersions: PhpVersion[];
  domainConfigs: PhpDomainConfig[];
  onConfigUpdated: () => void;
}

export function PhpVersionSelector({
  defaultVersion,
  supportedVersions,
  domainConfigs,
  onConfigUpdated,
}: PhpVersionSelectorProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [selectedDefault, setSelectedDefault] = useState(defaultVersion);
  const [isUpdatingDefault, setIsUpdatingDefault] = useState(false);
  const [updatingDomainId, setUpdatingDomainId] = useState<string | null>(null);

  const handleUpdateDefault = async () => {
    try {
      setIsUpdatingDefault(true);
      await setAccountVersion(selectedDefault);
      toast({
        title: t("common.success", "Success"),
        description: t("php.toast.versionUpdated", "Default PHP version updated to {{version}}", {
          version: selectedDefault,
        }),
      });
      onConfigUpdated();
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("php.errors.versionUpdateFailed", "Failed to update PHP version"),
        variant: "destructive",
      });
    } finally {
      setIsUpdatingDefault(false);
    }
  };

  const handleUpdateDomainVersion = async (domainId: string, version: string) => {
    try {
      setUpdatingDomainId(domainId);
      await setDomainVersion(domainId, version);
      toast({
        title: t("common.success", "Success"),
        description: t("php.toast.domainVersionUpdated", "Domain PHP version updated to {{version}}", {
          version,
        }),
      });
      onConfigUpdated();
    } catch (err: any) {
      toast({
        title: t("common.error", "Error"),
        description: err?.message || t("php.errors.versionUpdateFailed", "Failed to update PHP version"),
        variant: "destructive",
      });
    } finally {
      setUpdatingDomainId(null);
    }
  };

  return (
    <div className="space-y-6" data-testid="php-version-selector">
      {/* Account Default PHP Version Card */}
      <Card className="border">
        <CardHeader className="pb-3">
          <CardTitle className="text-base flex items-center gap-2">
            <Cpu className="h-5 w-5 text-primary" />
            {t("php.defaultVersionTitle", "Tenant Default PHP Version")}
          </CardTitle>
          <CardDescription className="text-xs">
            {t(
              "php.defaultVersionDesc",
              "Standard PHP-FPM runtime applied to all domains unless explicitly customized below."
            )}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="flex flex-col sm:flex-row sm:items-center justify-between gap-4 p-3 bg-muted/40 rounded-lg border">
            <div className="flex items-center gap-3">
              <label htmlFor="select-default-php-version" className="text-xs font-semibold text-muted-foreground whitespace-nowrap">
                {t("php.activeVersion", "PHP Version:")}
              </label>
              <select
                id="select-default-php-version"
                value={selectedDefault}
                onChange={(e) => setSelectedDefault(e.target.value)}
                className="h-9 px-3 rounded-md border bg-background text-xs font-mono font-semibold focus:outline-none focus:ring-2 focus:ring-primary"
                data-testid="select-default-php-version"
              >
                {supportedVersions.map((v) => (
                  <option key={v.version} value={v.version}>
                    PHP {v.version} {v.status === "deprecated" ? "(Deprecated)" : ""}
                  </option>
                ))}
              </select>

              <Badge
                variant={
                  supportedVersions.find((v) => v.version === selectedDefault)?.status === "deprecated"
                    ? "destructive"
                    : "outline"
                }
                className="text-[10px] font-mono"
              >
                {supportedVersions.find((v) => v.version === selectedDefault)?.status === "deprecated"
                  ? "Deprecated"
                  : "Production Ready"}
              </Badge>
            </div>

            <Button
              size="sm"
              onClick={handleUpdateDefault}
              disabled={isUpdatingDefault || selectedDefault === defaultVersion}
              className="text-xs h-9 gap-1.5"
              data-testid="btn-update-default-php"
            >
              <Check className="h-4 w-4" />
              {isUpdatingDefault ? t("common.processing", "Saving...") : t("php.actions.applyDefault", "Apply Default")}
            </Button>
          </div>
        </CardContent>
      </Card>

      {/* Per-Domain PHP Version Overrides */}
      <Card className="border">
        <CardHeader className="pb-3">
          <CardTitle className="text-base flex items-center gap-2">
            <Globe className="h-5 w-5 text-primary" />
            {t("php.domainOverridesTitle", "Domain-Specific PHP Versions")}
          </CardTitle>
          <CardDescription className="text-xs">
            {t(
              "php.domainOverridesDesc",
              "Override PHP versions per website to run legacy or modern PHP applications side-by-side."
            )}
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="rounded-lg border overflow-hidden">
            <Table>
              <TableHeader className="bg-muted/50">
                <TableRow>
                  <TableHead className="text-xs">{t("php.fields.domain", "Domain")}</TableHead>
                  <TableHead className="text-xs">{t("php.fields.mode", "Inheritance")}</TableHead>
                  <TableHead className="text-xs">{t("php.fields.version", "Assigned Version")}</TableHead>
                  <TableHead className="text-xs text-right">{t("common.action", "Action")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {domainConfigs.map((dom) => (
                  <TableRow key={dom.domain_id} data-testid={`row-domain-php-${dom.domain_id}`}>
                    <TableCell className="font-mono text-xs font-semibold">
                      {dom.domain_name}
                    </TableCell>
                    <TableCell className="text-xs">
                      {dom.is_custom ? (
                        <Badge variant="outline" className="text-[10px] font-medium text-amber-600 border-amber-500/30">
                          Custom Override
                        </Badge>
                      ) : (
                        <Badge variant="secondary" className="text-[10px] font-medium text-muted-foreground">
                          Inherited Default
                        </Badge>
                      )}
                    </TableCell>
                    <TableCell className="font-mono text-xs font-semibold">
                      PHP {dom.php_version}
                    </TableCell>
                    <TableCell className="text-right">
                      <select
                        value={dom.php_version}
                        onChange={(e) => handleUpdateDomainVersion(dom.domain_id, e.target.value)}
                        disabled={updatingDomainId === dom.domain_id}
                        aria-label={`Change PHP version for ${dom.domain_name}`}
                        className="h-8 px-2 rounded border bg-background text-xs font-mono focus:outline-none focus:ring-1 focus:ring-primary"
                        data-testid={`select-domain-php-${dom.domain_id}`}
                      >
                        {supportedVersions.map((v) => (
                          <option key={v.version} value={v.version}>
                            PHP {v.version}
                          </option>
                        ))}
                      </select>
                    </TableCell>
                  </TableRow>
                ))}
              </TableBody>
            </Table>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
