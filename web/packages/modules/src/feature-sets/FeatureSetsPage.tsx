import React, { useState, useEffect } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Table,
  TableHeader,
  TableBody,
  TableHead,
  TableRow,
  TableCell,
  Button,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import {
  Layers,
  Plus,
  Edit2,
  Trash2,
  RefreshCw,
  Star,
  CheckCircle2,
  AlertTriangle,
} from "lucide-react";
import { FeatureSet, CreateFeatureSetPayload } from "./types";
import { FeatureSetEditorDialog } from "./FeatureSetEditorDialog";

export function FeatureSetsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [featureSets, setFeatureSets] = useState<FeatureSet[]>([]);
  const [isLoading, setIsLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  // Editor Dialog State
  const [isEditorOpen, setIsEditorOpen] = useState(false);
  const [editingSet, setEditingSet] = useState<FeatureSet | null>(null);
  const [isSaving, setIsSaving] = useState(false);

  // Delete State
  const [deletingId, setDeletingId] = useState<string | null>(null);

  const fetchFeatureSets = async () => {
    try {
      setIsLoading(true);
      setError(null);
      const res = await apiFetch<{ items: FeatureSet[]; total: number }>(
        "/api/v1/admin/feature-sets"
      );
      setFeatureSets(res.items || []);
    } catch (err: any) {
      setError(err?.message || "Failed to load feature sets");
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    fetchFeatureSets();
  }, []);

  const handleOpenCreate = () => {
    setEditingSet(null);
    setIsEditorOpen(true);
  };

  const handleOpenEdit = (fs: FeatureSet) => {
    setEditingSet(fs);
    setIsEditorOpen(true);
  };

  const handleSave = async (payload: CreateFeatureSetPayload) => {
    setIsSaving(true);
    try {
      if (editingSet) {
        // Edit existing
        const res = await apiFetch<FeatureSet>(
          `/api/v1/admin/feature-sets/${editingSet.id}`,
          {
            method: "PUT",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify(payload),
          }
        );
        toast({
          title: t("common.appName"),
          description: t("featureSets.toast.updateSuccess", { name: res.name }),
          variant: "default",
        });
        setFeatureSets((prev) =>
          prev.map((item) => {
            if (payload.is_default && item.id !== res.id) {
              return { ...item, is_default: false };
            }
            return item.id === res.id ? res : item;
          })
        );
      } else {
        // Create new
        const res = await apiFetch<FeatureSet>("/api/v1/admin/feature-sets", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(payload),
        });
        toast({
          title: t("common.appName"),
          description: t("featureSets.toast.createSuccess", { name: res.name }),
          variant: "default",
        });
        setFeatureSets((prev) => {
          const list = payload.is_default
            ? prev.map((item) => ({ ...item, is_default: false }))
            : [...prev];
          return [res, ...list];
        });
      }
      setIsEditorOpen(false);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("featureSets.toast.error"),
        variant: "destructive",
      });
      throw err;
    } finally {
      setIsSaving(false);
    }
  };

  const handleDelete = async (fs: FeatureSet) => {
    if (fs.is_default) {
      toast({
        title: t("common.error"),
        description: t("featureSets.deleteConfirm.cannotDeleteDefault"),
        variant: "destructive",
      });
      return;
    }

    if (!window.confirm(t("featureSets.deleteConfirm.desc", { name: fs.name }))) {
      return;
    }

    try {
      setDeletingId(fs.id);
      await apiFetch(`/api/v1/admin/feature-sets/${fs.id}`, {
        method: "DELETE",
      });
      toast({
        title: t("common.appName"),
        description: t("featureSets.toast.deleteSuccess", { name: fs.name }),
        variant: "default",
      });
      setFeatureSets((prev) => prev.filter((item) => item.id !== fs.id));
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || "Failed to delete feature set",
        variant: "destructive",
      });
    } finally {
      setDeletingId(null);
    }
  };

  return (
    <div className="space-y-6">
      {/* Page Header */}
      <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
        <div>
          <div className="flex items-center gap-2">
            <Layers className="h-6 w-6 text-primary" />
            <h1 className="text-2xl font-bold tracking-tight">{t("featureSets.title")}</h1>
          </div>
          <p className="text-sm text-muted-foreground mt-1">{t("featureSets.desc")}</p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            onClick={fetchFeatureSets}
            disabled={isLoading}
            className="gap-2"
          >
            <RefreshCw className={`h-4 w-4 ${isLoading ? "animate-spin" : ""}`} />
            {t("common.reload")}
          </Button>
          <Button onClick={handleOpenCreate} className="gap-2">
            <Plus className="h-4 w-4" />
            {t("featureSets.createBtn")}
          </Button>
        </div>
      </div>

      {/* Main Card */}
      <Card>
        <CardContent className="p-0">
          {isLoading ? (
            <div className="py-16 text-center text-sm text-muted-foreground">
              <RefreshCw className="h-6 w-6 animate-spin mx-auto mb-2 text-primary" />
              {t("common.loading")}
            </div>
          ) : error ? (
            <div className="py-12 text-center space-y-3">
              <p className="text-sm text-destructive">{error}</p>
              <Button variant="outline" size="sm" onClick={fetchFeatureSets}>
                {t("common.retry")}
              </Button>
            </div>
          ) : featureSets.length === 0 ? (
            <div className="py-16 text-center text-sm text-muted-foreground">
              {t("featureSets.empty")}
            </div>
          ) : (
            <div className="overflow-x-auto">
              <Table>
                <TableHeader>
                  <TableRow>
                    <TableHead className="w-[30%]">{t("featureSets.table.name")}</TableHead>
                    <TableHead className="w-[35%]">{t("featureSets.table.features")}</TableHead>
                    <TableHead className="w-[15%]">{t("featureSets.table.plans")}</TableHead>
                    <TableHead className="w-[20%] text-right">
                      {t("featureSets.table.actions")}
                    </TableHead>
                  </TableRow>
                </TableHeader>
                <TableBody>
                  {featureSets.map((fs) => (
                    <TableRow key={fs.id} className="hover:bg-muted/50 transition-colors">
                      {/* Name & Description */}
                      <TableCell>
                        <div className="flex flex-col">
                          <div className="flex items-center gap-2">
                            <span className="font-semibold text-foreground text-sm">
                              {fs.name}
                            </span>
                            {fs.is_default && (
                              <Badge
                                variant="secondary"
                                className="text-[10px] px-1.5 py-0 font-medium bg-amber-500/10 text-amber-600 border border-amber-500/20 flex items-center gap-1"
                              >
                                <Star className="h-2.5 w-2.5 fill-amber-500" />
                                {t("featureSets.defaultBadge")}
                              </Badge>
                            )}
                          </div>
                          {fs.description && (
                            <p className="text-xs text-muted-foreground mt-0.5 line-clamp-1">
                              {fs.description}
                            </p>
                          )}
                        </div>
                      </TableCell>

                      {/* Features */}
                      <TableCell>
                        <div className="flex flex-col gap-1.5">
                          <span className="text-xs font-medium text-foreground">
                            {t("featureSets.featuresCount", { count: fs.features.length })}
                          </span>
                          <div className="flex flex-wrap gap-1 max-w-md">
                            {fs.features.slice(0, 5).map((featKey) => (
                              <span
                                key={featKey}
                                className="text-[10px] font-mono bg-muted/80 text-muted-foreground px-1.5 py-0.5 rounded border"
                              >
                                {featKey}
                              </span>
                            ))}
                            {fs.features.length > 5 && (
                              <span className="text-[10px] font-mono text-muted-foreground px-1 py-0.5">
                                +{fs.features.length - 5} more
                              </span>
                            )}
                          </div>
                        </div>
                      </TableCell>

                      {/* Plans Count */}
                      <TableCell>
                        <Badge variant="outline" className="text-xs font-normal">
                          {t("featureSets.plansUsing", { count: fs.plans_count })}
                        </Badge>
                      </TableCell>

                      {/* Actions */}
                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1.5">
                          <Button
                            variant="outline"
                            size="sm"
                            onClick={() => handleOpenEdit(fs)}
                            className="h-8 px-2.5 text-xs gap-1"
                          >
                            <Edit2 className="h-3.5 w-3.5" />
                            {t("featureSets.editBtn")}
                          </Button>
                          <Button
                            variant="ghost"
                            size="sm"
                            disabled={fs.is_default || deletingId === fs.id}
                            onClick={() => handleDelete(fs)}
                            title={
                              fs.is_default
                                ? t("featureSets.deleteConfirm.cannotDeleteDefault")
                                : t("featureSets.deleteBtn")
                            }
                            className="h-8 px-2.5 text-xs text-destructive hover:bg-destructive/10 hover:text-destructive disabled:opacity-40"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                            {t("featureSets.deleteBtn")}
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  ))}
                </TableBody>
              </Table>
            </div>
          )}
        </CardContent>
      </Card>

      {/* Feature Set Editor Dialog */}
      <FeatureSetEditorDialog
        open={isEditorOpen}
        onOpenChange={setIsEditorOpen}
        featureSet={editingSet}
        onSave={handleSave}
        isSaving={isSaving}
      />
    </div>
  );
}
