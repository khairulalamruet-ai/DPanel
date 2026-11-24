import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Input,
  Badge,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { CheckSquare, Square, Layers, Save } from "lucide-react";
import { FeatureSet, CreateFeatureSetPayload, FeatureCategoryKey } from "./types";
import { AVAILABLE_FEATURES, CATEGORY_ORDER } from "./feature-definitions";

export interface FeatureSetEditorDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  featureSet: FeatureSet | null;
  onSave: (payload: CreateFeatureSetPayload) => Promise<void>;
  isSaving?: boolean;
}

export function FeatureSetEditorDialog({
  open,
  onOpenChange,
  featureSet,
  onSave,
  isSaving = false,
}: FeatureSetEditorDialogProps) {
  const { t, i18n } = useTranslation();
  const lang = (i18n.language?.startsWith("bn") ? "bn" : "en") as "en" | "bn";

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [isDefault, setIsDefault] = useState(false);
  const [selectedFeatures, setSelectedFeatures] = useState<string[]>([]);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (featureSet) {
      setName(featureSet.name);
      setDescription(featureSet.description || "");
      setIsDefault(featureSet.is_default);
      setSelectedFeatures(featureSet.features || []);
    } else {
      setName("");
      setDescription("");
      setIsDefault(false);
      // Pre-select some sensible defaults on new set
      setSelectedFeatures(["domains", "docroot", "ssl_letsencrypt", "file_manager"]);
    }
    setError(null);
  }, [featureSet, open]);

  const toggleFeature = (key: string) => {
    setSelectedFeatures((prev) =>
      prev.includes(key) ? prev.filter((k) => k !== key) : [...prev, key]
    );
  };

  const handleSelectAllCategory = (catKey: FeatureCategoryKey) => {
    const catFeatureKeys = AVAILABLE_FEATURES.filter((f) => f.category === catKey).map(
      (f) => f.key
    );
    setSelectedFeatures((prev) => Array.from(new Set([...prev, ...catFeatureKeys])));
  };

  const handleDeselectAllCategory = (catKey: FeatureCategoryKey) => {
    const catFeatureKeys = AVAILABLE_FEATURES.filter((f) => f.category === catKey).map(
      (f) => f.key
    );
    setSelectedFeatures((prev) => prev.filter((k) => !catFeatureKeys.includes(k)));
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) {
      setError(t("featureSets.dialog.nameLabel") + " is required");
      return;
    }

    try {
      await onSave({
        name: name.trim(),
        description: description.trim(),
        is_default: isDefault,
        features: selectedFeatures,
      });
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || "Failed to save feature set");
    }
  };

  const isEditing = Boolean(featureSet);

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-3xl max-h-[90vh] flex flex-col p-6">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <Layers className="h-5 w-5" />
            <DialogTitle>
              {isEditing
                ? t("featureSets.dialog.editTitle")
                : t("featureSets.dialog.createTitle")}
            </DialogTitle>
          </div>
          <DialogDescription>{t("featureSets.dialog.desc")}</DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="flex-1 overflow-y-auto space-y-6 pr-1 my-4">
          {/* Basic Details */}
          <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div className="space-y-1.5">
              <label htmlFor="fs-name" className="text-sm font-medium text-foreground flex items-center gap-1">
                <span>{t("featureSets.dialog.nameLabel")}</span>
                <span className="text-destructive font-bold">*</span>
              </label>
              <Input
                id="fs-name"
                value={name}
                onChange={(e) => {
                  setName(e.target.value);
                  if (error) setError(null);
                }}
                placeholder={t("featureSets.dialog.namePlaceholder")}
                disabled={isSaving}
                autoFocus
              />
              {error && <p className="text-xs text-destructive">{error}</p>}
            </div>

            <div className="space-y-1.5">
              <label htmlFor="fs-desc" className="text-sm font-medium text-foreground">
                {t("featureSets.dialog.descLabel")}
              </label>
              <Input
                id="fs-desc"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                placeholder={t("featureSets.dialog.descPlaceholder")}
                disabled={isSaving}
              />
            </div>
          </div>

          {/* Default Switch */}
          <div className="flex items-center justify-between rounded-lg border p-3 bg-muted/20">
            <div>
              <p className="text-sm font-medium text-foreground">{t("featureSets.dialog.defaultLabel")}</p>
              <p className="text-xs text-muted-foreground">{t("featureSets.dialog.defaultDesc")}</p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={isDefault}
              disabled={isSaving}
              onClick={() => setIsDefault(!isDefault)}
              className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out ${
                isDefault ? "bg-primary" : "bg-muted"
              }`}
            >
              <span
                className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow-lg ring-0 transition duration-200 ease-in-out ${
                  isDefault ? "translate-x-5" : "translate-x-0"
                }`}
              />
            </button>
          </div>

          {/* Features by Category */}
          <div className="space-y-6">
            <h3 className="text-sm font-semibold tracking-tight text-foreground uppercase border-b pb-2">
              {t("featureSets.table.features")} ({selectedFeatures.length} / {AVAILABLE_FEATURES.length})
            </h3>

            {CATEGORY_ORDER.map((catKey) => {
              const catFeatures = AVAILABLE_FEATURES.filter((f) => f.category === catKey);
              const selectedInCat = catFeatures.filter((f) => selectedFeatures.includes(f.key));
              const allSelected = selectedInCat.length === catFeatures.length;

              return (
                <div key={catKey} className="rounded-lg border bg-card p-4 space-y-3">
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-2">
                      <span className="font-semibold text-sm text-foreground">
                        {t(`featureSets.categories.${catKey}`)}
                      </span>
                      <Badge variant="secondary" className="text-xs font-mono">
                        {selectedInCat.length} / {catFeatures.length}
                      </Badge>
                    </div>
                    <div className="flex items-center gap-2">
                      <Button
                        type="button"
                        variant="ghost"
                        size="sm"
                        className="text-xs h-7 px-2"
                        onClick={() =>
                          allSelected
                            ? handleDeselectAllCategory(catKey)
                            : handleSelectAllCategory(catKey)
                        }
                      >
                        {allSelected
                          ? t("featureSets.dialog.deselectAll")
                          : t("featureSets.dialog.selectAll")}
                      </Button>
                    </div>
                  </div>

                  <div className="grid grid-cols-1 md:grid-cols-2 gap-2.5 pt-1">
                    {catFeatures.map((feat) => {
                      const isChecked = selectedFeatures.includes(feat.key);
                      return (
                        <div
                          key={feat.key}
                          onClick={() => toggleFeature(feat.key)}
                          className={`flex items-start gap-3 p-2.5 rounded-md border text-sm cursor-pointer transition-colors ${
                            isChecked
                              ? "border-primary/50 bg-primary/5 text-foreground"
                              : "border-border/60 hover:bg-muted/40 text-muted-foreground"
                          }`}
                        >
                          <div className="mt-0.5 text-primary">
                            {isChecked ? (
                              <CheckSquare className="h-4 w-4" />
                            ) : (
                              <Square className="h-4 w-4 text-muted-foreground" />
                            )}
                          </div>
                          <div className="flex-1 space-y-0.5">
                            <div className="flex items-center justify-between gap-1">
                              <span className="font-medium text-foreground text-xs">
                                {feat.title[lang] || feat.title.en}
                              </span>
                              <span className="font-mono text-[10px] bg-muted px-1 rounded text-muted-foreground">
                                {feat.key}
                              </span>
                            </div>
                            <p className="text-[11px] text-muted-foreground line-clamp-2">
                              {feat.description[lang] || feat.description.en}
                            </p>
                          </div>
                        </div>
                      );
                    })}
                  </div>
                </div>
              );
            })}
          </div>

          <DialogFooter className="pt-4 border-t">
            <Button
              type="button"
              variant="outline"
              disabled={isSaving}
              onClick={() => onOpenChange(false)}
            >
              {t("featureSets.dialog.cancel")}
            </Button>
            <Button type="submit" disabled={isSaving} className="gap-2">
              <Save className="h-4 w-4" />
              {isSaving ? t("featureSets.dialog.saving") : t("featureSets.dialog.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
