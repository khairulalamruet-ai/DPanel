import React, { useState } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
  Button,
  Input,
  Badge,
  useToast,
} from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";
import { useTranslation } from "@builtpanel/i18n";
import {
  User,
  Package,
  Server,
  Globe,
  CheckCircle2,
  ArrowRight,
  ArrowLeft,
  Key,
  Shield,
  Layers,
  Sparkles,
  Info,
  AlertTriangle,
} from "lucide-react";
import {
  WizardStep,
  UserWizardFormData,
  OperationStep,
  ContainerInfo,
  CreateUserWizardResponse,
} from "./types";
import { HOSTING_PLANS, CLUSTER_NODES, RESERVED_USERNAMES } from "./constants";
import { OperationProgress } from "./OperationProgress";
import { selectPlacementNode } from "../nodes/placement";

export function generateRandomPassword(): string {
  const chars = "abcdefghjkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789!@#$%^&*";
  let pass = "";
  for (let i = 0; i < 16; i++) {
    pass += chars.charAt(Math.floor(Math.random() * chars.length));
  }
  return pass;
}

export function UserCreateWizardPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [step, setStep] = useState<WizardStep>(1);

  // Form State
  const [formData, setFormData] = useState<UserWizardFormData>({
    username: "",
    email: "",
    password: "",
    plan_id: "plan_standard",
    node_mode: "auto",
    node_id: "node-sgp-01",
    primary_domain: "",
    create_vhost: true,
  });

  const [errors, setErrors] = useState<Record<string, string>>({});
  const [isSubmitting, setIsSubmitting] = useState(false);

  // Operation progress state
  const [isProvisioning, setIsProvisioning] = useState(false);
  const [isComplete, setIsComplete] = useState(false);
  const [containerInfo, setContainerInfo] = useState<ContainerInfo | null>(null);
  const [opSteps, setOpSteps] = useState<OperationStep[]>([
    {
      id: "step_tenant",
      title: "Create System Tenant User",
      description: "Allocating unique Linux UID and /srv/tenants/<username> directory",
      status: "pending",
    },
    {
      id: "step_quota",
      title: "Set Storage & Resource Quota",
      description: "Applying filesystem disk quota and cgroup resource limits",
      status: "pending",
    },
    {
      id: "step_container",
      title: "Provision Hardened Docker Stack",
      description: "Launching rootless container stack with seccomp profile",
      status: "pending",
    },
    {
      id: "step_health",
      title: "Verify Container Health & Networking",
      description: "Validating non-root processes and internal bridge connectivity",
      status: "pending",
    },
  ]);

  // Validation
  const validateStep1 = (): boolean => {
    const errs: Record<string, string> = {};
    const u = formData.username.trim().toLowerCase();

    if (!u) {
      errs.username = t("userWizard.validation.usernameRequired");
    } else if (u.length < 3 || u.length > 32) {
      errs.username = t("userWizard.validation.usernameLength");
    } else if (!/^[a-z0-9][a-z0-9_-]*$/.test(u)) {
      errs.username = t("userWizard.validation.usernameFormat");
    } else if (RESERVED_USERNAMES.has(u)) {
      errs.username = t("userWizard.validation.usernameReserved");
    }

    const email = formData.email.trim();
    if (!email) {
      errs.email = t("userWizard.validation.emailRequired");
    } else if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(email)) {
      errs.email = t("userWizard.validation.emailInvalid");
    }

    if (!formData.password) {
      errs.password = t("userWizard.validation.passwordRequired");
    } else if (formData.password.length < 8) {
      errs.password = t("userWizard.validation.passwordMin");
    }

    setErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const validateStep3 = (): boolean => {
    const errs: Record<string, string> = {};
    if (formData.node_mode === "manual" && formData.node_id) {
      const target = CLUSTER_NODES.find((n) => n.id === formData.node_id);
      if (target?.status === "draining") {
        errs.node = t("userWizard.step3.nodeDrainingError");
      }
    }
    setErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const validateStep4 = (): boolean => {
    const errs: Record<string, string> = {};
    const d = formData.primary_domain?.trim();
    if (d && !/^[a-zA-Z0-9][a-zA-Z0-9-_.]+\.[a-zA-Z]{2,}$/.test(d)) {
      errs.primary_domain = t("userWizard.validation.domainInvalid");
    }
    setErrors(errs);
    return Object.keys(errs).length === 0;
  };

  const handleNext = () => {
    if (step === 1 && !validateStep1()) return;
    if (step === 3 && !validateStep3()) return;
    if (step === 4 && !validateStep4()) return;

    setErrors({});
    setStep((s) => Math.min(5, s + 1) as WizardStep);
  };

  const handleBack = () => {
    setErrors({});
    setStep((s) => Math.max(1, s - 1) as WizardStep);
  };

  const handleCreateUser = async () => {
    setIsSubmitting(true);
    setIsProvisioning(true);

    try {
      // Step 1: Start
      setOpSteps((prev) =>
        prev.map((s, idx) => (idx === 0 ? { ...s, status: "running" } : s))
      );

      const targetNodeId =
        formData.node_mode === "auto" ? autoSelectedNode.id : (formData.node_id || autoSelectedNode.id);
      const targetNode = CLUSTER_NODES.find((n) => n.id === targetNodeId);

      if (targetNode?.status === "draining") {
        toast({
          title: t("common.error"),
          description: t("userWizard.step3.nodeDrainingError"),
          variant: "destructive",
        });
        setIsSubmitting(false);
        setIsProvisioning(false);
        return;
      }

      const payload = {
        username: formData.username.trim().toLowerCase(),
        email: formData.email.trim(),
        password: formData.password,
        plan_id: formData.plan_id,
        node_id: targetNodeId,
        primary_domain: formData.primary_domain?.trim() || undefined,
        create_vhost: formData.create_vhost,
      };

      const res = await apiFetch<CreateUserWizardResponse>("/api/v1/admin/users", {
        method: "POST",
        body: JSON.stringify(payload),
      });

      // Animate progress steps
      await new Promise((r) => setTimeout(r, 400));
      setOpSteps((prev) =>
        prev.map((s, idx) =>
          idx === 0 ? { ...s, status: "succeeded" } : idx === 1 ? { ...s, status: "running" } : s
        )
      );

      await new Promise((r) => setTimeout(r, 400));
      setOpSteps((prev) =>
        prev.map((s, idx) =>
          idx <= 1 ? { ...s, status: "succeeded" } : idx === 2 ? { ...s, status: "running" } : s
        )
      );

      await new Promise((r) => setTimeout(r, 400));
      setOpSteps((prev) =>
        prev.map((s, idx) =>
          idx <= 2 ? { ...s, status: "succeeded" } : idx === 3 ? { ...s, status: "running" } : s
        )
      );

      await new Promise((r) => setTimeout(r, 400));
      setOpSteps((prev) => prev.map((s) => ({ ...s, status: "succeeded" })));

      setContainerInfo(res.container);
      setIsComplete(true);

      toast({
        title: t("common.appName"),
        description: t("userWizard.toast.success", { username: payload.username }),
      });
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err?.message || t("userWizard.toast.error"),
        variant: "destructive",
      });
      setOpSteps((prev) =>
        prev.map((s) => (s.status === "running" ? { ...s, status: "failed" } : s))
      );
    } finally {
      setIsSubmitting(false);
    }
  };

  const handleReset = () => {
    setStep(1);
    setIsProvisioning(false);
    setIsComplete(false);
    setContainerInfo(null);
    setFormData({
      username: "",
      email: "",
      password: "",
      plan_id: "plan_standard",
      node_mode: "auto",
      node_id: "node-sgp-01",
      primary_domain: "",
      create_vhost: true,
    });
    setOpSteps((prev) => prev.map((s) => ({ ...s, status: "pending" })));
  };

  const placementResult = selectPlacementNode(CLUSTER_NODES, "least-loaded");
  const autoSelectedNode = placementResult.selectedNode || CLUSTER_NODES[0];
  const selectedPlan = HOSTING_PLANS.find((p) => p.id === formData.plan_id) || HOSTING_PLANS[1];
  const selectedNode = CLUSTER_NODES.find((n) => n.id === formData.node_id) || autoSelectedNode;

  const stepsList = [
    { num: 1, label: t("userWizard.steps.account"), icon: User },
    { num: 2, label: t("userWizard.steps.plan"), icon: Package },
    { num: 3, label: t("userWizard.steps.node"), icon: Server },
    { num: 4, label: t("userWizard.steps.domain"), icon: Globe },
    { num: 5, label: t("userWizard.steps.confirm"), icon: CheckCircle2 },
  ];

  return (
    <div className="max-w-4xl mx-auto space-y-6 pb-12">
      {/* Page Header */}
      <div>
        <div className="flex items-center gap-2">
          <Sparkles className="h-6 w-6 text-primary" />
          <h1 className="text-2xl font-bold tracking-tight text-foreground">
            {t("userWizard.title")}
          </h1>
        </div>
        <p className="text-sm text-muted-foreground mt-1">{t("userWizard.desc")}</p>
      </div>

      {/* Wizard Progress Indicator */}
      {!isProvisioning && (
        <div className="flex items-center justify-between border bg-card p-4 rounded-xl shadow-sm">
          {stepsList.map((item, idx) => {
            const Icon = item.icon;
            const isCurrent = step === item.num;
            const isPassed = step > item.num;

            return (
              <React.Fragment key={item.num}>
                <div className="flex items-center gap-2.5">
                  <div
                    className={`h-8 w-8 rounded-full flex items-center justify-center text-xs font-bold transition-all ${
                      isCurrent
                        ? "bg-primary text-primary-foreground shadow-md ring-2 ring-primary/20"
                        : isPassed
                        ? "bg-emerald-500 text-white"
                        : "bg-muted text-muted-foreground"
                    }`}
                  >
                    {isPassed ? <CheckCircle2 className="h-4 w-4" /> : <Icon className="h-4 w-4" />}
                  </div>
                  <div className="hidden sm:block">
                    <p className={`text-xs font-semibold ${isCurrent ? "text-foreground" : "text-muted-foreground"}`}>
                      {item.label}
                    </p>
                  </div>
                </div>
                {idx < stepsList.length - 1 && (
                  <div
                    className={`flex-1 h-[2px] mx-2 sm:mx-3 transition-colors ${
                      step > item.num ? "bg-emerald-500" : "bg-muted"
                    }`}
                  />
                )}
              </React.Fragment>
            );
          })}
        </div>
      )}

      {/* Main Step Container */}
      {isProvisioning ? (
        <OperationProgress
          steps={opSteps}
          isComplete={isComplete}
          containerInfo={containerInfo}
          username={formData.username}
          onReset={handleReset}
          onNavigateUsers={() => (window.location.href = "/users")}
          onImpersonate={() => (window.location.href = `/client?user=${formData.username}`)}
        />
      ) : (
        <Card className="border bg-card shadow-sm">
          <CardHeader>
            <CardTitle>
              {step === 1 && t("userWizard.step1.title")}
              {step === 2 && t("userWizard.step2.title")}
              {step === 3 && t("userWizard.step3.title")}
              {step === 4 && t("userWizard.step4.title")}
              {step === 5 && t("userWizard.step5.title")}
            </CardTitle>
            <CardDescription>
              {step === 1 && t("userWizard.step1.desc")}
              {step === 2 && t("userWizard.step2.desc")}
              {step === 3 && t("userWizard.step3.desc")}
              {step === 4 && t("userWizard.step4.desc")}
              {step === 5 && t("userWizard.step5.desc")}
            </CardDescription>
          </CardHeader>

          <CardContent className="space-y-6">
            {/* Step 1: Account */}
            {step === 1 && (
              <div className="space-y-4">
                <div className="space-y-1.5">
                  <label htmlFor="wizard-username" className="text-sm font-medium text-foreground">
                    {t("userWizard.step1.usernameLabel")}{" "}
                    <span className="text-destructive font-bold">*</span>
                  </label>
                  <Input
                    id="wizard-username"
                    value={formData.username}
                    onChange={(e) => {
                      setFormData((f) => ({ ...f, username: e.target.value.toLowerCase() }));
                      if (errors.username) setErrors((err) => ({ ...err, username: "" }));
                    }}
                    placeholder={t("userWizard.step1.usernamePlaceholder")}
                    autoFocus
                  />
                  {errors.username ? (
                    <p className="text-xs text-destructive">{errors.username}</p>
                  ) : (
                    <p className="text-[11px] text-muted-foreground">
                      {t("userWizard.step1.usernameHelp")}
                    </p>
                  )}
                </div>

                <div className="space-y-1.5">
                  <label htmlFor="wizard-email" className="text-sm font-medium text-foreground">
                    {t("userWizard.step1.emailLabel")}{" "}
                    <span className="text-destructive font-bold">*</span>
                  </label>
                  <Input
                    id="wizard-email"
                    type="email"
                    value={formData.email}
                    onChange={(e) => {
                      setFormData((f) => ({ ...f, email: e.target.value }));
                      if (errors.email) setErrors((err) => ({ ...err, email: "" }));
                    }}
                    placeholder="client@example.com"
                  />
                  {errors.email && <p className="text-xs text-destructive">{errors.email}</p>}
                </div>

                <div className="space-y-1.5">
                  <div className="flex items-center justify-between">
                    <label htmlFor="wizard-password" className="text-sm font-medium text-foreground">
                      {t("userWizard.step1.passwordLabel")}{" "}
                      <span className="text-destructive font-bold">*</span>
                    </label>
                    <button
                      type="button"
                      onClick={() =>
                        setFormData((f) => ({ ...f, password: generateRandomPassword() }))
                      }
                      className="text-xs text-primary hover:underline flex items-center gap-1 font-medium"
                    >
                      <Key className="h-3 w-3" />
                      {t("userWizard.step1.generatePassword")}
                    </button>
                  </div>
                  <Input
                    id="wizard-password"
                    type="text"
                    value={formData.password}
                    onChange={(e) => {
                      setFormData((f) => ({ ...f, password: e.target.value }));
                      if (errors.password) setErrors((err) => ({ ...err, password: "" }));
                    }}
                    placeholder="••••••••"
                  />
                  {errors.password && <p className="text-xs text-destructive">{errors.password}</p>}
                </div>
              </div>
            )}

            {/* Step 2: Plan */}
            {step === 2 && (
              <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                {HOSTING_PLANS.map((plan) => {
                  const isSelected = formData.plan_id === plan.id;
                  return (
                    <div
                      key={plan.id}
                      onClick={() => setFormData((f) => ({ ...f, plan_id: plan.id }))}
                      className={`relative p-4 rounded-xl border cursor-pointer transition-all ${
                        isSelected
                          ? "border-primary bg-primary/5 ring-2 ring-primary/20 shadow-sm"
                          : "border-border bg-card hover:bg-muted/40"
                      }`}
                    >
                      <div className="flex items-start justify-between">
                        <div>
                          <div className="flex items-center gap-2">
                            <span className="font-semibold text-foreground text-sm">
                              {plan.name}
                            </span>
                            {plan.is_default && (
                              <Badge variant="secondary" className="text-[10px]">
                                Default
                              </Badge>
                            )}
                          </div>
                          <p className="text-xs text-muted-foreground mt-1">
                            {plan.description}
                          </p>
                        </div>
                        <input
                          type="radio"
                          name="selected_plan"
                          checked={isSelected}
                          onChange={() => setFormData((f) => ({ ...f, plan_id: plan.id }))}
                          className="h-4 w-4 text-primary focus:ring-primary cursor-pointer mt-0.5"
                        />
                      </div>

                      <div className="grid grid-cols-4 gap-2 mt-4 pt-3 border-t text-center text-xs">
                        <div>
                          <span className="text-[10px] text-muted-foreground block">CPU</span>
                          <span className="font-bold font-mono">
                            {plan.cpu === -1 ? "∞" : `${plan.cpu} Core`}
                          </span>
                        </div>
                        <div>
                          <span className="text-[10px] text-muted-foreground block">Memory</span>
                          <span className="font-bold font-mono">
                            {plan.memory_mb === -1 ? "∞" : `${plan.memory_mb / 1024} GB`}
                          </span>
                        </div>
                        <div>
                          <span className="text-[10px] text-muted-foreground block">Disk</span>
                          <span className="font-bold font-mono">
                            {plan.disk_gb === -1 ? "∞" : `${plan.disk_gb} GB`}
                          </span>
                        </div>
                        <div>
                          <span className="text-[10px] text-muted-foreground block">Domains</span>
                          <span className="font-bold font-mono">
                            {plan.domains_limit === -1 ? "∞" : plan.domains_limit}
                          </span>
                        </div>
                      </div>
                    </div>
                  );
                })}
              </div>
            )}

            {/* Step 3: Node Allocation */}
            {step === 3 && (
              <div className="space-y-4">
                {/* Auto Mode Option */}
                <div
                  onClick={() => {
                    setErrors({});
                    setFormData((f) => ({ ...f, node_mode: "auto" }));
                  }}
                  className={`p-4 rounded-xl border cursor-pointer transition-all ${
                    formData.node_mode === "auto"
                      ? "border-primary bg-primary/5 ring-2 ring-primary/20"
                      : "border-border bg-card hover:bg-muted/40"
                  }`}
                >
                  <div className="flex items-center justify-between">
                    <div className="flex items-center gap-3">
                      <div className="h-9 w-9 rounded-full bg-emerald-500/10 text-emerald-600 flex items-center justify-center">
                        <Sparkles className="h-5 w-5" />
                      </div>
                      <div>
                        <div className="flex items-center gap-2">
                          <p className="text-sm font-semibold text-foreground">
                            {t("userWizard.step3.autoLabel")}
                          </p>
                          <Badge variant="default" className="bg-emerald-500/20 text-emerald-600 dark:text-emerald-400 border-emerald-500/30 text-[10px]">
                            Recommended
                          </Badge>
                        </div>
                        <p className="text-xs text-muted-foreground mt-0.5">
                          {t("userWizard.step3.autoDesc")} (Selected: <strong className="text-foreground font-mono">{autoSelectedNode.name}</strong> • {placementResult.reason})
                        </p>
                        {placementResult.excludedNodes.length > 0 && (
                          <p className="text-[11px] text-amber-500 flex items-center gap-1 mt-1 font-medium">
                            <AlertTriangle className="h-3 w-3" />
                            {t("userWizard.step3.excludedDrainingNotice", { count: placementResult.excludedNodes.length })}
                          </p>
                        )}
                      </div>
                    </div>
                    <input
                      type="radio"
                      name="node_mode"
                      checked={formData.node_mode === "auto"}
                      onChange={() => {
                        setErrors({});
                        setFormData((f) => ({ ...f, node_mode: "auto" }));
                      }}
                      className="h-4 w-4 text-primary focus:ring-primary cursor-pointer"
                    />
                  </div>
                </div>

                {/* Manual Node Selection */}
                <div className="space-y-2 pt-2">
                  <p className="text-xs font-semibold uppercase text-muted-foreground">
                    {t("userWizard.step3.orSelectManual")}
                  </p>
                  <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-3">
                    {CLUSTER_NODES.map((node) => {
                      const isSelected =
                        formData.node_mode === "manual" && formData.node_id === node.id;
                      const isDraining = node.status === "draining";

                      return (
                        <div
                          key={node.id}
                          data-testid={isDraining ? "node-item-draining" : `node-item-${node.id}`}
                          onClick={() => {
                            if (isDraining) {
                              setErrors({ node: t("userWizard.step3.nodeDrainingError") });
                              setFormData((f) => ({ ...f, node_mode: "manual", node_id: node.id }));
                              return;
                            }
                            setErrors({});
                            setFormData((f) => ({ ...f, node_mode: "manual", node_id: node.id }));
                          }}
                          className={`p-3.5 rounded-xl border transition-all ${
                            isDraining
                              ? "border-amber-500/30 bg-muted/40 opacity-70 cursor-not-allowed"
                              : isSelected
                              ? "border-primary bg-primary/5 ring-2 ring-primary/20 cursor-pointer"
                              : "border-border bg-card hover:bg-muted/40 cursor-pointer"
                          }`}
                        >
                          <div className="flex items-center justify-between">
                            <span className="font-mono text-xs font-bold text-foreground">
                              {node.name}
                            </span>
                            {isDraining ? (
                              <Badge variant="outline" className="text-[10px] text-amber-500 border-amber-500 py-0">
                                Draining
                              </Badge>
                            ) : (
                              <span className="h-2 w-2 rounded-full bg-emerald-500" />
                            )}
                          </div>
                          <p className="text-[11px] text-muted-foreground mt-0.5 truncate">
                            {node.region}
                          </p>
                          {isDraining ? (
                            <p className="mt-2 text-[10px] text-amber-500 font-medium">
                              {t("userWizard.step3.nodeDrainingTooltip")}
                            </p>
                          ) : (
                            <div className="mt-3 pt-2 border-t flex justify-between text-[11px] text-muted-foreground">
                              <span>CPU: {node.cpu_usage_pct}%</span>
                              <span>RAM: {node.memory_usage_pct}%</span>
                            </div>
                          )}
                        </div>
                      );
                    })}
                  </div>
                  {errors.node && (
                    <p className="text-xs text-destructive font-medium mt-1.5" data-testid="node-selection-error">
                      {errors.node}
                    </p>
                  )}
                </div>
              </div>
            )}

            {/* Step 4: Primary Domain */}
            {step === 4 && (
              <div className="space-y-5">
                <div className="space-y-1.5">
                  <label htmlFor="wizard-domain" className="text-sm font-medium text-foreground">
                    {t("userWizard.step4.domainLabel")}
                  </label>
                  <Input
                    id="wizard-domain"
                    value={formData.primary_domain || ""}
                    onChange={(e) => {
                      setFormData((f) => ({ ...f, primary_domain: e.target.value.toLowerCase() }));
                      if (errors.primary_domain) setErrors((err) => ({ ...err, primary_domain: "" }));
                    }}
                    placeholder="mywebsite.com"
                  />
                  {errors.primary_domain ? (
                    <p className="text-xs text-destructive">{errors.primary_domain}</p>
                  ) : (
                    <p className="text-[11px] text-muted-foreground">
                      {t("userWizard.step4.domainHelp")}
                    </p>
                  )}
                </div>

                <div className="flex items-center gap-3 p-3.5 rounded-xl border bg-muted/20">
                  <input
                    type="checkbox"
                    id="create-vhost"
                    checked={formData.create_vhost}
                    onChange={(e) =>
                      setFormData((f) => ({ ...f, create_vhost: e.target.checked }))
                    }
                    className="h-4 w-4 rounded border-input text-primary focus:ring-primary cursor-pointer"
                  />
                  <label htmlFor="create-vhost" className="text-xs text-foreground cursor-pointer select-none">
                    <span className="font-semibold block">{t("userWizard.step4.vhostLabel")}</span>
                    <span className="text-muted-foreground text-[11px]">
                      {t("userWizard.step4.vhostDesc")}
                    </span>
                  </label>
                </div>
              </div>
            )}

            {/* Step 5: Review & Confirm */}
            {step === 5 && (
              <div className="space-y-4">
                <div className="rounded-xl border bg-card p-4 space-y-3">
                  <h3 className="text-sm font-semibold text-foreground uppercase tracking-tight flex items-center gap-2 border-b pb-2">
                    <Info className="h-4 w-4 text-primary" />
                    <span>{t("userWizard.step5.summaryHeader")}</span>
                  </h3>

                  <div className="grid grid-cols-2 md:grid-cols-4 gap-4 text-xs pt-1">
                    <div>
                      <span className="text-muted-foreground block text-[11px]">Username</span>
                      <span className="font-bold text-foreground font-mono">{formData.username}</span>
                    </div>
                    <div>
                      <span className="text-muted-foreground block text-[11px]">Email</span>
                      <span className="font-semibold text-foreground truncate block">{formData.email}</span>
                    </div>
                    <div>
                      <span className="text-muted-foreground block text-[11px]">Hosting Plan</span>
                      <Badge variant="outline" className="font-normal text-[11px] mt-0.5">
                        {selectedPlan.name}
                      </Badge>
                    </div>
                    <div>
                      <span className="text-muted-foreground block text-[11px]">Target Node</span>
                      <span className="font-semibold text-foreground font-mono">
                        {formData.node_mode === "auto" ? `Auto (${autoSelectedNode.name})` : selectedNode.name}
                      </span>
                    </div>
                  </div>

                  <div className="pt-3 border-t grid grid-cols-2 gap-4 text-xs">
                    <div>
                      <span className="text-muted-foreground block text-[11px]">Primary Domain</span>
                      <span className="font-semibold text-foreground">
                        {formData.primary_domain || "None (Will add later)"}
                      </span>
                    </div>
                    <div>
                      <span className="text-muted-foreground block text-[11px]">Container Isolation</span>
                      <span className="font-semibold text-emerald-600 dark:text-emerald-400">
                        Rootless Container & XFS Quota
                      </span>
                    </div>
                  </div>
                </div>
              </div>
            )}
          </CardContent>

          <CardFooter className="flex items-center justify-between border-t p-4 bg-muted/10">
            {step > 1 ? (
              <Button variant="outline" size="sm" onClick={handleBack} disabled={isSubmitting} className="gap-2">
                <ArrowLeft className="h-4 w-4" />
                {t("common.back")}
              </Button>
            ) : (
              <div />
            )}

            {step < 5 ? (
              <Button size="sm" onClick={handleNext} className="gap-2">
                {t("common.next")}
                <ArrowRight className="h-4 w-4" />
              </Button>
            ) : (
              <Button size="sm" onClick={handleCreateUser} disabled={isSubmitting} className="gap-2">
                <CheckCircle2 className="h-4 w-4" />
                {t("userWizard.step5.confirmButton")}
              </Button>
            )}
          </CardFooter>
        </Card>
      )}
    </div>
  );
}
