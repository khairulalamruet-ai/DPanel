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
import { Layers, Cpu, Server, Save, HardDrive, Globe, Network, MapPin, Tag } from "lucide-react";
import { Plan, CreatePlanPayload, PlanResources, PlanLimits } from "./types";
import { PRESET_NODE_GROUPS, getNodeGroupById } from "./node-groups";

export interface PlanEditorDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  plan: Plan | null;
  onSave: (payload: CreatePlanPayload) => Promise<void>;
  isSaving?: boolean;
}

interface LimitFieldProps {
  id: string;
  label: string;
  value: number;
  onChange: (val: number) => void;
  unlimitedText: string;
  step?: number;
  min?: number;
  unit?: string;
  disabled?: boolean;
}

function LimitField({
  id,
  label,
  value,
  onChange,
  unlimitedText,
  step = 1,
  min = 1,
  unit,
  disabled = false,
}: LimitFieldProps) {
  const isUnlimited = value === -1;

  const handleToggleUnlimited = () => {
    if (disabled) return;
    if (isUnlimited) {
      onChange(min);
    } else {
      onChange(-1);
    }
  };

  return (
    <div className="space-y-1.5 p-3 rounded-lg border bg-card">
      <div className="flex items-center justify-between">
        <label htmlFor={id} className="text-xs font-semibold text-foreground">
          {label} {unit && <span className="font-normal text-muted-foreground">({unit})</span>}
        </label>
        <button
          type="button"
          onClick={handleToggleUnlimited}
          disabled={disabled}
          className={`text-[11px] font-medium px-2 py-0.5 rounded transition-colors ${
            isUnlimited
              ? "bg-primary text-primary-foreground font-bold"
              : "bg-muted text-muted-foreground hover:bg-muted/80 hover:text-foreground"
          }`}
        >
          {unlimitedText}
        </button>
      </div>
      <div>
        {isUnlimited ? (
          <div className="h-9 flex items-center px-3 rounded-md border bg-muted/30 text-xs font-mono text-primary font-bold">
            ∞ {unlimitedText}
          </div>
        ) : (
          <Input
            id={id}
            type="number"
            min={min}
            step={step}
            value={value}
            disabled={disabled}
            onChange={(e) => {
              const val = e.target.value === "" ? min : Number(e.target.value);
              onChange(val);
            }}
            className="h-9 text-xs"
          />
        )}
      </div>
    </div>
  );
}

