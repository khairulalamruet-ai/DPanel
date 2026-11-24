import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
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
import { diskUsage, remeasure } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  HardDrive,
  Folder,
  RefreshCw,
  ExternalLink,
  Layers,
  Database,
  AlertCircle,
  FileText,
} from "lucide-react";
import { DiskUsageResponse, DirectoryUsageItem } from "./types";

export function DiskUsagePage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [data, setData] = useState<DiskUsageResponse | null>(null);
  const [isLoading, setIsLoading] = useState(false);
  const [isRecalculating, setIsRecalculating] = useState(false);

  const fetchDiskUsage = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = await diskUsage();
      setData(res);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    fetchDiskUsage();
  }, [fetchDiskUsage]);

  const handleRecalculate = async () => {
    try {
      setIsRecalculating(true);
      await remeasure();
      toast({
        title: t("common.success"),
        description: t("diskUsage.recalculatedSuccess"),
      });
      await fetchDiskUsage();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsRecalculating(false);
    }
  };

  const topDirs: DirectoryUsageItem[] = data?.top_directories || [];

  return (
    <div className="p-6 space-y-6" data-testid="disk-usage-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="disk-usage-title">
            {t("diskUsage.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("diskUsage.description")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="gap-2"
            onClick={handleRecalculate}
            disabled={isRecalculating || isLoading}
            data-testid="btn-recalculate-disk"
          >
            <RefreshCw className={`w-4 h-4 ${isRecalculating ? "animate-spin" : ""}`} />
            <span>{isRecalculating ? t("common.loading") : t("diskUsage.recalculate")}</span>
          </Button>
        </div>
      </div>

      {/* Quota Overview Cards */}
      <div className="grid grid-cols-1 md:grid-cols-2 gap-6">
        {/* Disk Quota Card */}
        <Card data-testid="card-disk-quota">
          <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
            <CardTitle className="text-sm font-semibold flex items-center gap-2">
              <HardDrive className="w-4 h-4 text-primary" />
              <span>{t("diskUsage.diskQuota")}</span>
            </CardTitle>
            {data && (
              <Badge variant={data.disk_usage_pct > 90 ? "destructive" : "secondary"}>
                {data.disk_usage_pct}% {t("diskUsage.used")}
              </Badge>
            )}
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex items-baseline justify-between">
              <span className="text-2xl font-bold" data-testid="metric-disk-used">
                {data?.disk_used_formatted || "-"}
              </span>
              <span className="text-sm text-muted-foreground">
                {t("diskUsage.limit")}: {data?.disk_limit_formatted || "-"}
              </span>
            </div>
            {/* Progress bar */}
            <div className="w-full bg-muted rounded-full h-2.5 overflow-hidden">
              <div
                className={`h-2.5 rounded-full transition-all ${
                  (data?.disk_usage_pct || 0) > 90
                    ? "bg-destructive"
                    : (data?.disk_usage_pct || 0) > 75
                    ? "bg-amber-500"
                    : "bg-primary"
                }`}
                style={{ width: `${Math.min(data?.disk_usage_pct || 0, 100)}%` }}
                data-testid="disk-progress-bar"
              />
            </div>
            <p className="text-xs text-muted-foreground flex items-center gap-1">
              <span>{t("diskUsage.lastScanned")}:</span>
              <span>{data?.last_scanned_at ? new Date(data.last_scanned_at).toLocaleString() : "-"}</span>
            </p>
          </CardContent>
        </Card>

        {/* Inodes Quota Card */}
        <Card data-testid="card-inodes-quota">
          <CardHeader className="flex flex-row items-center justify-between pb-2 space-y-0">
            <CardTitle className="text-sm font-semibold flex items-center gap-2">
              <Layers className="w-4 h-4 text-primary" />
              <span>{t("diskUsage.inodesQuota")}</span>
            </CardTitle>
            {data && (
              <Badge variant={data.inodes_usage_pct > 90 ? "destructive" : "secondary"}>
                {data.inodes_usage_pct}% {t("diskUsage.used")}
              </Badge>
            )}
          </CardHeader>
          <CardContent className="space-y-3">
            <div className="flex items-baseline justify-between">
              <span className="text-2xl font-bold" data-testid="metric-inodes-used">
                {data ? data.inodes_used.toLocaleString() : "-"}
              </span>
              <span className="text-sm text-muted-foreground">
                {t("diskUsage.limit")}: {data ? data.inodes_limit.toLocaleString() : "-"}
              </span>
            </div>
            {/* Progress bar */}
            <div className="w-full bg-muted rounded-full h-2.5 overflow-hidden">
              <div
                className={`h-2.5 rounded-full transition-all ${
                  (data?.inodes_usage_pct || 0) > 90
                    ? "bg-destructive"
                    : (data?.inodes_usage_pct || 0) > 75
                    ? "bg-amber-500"
                    : "bg-primary"
                }`}
                style={{ width: `${Math.min(data?.inodes_usage_pct || 0, 100)}%` }}
                data-testid="inodes-progress-bar"
              />
            </div>
            <p className="text-xs text-muted-foreground">
              {t("diskUsage.inodesDescription")}
            </p>
          </CardContent>
        </Card>
      </div>

      {/* Top 10 Largest Folders Table (DoD Requirement) */}
      <Card data-testid="card-top-directories">
        <CardHeader className="py-4 px-6 border-b">
          <div className="flex items-center justify-between">
            <CardTitle className="text-base font-semibold flex items-center gap-2">
              <Folder className="w-5 h-5 text-amber-500" />
              <span data-testid="top-directories-heading">
                {t("diskUsage.topTenDirectories")}
              </span>
            </CardTitle>
            <span className="text-xs text-muted-foreground">
              {t("diskUsage.topTenDescription")}
            </span>
          </div>
        </CardHeader>

        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead className="w-12 text-center">#</TableHead>
                <TableHead>{t("diskUsage.folderPath")}</TableHead>
                <TableHead className="w-32">{t("diskUsage.size")}</TableHead>
                <TableHead className="w-48">{t("diskUsage.quotaShare")}</TableHead>
                <TableHead className="w-28 text-right">{t("diskUsage.inodes")}</TableHead>
                <TableHead className="w-28 text-right">{t("diskUsage.filesCount")}</TableHead>
                <TableHead className="w-24 text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-10 text-muted-foreground">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : topDirs.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={7} className="text-center py-10 text-muted-foreground">
                    {t("diskUsage.noDirectoriesFound")}
                  </TableCell>
                </TableRow>
              ) : (
                topDirs.map((dir, index) => (
                  <TableRow
                    key={dir.path}
                    className="hover:bg-muted/50 transition-colors"
                    data-testid={`top-dir-row-${index}`}
                  >
                    <TableCell className="text-center font-mono text-xs text-muted-foreground font-semibold">
                      {index + 1}
                    </TableCell>

                    <TableCell>
                      <div className="flex items-center gap-2">
                        <Folder className="w-4 h-4 text-amber-500 shrink-0" />
                        <span
                          className="font-mono text-xs text-foreground truncate max-w-sm"
                          title={dir.path}
                          data-testid={`dir-path-${index}`}
                        >
                          {dir.path}
                        </span>
                      </div>
                    </TableCell>

                    <TableCell className="font-mono text-xs font-semibold" data-testid={`dir-size-${index}`}>
                      {dir.size_formatted}
                    </TableCell>

                    <TableCell>
                      <div className="flex items-center gap-2">
                        <div className="w-24 bg-muted rounded-full h-2 overflow-hidden">
                          <div
                            className="bg-primary h-2 rounded-full"
                            style={{ width: `${Math.min(dir.percentage_of_total, 100)}%` }}
                          />
                        </div>
                        <span className="font-mono text-xs text-muted-foreground">
                          {dir.percentage_of_total}%
                        </span>
                      </div>
                    </TableCell>

                    <TableCell className="text-right font-mono text-xs text-muted-foreground">
                      {dir.inodes_count.toLocaleString()}
                    </TableCell>

                    <TableCell className="text-right font-mono text-xs text-muted-foreground">
                      {dir.files_count.toLocaleString()}
                    </TableCell>

                    <TableCell className="text-right">
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-7 w-7 p-0"
                        title={t("diskUsage.openInFileManager")}
                        onClick={() => {
                          window.location.href = `/files?path=${encodeURIComponent(dir.path)}`;
                        }}
                        data-testid={`btn-open-dir-${index}`}
                      >
                        <ExternalLink className="w-3.5 h-3.5" />
                      </Button>
                    </TableCell>
                  </TableRow>
                ))
              )}
            </TableBody>
          </Table>
        </div>
      </Card>
    </div>
  );
}
