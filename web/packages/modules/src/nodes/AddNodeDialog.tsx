import React, { useState, useEffect } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  Badge,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch, getCsrfToken } from "@builtpanel/api";
import { Server, Copy, Check, Terminal, ShieldCheck, Loader2, KeyRound } from "lucide-react";
import type { CreatedEnrollment } from "./types";

interface AddNodeDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onEnrolled?: () => void;
}

export function AddNodeDialog({ open, onOpenChange, onEnrolled }: AddNodeDialogProps) {
  const { t } = useTranslation();
  const [nodeName, setNodeName] = useState("");
  const [step, setStep] = useState<"input" | "sudo" | "success">("input");
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [sudoPassword, setSudoPassword] = useState("");
  const [sudoError, setSudoError] = useState<string | null>(null);
  const [data, setData] = useState<CreatedEnrollment | null>(null);
  const [copiedCmd, setCopiedCmd] = useState(false);
  const [copiedToken, setCopiedToken] = useState(false);

  useEffect(() => {
    if (open) {
      setNodeName("");
      setStep("input");
      setLoading(false);
      setError(null);
      setSudoPassword("");
      setSudoError(null);
      setData(null);
      setCopiedCmd(false);
      setCopiedToken(false);
    }
  }, [open]);

  const handleCreateNode = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    const name = nodeName.trim();
    if (!name) return;

    try {
      setLoading(true);
      setError(null);
      const res = await apiFetch<CreatedEnrollment>("/api/v1/admin/nodes", {
        method: "POST",
        headers: {
          "X-CSRF-Token": getCsrfToken() || "",
        },
        body: JSON.stringify({ name }),
      });
      setData(res);
      setStep("success");
    } catch (err: any) {
      const isReauth =
        err?.status === 403 &&
        (err?.details?.code === "reauth_required" ||
          err?.details?.title === "reauth_required" ||
          err?.message?.includes("reauth_required"));

      if (isReauth) {
        setStep("sudo");
      } else {
        setError(err?.details?.detail || err?.message || t("common.error"));
      }
    } finally {
      setLoading(false);
    }
  };

  const handleVerifySudo = async (e?: React.FormEvent) => {
    if (e) e.preventDefault();
    if (!sudoPassword) return;

    try {
      setLoading(true);
      setSudoError(null);
      await apiFetch("/api/v1/auth/sudo", {
        method: "POST",
        headers: {
          "X-CSRF-Token": getCsrfToken() || "",
        },
        body: JSON.stringify({ password: sudoPassword }),
      });

      // Sudo verified, retry creating node
      const res = await apiFetch<CreatedEnrollment>("/api/v1/admin/nodes", {
        method: "POST",
        headers: {
          "X-CSRF-Token": getCsrfToken() || "",
        },
        body: JSON.stringify({ name: nodeName.trim() }),
      });
      setData(res);
      setStep("success");
    } catch (err: any) {
      setSudoError(err?.details?.detail || err?.message || t("common.error"));
    } finally {
      setLoading(false);
    }
  };

  const copyCommand = () => {
    if (!data?.enroll_command) return;
    navigator.clipboard?.writeText(data.enroll_command);
    setCopiedCmd(true);
    setTimeout(() => setCopiedCmd(false), 2000);
  };

  const copyToken = () => {
    if (!data?.token) return;
    navigator.clipboard?.writeText(data.token);
    setCopiedToken(true);
    setTimeout(() => setCopiedToken(false), 2000);
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-xl">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <Server className="h-5 w-5 text-primary" />
            <DialogTitle>{t("nodes.wizard.title")}</DialogTitle>
          </div>
          <DialogDescription>{t("nodes.wizard.desc")}</DialogDescription>
        </DialogHeader>

        <div className="py-2">
          {step === "input" && (
            <form onSubmit={handleCreateNode} className="space-y-4">
              <div className="space-y-2">
                <label htmlFor="node-name-input" className="text-sm font-medium leading-none">
                  {t("nodes.wizard.nameLabel")}
                </label>
                <Input
                  id="node-name-input"
                  value={nodeName}
                  onChange={(e) => setNodeName(e.target.value)}
                  placeholder={t("nodes.wizard.namePlaceholder")}
                  disabled={loading}
                  autoFocus
                />
              </div>

              {error && (
                <div className="p-3 rounded-lg bg-destructive/10 text-destructive text-xs">
                  {error}
                </div>
              )}

              <DialogFooter className="pt-2 gap-2">
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => onOpenChange(false)}
                  disabled={loading}
                >
                  {t("common.cancel")}
                </Button>
                <Button type="submit" disabled={!nodeName.trim() || loading} className="gap-2">
                  {loading && <Loader2 className="h-4 w-4 animate-spin" />}
                  {loading ? t("nodes.wizard.generating") : t("nodes.wizard.generateToken")}
                </Button>
              </DialogFooter>
            </form>
          )}

          {step === "sudo" && (
            <form onSubmit={handleVerifySudo} className="space-y-4">
              <div className="flex items-start gap-3 p-3 rounded-lg bg-amber-500/10 border border-amber-500/20 text-xs text-amber-900 dark:text-amber-200">
                <KeyRound className="h-4 w-4 shrink-0 text-amber-600 dark:text-amber-400 mt-0.5" />
                <p>{t("nodes.wizard.sudoPrompt")}</p>
              </div>

              <div className="space-y-2">
                <label htmlFor="sudo-password-input" className="text-sm font-medium leading-none">
                  {t("nodes.wizard.passwordLabel")}
                </label>
                <Input
                  id="sudo-password-input"
                  type="password"
                  value={sudoPassword}
                  onChange={(e) => setSudoPassword(e.target.value)}
                  placeholder="••••••••••••"
                  disabled={loading}
                  autoFocus
                />
              </div>

              {sudoError && (
                <div className="p-3 rounded-lg bg-destructive/10 text-destructive text-xs">
                  {sudoError}
                </div>
              )}

              <DialogFooter className="pt-2 gap-2">
                <Button
                  type="button"
                  variant="outline"
                  onClick={() => setStep("input")}
                  disabled={loading}
                >
                  {t("common.back")}
                </Button>
                <Button type="submit" disabled={!sudoPassword || loading} className="gap-2">
                  {loading && <Loader2 className="h-4 w-4 animate-spin" />}
                  {t("nodes.wizard.verifySudo")}
                </Button>
              </DialogFooter>
            </form>
          )}

          {step === "success" && data && (
            <div className="space-y-4">
              {/* Token Display */}
              <div className="p-3.5 rounded-lg border bg-muted/30 space-y-2 text-xs">
                <div className="flex items-center justify-between">
                  <div className="flex items-center gap-2">
                    <span className="font-semibold text-foreground">{data.name}</span>
                    <Badge variant="outline" className="text-[10px] text-amber-500 border-amber-500 font-mono">
                      {t("nodes.wizard.expiresIn")}
                    </Badge>
                  </div>
                  <Button
                    type="button"
                    variant="outline"
                    size="sm"
                    onClick={copyToken}
                    className="h-7 px-2.5 text-xs gap-1"
                  >
                    {copiedToken ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                    {copiedToken ? t("nodes.wizard.copied") : t("nodes.wizard.copyToken")}
                  </Button>
                </div>
                <p className="font-mono text-xs break-all bg-muted/60 p-2 rounded border">
                  {data.token}
                </p>
              </div>

              {/* Install Command (shown when enroll_command is present) */}
              {data.enroll_command && (
                <div className="space-y-2">
                  <div className="flex items-center justify-between">
                    <label className="text-xs font-semibold flex items-center gap-1.5 text-muted-foreground uppercase">
                      <Terminal className="h-3.5 w-3.5 text-primary" />
                      {t("nodes.wizard.commandLabel")}
                    </label>
                    <Button
                      type="button"
                      variant="outline"
                      size="sm"
                      onClick={copyCommand}
                      className="h-7 px-2.5 text-xs gap-1"
                    >
                      {copiedCmd ? <Check className="h-3.5 w-3.5 text-emerald-500" /> : <Copy className="h-3.5 w-3.5" />}
                      {copiedCmd ? t("nodes.wizard.copied") : t("nodes.wizard.copyCommand")}
                    </Button>
                  </div>
                  <pre className="p-3.5 rounded-lg bg-zinc-950 text-zinc-100 font-mono text-xs overflow-x-auto select-all border">
                    <code>{data.enroll_command}</code>
                  </pre>
                </div>
              )}

              {/* Security Notice */}
              <div className="flex items-start gap-2.5 p-3 rounded-lg bg-primary/5 border border-primary/10 text-xs text-muted-foreground">
                <ShieldCheck className="h-4 w-4 text-primary shrink-0 mt-0.5" />
                <p>{t("nodes.wizard.securityNotice")}</p>
              </div>

              <DialogFooter className="pt-2">
                <Button
                  type="button"
                  onClick={() => {
                    onOpenChange(false);
                    onEnrolled?.();
                  }}
                >
                  {t("nodes.wizard.done")}
                </Button>
              </DialogFooter>
            </div>
          )}
        </div>
      </DialogContent>
    </Dialog>
  );
}