export function PlanEditorDialog({
  open,
  onOpenChange,
  plan,
  onSave,
  isSaving = false,
}: PlanEditorDialogProps) {
  const { t } = useTranslation();

  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [featureSetId, setFeatureSetId] = useState("fs_default");
  const [featureSetName, setFeatureSetName] = useState("Default (Full Stack)");
  const [isDefault, setIsDefault] = useState(false);

  // Resources State
  const [resources, setResources] = useState<PlanResources>({
    cpu: 2,
    memory_mb: 2048,
    disk_mb: 25600,
    bandwidth_mb: -1,
    pids_limit: 256,
  });

  // Limits State
  const [limits, setLimits] = useState<PlanLimits>({
    max_domains: 5,
    max_subdomains: 20,
    max_databases: 5,
    max_ftp_accounts: 5,
    max_email_accounts: 20,
  });

  // Node Group & Cluster State
  const [nodeGroupId, setNodeGroupId] = useState("ng_global");
  const [nodeGroupName, setNodeGroupName] = useState("Global Cluster (Default)");
  const [targetRegion, setTargetRegion] = useState("global");
  const [targetTags, setTargetTags] = useState<string[]>(["general", "standard"]);

  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (plan) {
      setName(plan.name);
      setDescription(plan.description || "");
      setFeatureSetId(plan.feature_set_id || "fs_default");
      setFeatureSetName(plan.feature_set_name || "Default (Full Stack)");
      setIsDefault(plan.is_default);
      setResources({ ...plan.resources });
      setLimits({ ...plan.limits });
      const currentGroup = getNodeGroupById(plan.node_group_id);
      setNodeGroupId(plan.node_group_id || currentGroup.id);
      setNodeGroupName(plan.node_group_name || currentGroup.name);
      setTargetRegion(plan.target_region || currentGroup.region);
      setTargetTags(plan.target_tags || currentGroup.tags);
    } else {
      setName("");
      setDescription("");
      setFeatureSetId("fs_default");
      setFeatureSetName("Default (Full Stack)");
      setIsDefault(false);
      setResources({
        cpu: 2,
        memory_mb: 2048,
        disk_mb: 25600,
        bandwidth_mb: -1,
        pids_limit: 256,
      });
      setLimits({
        max_domains: 5,
        max_subdomains: 20,
        max_databases: 5,
        max_ftp_accounts: 5,
        max_email_accounts: 20,
      });
      const defaultGroup = PRESET_NODE_GROUPS[0];
      setNodeGroupId(defaultGroup.id);
      setNodeGroupName(defaultGroup.name);
      setTargetRegion(defaultGroup.region);
      setTargetTags(defaultGroup.tags);
    }
    setError(null);
  }, [plan, open]);

  const handleNodeGroupChange = (groupId: string) => {
    const selected = getNodeGroupById(groupId);
    setNodeGroupId(selected.id);
    setNodeGroupName(selected.name);
    setTargetRegion(selected.region);
    setTargetTags(selected.tags);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) {
      setError(t("plans.dialog.nameLabel") + " is required");
      return;
    }

    try {
      await onSave({
        name: name.trim(),
        description: description.trim(),
        feature_set_id: featureSetId,
        feature_set_name: featureSetName,
        is_default: isDefault,
        resources,
        limits,
        node_group_id: nodeGroupId,
        node_group_name: nodeGroupName,
        target_region: targetRegion,
        target_tags: targetTags,
      });
      onOpenChange(false);
    } catch (err: any) {
      setError(err?.message || "Failed to save plan");
    }
  };

  const isEditing = Boolean(plan);
  const unlimitedWord = t("plans.unlimited");

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent onClose={() => onOpenChange(false)} className="max-w-3xl max-h-[90vh] flex flex-col p-6">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <Server className="h-5 w-5" />
            <DialogTitle>
              {isEditing ? t("plans.dialog.editTitle") : t("plans.dialog.createTitle")}
            </DialogTitle>
          </div>
          <DialogDescription>{t("plans.dialog.desc")}</DialogDescription>
        </DialogHeader>

        <form noValidate onSubmit={handleSubmit} className="flex-1 overflow-y-auto space-y-6 pr-1 my-4">
          {/* Basic Section */}
          <div className="space-y-4">
            <h3 className="text-sm font-semibold tracking-tight text-foreground uppercase border-b pb-1.5 flex items-center gap-2">
              <Layers className="h-4 w-4 text-muted-foreground" />
              <span>{t("plans.dialog.basicSection")}</span>
            </h3>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
              <div className="space-y-1.5">
                <label htmlFor="plan-name" className="text-sm font-medium text-foreground flex items-center gap-1">
                  <span>{t("plans.dialog.nameLabel")}</span>
                  <span className="text-destructive font-bold">*</span>
                </label>
                <Input
                  id="plan-name"
                  value={name}
                  onChange={(e) => {
                    setName(e.target.value);
                    if (error) setError(null);
                  }}
                  placeholder={t("plans.dialog.namePlaceholder")}
                  disabled={isSaving}
                  autoFocus
                />
                {error && <p className="text-xs text-destructive">{error}</p>}
              </div>

              <div className="space-y-1.5">
                <label htmlFor="plan-desc" className="text-sm font-medium text-foreground">
                  {t("plans.dialog.descLabel")}
                </label>
                <Input
                  id="plan-desc"
                  value={description}
                  onChange={(e) => setDescription(e.target.value)}
                  placeholder={t("plans.dialog.descPlaceholder")}
                  disabled={isSaving}
                />
              </div>
            </div>

            <div className="grid grid-cols-1 md:grid-cols-2 gap-4 items-center">
              <div className="space-y-1.5">
                <label htmlFor="plan-featureset" className="text-sm font-medium text-foreground">
                  {t("plans.dialog.featureSetLabel")}
                </label>
                <select
                  id="plan-featureset"
                  value={featureSetId}
                  disabled={isSaving}
                  onChange={(e) => {
                    setFeatureSetId(e.target.value);
                    const opt = e.target.options[e.target.selectedIndex];
                    setFeatureSetName(opt ? opt.text : e.target.value);
                  }}
                  className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring text-foreground"
                >
                  <option value="fs_default">Default (Full Stack)</option>
                  <option value="fs_basic">Basic Web</option>
                  <option value="fs_dns_only">DNS Only</option>
                  <option value="fs_db_only">Database Only</option>
                </select>
              </div>

              {/* Default Plan Switch */}
              <div className="flex items-center justify-between rounded-lg border p-2.5 bg-muted/20 mt-4 md:mt-0">
                <div>
                  <p className="text-xs font-medium text-foreground">{t("plans.dialog.defaultLabel")}</p>
                  <p className="text-[11px] text-muted-foreground">{t("plans.dialog.defaultDesc")}</p>
                </div>
                <button
                  type="button"
                  role="switch"
                  aria-checked={isDefault}
                  disabled={isSaving}
                  onClick={() => setIsDefault(!isDefault)}
                  className={`relative inline-flex h-5 w-9 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out ${
                    isDefault ? "bg-primary" : "bg-muted"
                  }`}
                >
                  <span
                    className={`pointer-events-none inline-block h-4 w-4 transform rounded-full bg-white shadow-lg ring-0 transition duration-200 ease-in-out ${
                      isDefault ? "translate-x-4" : "translate-x-0"
                    }`}
                  />
                </button>
              </div>
            </div>
          </div>

          {/* Node Group & Cluster Placement Section */}
          <div className="space-y-4">
            <h3 className="text-sm font-semibold tracking-tight text-foreground uppercase border-b pb-1.5 flex items-center gap-2">
              <Network className="h-4 w-4 text-muted-foreground" />
              <span>{t("plans.dialog.nodeGroupSection")}</span>
            </h3>

            <div className="space-y-3">
              <div className="space-y-1.5">
                <label htmlFor="plan-node-group" className="text-sm font-medium text-foreground flex items-center gap-1">
                  <span>{t("plans.dialog.nodeGroupLabel")}</span>
                </label>
                <select
                  id="plan-node-group"
                  data-testid="plan-node-group-select"
                  value={nodeGroupId}
                  disabled={isSaving}
                  onChange={(e) => handleNodeGroupChange(e.target.value)}
                  className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring text-foreground"
                >
                  {PRESET_NODE_GROUPS.map((group) => (
                    <option key={group.id} value={group.id}>
                      {group.name} ({group.region})
                    </option>
                  ))}
                </select>
                <p className="text-xs text-muted-foreground">
                  {t("plans.dialog.nodeGroupDesc")}
                </p>
              </div>

              {/* Selected Node Group Details */}
              {(() => {
                const group = getNodeGroupById(nodeGroupId);
                return (
                  <div className="rounded-lg border bg-muted/20 p-3 space-y-2">
                    <div className="flex flex-wrap items-center justify-between gap-2">
                      <div className="flex items-center gap-2">
                        <span className="text-xs font-semibold text-foreground">
                          {group.name}
                        </span>
                        <Badge
                          variant="secondary"
                          data-testid="selected-group-region"
                          className="text-[10px] px-1.5 py-0 flex items-center gap-1 font-mono"
                        >
                          <MapPin className="h-2.5 w-2.5" />
                          {group.region}
                        </Badge>
                      </div>
                      <span className="text-[11px] text-muted-foreground">
                        {group.nodes_count} {t("plans.dialog.nodesCount", { count: group.nodes_count })}
                      </span>
                    </div>
                    <p className="text-xs text-muted-foreground">
                      {group.description}
                    </p>
                    <div className="flex items-center gap-1.5 pt-1">
                      <Tag className="h-3 w-3 text-muted-foreground shrink-0" />
                      <span className="text-[11px] text-muted-foreground font-medium">
                        {t("plans.dialog.tagsLabel")}:
                      </span>
                      <div className="flex flex-wrap gap-1">
                        {group.tags.map((tag) => (
                          <Badge key={tag} variant="outline" className="text-[10px] px-1.5 py-0 font-mono">
                            {tag}
                          </Badge>
                        ))}
                      </div>
                    </div>
                  </div>
                );
              })()}
            </div>
          </div>

          {/* Resources Section */}
          <div className="space-y-4">
            <h3 className="text-sm font-semibold tracking-tight text-foreground uppercase border-b pb-1.5 flex items-center gap-2">
              <Cpu className="h-4 w-4 text-muted-foreground" />
              <span>{t("plans.dialog.resourcesSection")}</span>
            </h3>

            <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-3">
              <LimitField
                id="res-cpu"
                label={t("plans.dialog.cpuLabel")}
                value={resources.cpu}
                onChange={(cpu) => setResources((r) => ({ ...r, cpu }))}
                unlimitedText={unlimitedWord}
                disabled={isSaving}
              />
              <LimitField
                id="res-memory"
                label={t("plans.dialog.memoryLabel")}
                value={resources.memory_mb}
                onChange={(memory_mb) => setResources((r) => ({ ...r, memory_mb }))}
                unlimitedText={unlimitedWord}
                unit="MB"
                min={256}
                step={256}
                disabled={isSaving}
              />
              <LimitField
                id="res-disk"
                label={t("plans.dialog.diskLabel")}
                value={resources.disk_mb}
                onChange={(disk_mb) => setResources((r) => ({ ...r, disk_mb }))}
                unlimitedText={unlimitedWord}
                unit="MB"
                min={1024}
                step={1024}
                disabled={isSaving}
              />
              <LimitField
                id="res-bandwidth"
                label={t("plans.dialog.bandwidthLabel")}
                value={resources.bandwidth_mb}
                onChange={(bandwidth_mb) => setResources((r) => ({ ...r, bandwidth_mb }))}
                unlimitedText={unlimitedWord}
                unit="MB"
                min={1024}
                step={1024}
                disabled={isSaving}
              />
              <LimitField
                id="res-pids"
                label={t("plans.dialog.pidsLabel")}
                value={resources.pids_limit}
                onChange={(pids_limit) => setResources((r) => ({ ...r, pids_limit }))}
                unlimitedText={unlimitedWord}
                min={32}
                step={32}
                disabled={isSaving}
              />
            </div>
          </div>

          {/* Limits Section */}
          <div className="space-y-4">
            <h3 className="text-sm font-semibold tracking-tight text-foreground uppercase border-b pb-1.5 flex items-center gap-2">
              <Globe className="h-4 w-4 text-muted-foreground" />
              <span>{t("plans.dialog.limitsSection")}</span>
            </h3>

            <div className="grid grid-cols-1 sm:grid-cols-2 md:grid-cols-3 gap-3">
              <LimitField
                id="lim-domains"
                label={t("plans.dialog.domainsLabel")}
                value={limits.max_domains}
                onChange={(max_domains) => setLimits((l) => ({ ...l, max_domains }))}
                unlimitedText={unlimitedWord}
                disabled={isSaving}
              />
              <LimitField
                id="lim-subdomains"
                label={t("plans.dialog.subdomainsLabel")}
                value={limits.max_subdomains}
                onChange={(max_subdomains) => setLimits((l) => ({ ...l, max_subdomains }))}
                unlimitedText={unlimitedWord}
                disabled={isSaving}
              />
              <LimitField
                id="lim-databases"
                label={t("plans.dialog.databasesLabel")}
                value={limits.max_databases}
                onChange={(max_databases) => setLimits((l) => ({ ...l, max_databases }))}
                unlimitedText={unlimitedWord}
                disabled={isSaving}
              />
              <LimitField
                id="lim-ftp"
                label={t("plans.dialog.ftpLabel")}
                value={limits.max_ftp_accounts}
                onChange={(max_ftp_accounts) => setLimits((l) => ({ ...l, max_ftp_accounts }))}
                unlimitedText={unlimitedWord}
                disabled={isSaving}
              />
              <LimitField
                id="lim-emails"
                label={t("plans.dialog.emailsLabel")}
                value={limits.max_email_accounts}
                onChange={(max_email_accounts) => setLimits((l) => ({ ...l, max_email_accounts }))}
                unlimitedText={unlimitedWord}
                disabled={isSaving}
              />
            </div>
          </div>

          <DialogFooter className="pt-4 border-t">
            <Button
              type="button"
              variant="outline"
              disabled={isSaving}
              onClick={() => onOpenChange(false)}
            >
              {t("plans.dialog.cancel")}
            </Button>
            <Button type="submit" disabled={isSaving} className="gap-2">
              <Save className="h-4 w-4" />
              {isSaving ? t("plans.dialog.saving") : t("plans.dialog.save")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
