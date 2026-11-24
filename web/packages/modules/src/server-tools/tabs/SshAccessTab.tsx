import React, { useState, useEffect } from "react";
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
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import {
  Terminal,
  ShieldAlert,
  Key,
  FileCode,
  Sliders,
  Plus,
  Trash2,
  CheckCircle2,
  AlertTriangle,
  Loader2,
  Save,
  RotateCcw,
} from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { SshBasicConfig, SshAuthorizedKey } from "../types";
import {
  loadSshBasicConfig,
  updateSshBasicConfig,
  loadSshKeys,
  addSshKey,
  deleteSshKey,
  loadSshRawConfig,
  updateSshRawConfig,
} from "../api";
import { SudoConfirmModal } from "../SudoConfirmModal";

interface SshAccessTabProps {
  nodeId: string;
  canManage: boolean;
  onAuditActionTriggered?: () => void;
}

export function SshAccessTab({ nodeId, canManage, onAuditActionTriggered }: SshAccessTabProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [subTab, setSubTab] = useState<"basic" | "keys" | "advanced">("basic");
  const [isLoading, setIsLoading] = useState(false);

  // Basic Config State
  const [basicConfig, setBasicConfig] = useState<SshBasicConfig>({
    port: 22,
    permit_root_login: "no",
    password_authentication: false,
    pubkey_authentication: true,
    max_auth_tries: 6,
  });
  const [initialBasicConfig, setInitialBasicConfig] = useState<SshBasicConfig>({ ...basicConfig });

  // Keys State
  const [keys, setKeys] = useState<SshAuthorizedKey[]>([]);
  const [isAddKeyOpen, setIsAddKeyOpen] = useState(false);
  const [newKeyName, setNewKeyName] = useState("");
  const [newPublicKey, setNewPublicKey] = useState("");
  const [keyToDelete, setKeyToDelete] = useState<SshAuthorizedKey | null>(null);

  // Advanced Raw Config State
  const [rawConfig, setRawConfig] = useState("");
  const [initialRawConfig, setInitialRawConfig] = useState("");

  // Sudo confirmation states
  const [pendingAction, setPendingAction] = useState<
    | { type: "basic"; config: SshBasicConfig }
    | { type: "add_key"; name: string; public_key: string }
    | { type: "delete_key"; keyId: string }
    | { type: "raw"; raw: string }
    | null
  >(null);

  const fetchAll = async () => {
    if (!nodeId) return;
    try {
      setIsLoading(true);
      const [cfg, keyList, raw] = await Promise.all([
        loadSshBasicConfig(nodeId),
        loadSshKeys(nodeId),
        loadSshRawConfig(nodeId),
      ]);
      setBasicConfig(cfg);
      setInitialBasicConfig(cfg);
      setKeys(keyList);
      setRawConfig(raw);
      setInitialRawConfig(raw);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsLoading(false);
    }
  };

  useEffect(() => {
    void fetchAll();
  }, [nodeId]);

  const hasBasicChanged = JSON.stringify(basicConfig) !== JSON.stringify(initialBasicConfig);
  const hasRawChanged = rawConfig !== initialRawConfig;

  const handleSudoConfirm = async (password: string) => {
    if (!pendingAction) return;

    try {
      if (pendingAction.type === "basic") {
        await updateSshBasicConfig(nodeId, pendingAction.config, password);
        setInitialBasicConfig({ ...pendingAction.config });
        toast({
          title: t("common.success"),
          description: t("system.ssh.basicUpdatedSuccess", "SSH daemon configuration successfully validated and updated."),
        });
      } else if (pendingAction.type === "add_key") {
        await addSshKey(nodeId, { name: pendingAction.name, public_key: pendingAction.public_key }, password);
        setNewKeyName("");
        setNewPublicKey("");
        setIsAddKeyOpen(false);
        const updatedKeys = await loadSshKeys(nodeId);
        setKeys(updatedKeys);
        toast({
          title: t("common.success"),
          description: t("system.ssh.keyAddedSuccess", "Authorized public key added to server."),
        });
      } else if (pendingAction.type === "delete_key") {
        await deleteSshKey(nodeId, pendingAction.keyId, password);
        setKeyToDelete(null);
        const updatedKeys = await loadSshKeys(nodeId);
        setKeys(updatedKeys);
        toast({
          title: t("common.success"),
          description: t("system.ssh.keyDeletedSuccess", "Authorized public key removed."),
        });
      } else if (pendingAction.type === "raw") {
        await updateSshRawConfig(nodeId, pendingAction.raw, password);
        setInitialRawConfig(pendingAction.raw);
        toast({
          title: t("common.success"),
          description: t("system.ssh.rawUpdatedSuccess", "sshd_config verified and reloaded successfully."),
        });
      }

      onAuditActionTriggered?.();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
      throw err;
    } finally {
      setPendingAction(null);
    }
  };

  return (
    <div className="space-y-6">
      {/* Lock-out Warning Banner (Mandatory Rule) */}
      <div className="rounded-lg border border-amber-500/30 bg-amber-500/10 p-4 text-xs text-amber-800 dark:text-amber-300" data-testid="ssh-lockout-warning">
        <div className="flex items-start gap-3">
          <AlertTriangle className="h-5 w-5 text-amber-600 dark:text-amber-400 shrink-0 mt-0.5" />
          <div className="space-y-1">
            <h4 className="font-semibold text-sm text-foreground">
              {t("system.ssh.lockoutWarningTitle", "Critical SSH Access & Lock-out Notice")}
            </h4>
            <p className="leading-relaxed">
              {t(
                "system.ssh.lockoutWarningBody",
                "Modifying SSH port, root login, or authentication mechanisms can unintentionally cut off administrator access. BuiltPanel backend automatically performs strict syntax checking and validates connectivity before applying changes; if verification fails, the previous configuration is automatically rolled back.",
              )}
            </p>
          </div>
        </div>
      </div>

      {/* Sub-tabs header */}
      <div className="flex items-center gap-2 border-b pb-2">
        <button
          type="button"
          data-testid="subtab-ssh-basic"
          onClick={() => setSubTab("basic")}
          className={`flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
            subTab === "basic"
              ? "bg-primary text-primary-foreground font-semibold shadow-sm"
              : "text-muted-foreground hover:bg-muted hover:text-foreground"
          }`}
        >
          <Sliders className="h-3.5 w-3.5" />
          <span>{t("system.ssh.subtabBasic", "Basic Settings")}</span>
        </button>

        <button
          type="button"
          data-testid="subtab-ssh-keys"
          onClick={() => setSubTab("keys")}
          className={`flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
            subTab === "keys"
              ? "bg-primary text-primary-foreground font-semibold shadow-sm"
              : "text-muted-foreground hover:bg-muted hover:text-foreground"
          }`}
        >
          <Key className="h-3.5 w-3.5" />
          <span>{t("system.ssh.subtabKeys", "Authorized Keys")}</span>
          <span className="rounded-full bg-muted/60 px-1.5 py-0.2 text-[10px] font-mono">
            {keys.length}
          </span>
        </button>

        <button
          type="button"
          data-testid="subtab-ssh-advanced"
          onClick={() => setSubTab("advanced")}
          className={`flex items-center gap-1.5 px-3 py-1.5 text-xs font-medium rounded-md transition-colors ${
            subTab === "advanced"
              ? "bg-primary text-primary-foreground font-semibold shadow-sm"
              : "text-muted-foreground hover:bg-muted hover:text-foreground"
          }`}
        >
          <FileCode className="h-3.5 w-3.5" />
          <span>{t("system.ssh.subtabAdvanced", "Advanced (sshd_config)")}</span>
        </button>
      </div>

      {/* 1. Basic Sub-tab */}
      {subTab === "basic" && (
        <Card data-testid="card-ssh-basic">
          <CardHeader>
            <CardTitle className="text-base font-semibold flex items-center gap-2">
              <Sliders className="h-4 w-4 text-primary" />
              <span>{t("system.ssh.basicConfigTitle", "Primary SSH Daemon Security Parameters")}</span>
            </CardTitle>
            <CardDescription className="text-xs text-muted-foreground">
              {t("system.ssh.basicConfigDesc", "Configure listening port, root user login permissions, and allowed credential authentication types")}
            </CardDescription>
          </CardHeader>

          <CardContent className="space-y-4">
            <div className="grid grid-cols-1 gap-4 sm:grid-cols-2">
              {/* SSH Port */}
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">
                  {t("serverTools.sshPort", "SSH Port")}
                </label>
                <Input
                  data-testid="input-ssh-port"
                  type="number"
                  min={1}
                  max={65535}
                  value={basicConfig.port}
                  onChange={(e) => setBasicConfig({ ...basicConfig, port: Number(e.target.value) || 22 })}
                  className="font-mono text-xs"
                />
                <span className="text-[11px] text-muted-foreground">
                  {t("system.ssh.portHelp", "Default standard port is 22. Custom ports must be allowed in firewall.")}
                </span>
              </div>

              {/* Max Auth Tries */}
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">
                  {t("serverTools.maxAuthTries", "Max Auth Tries")}
                </label>
                <Input
                  data-testid="input-ssh-max-tries"
                  type="number"
                  min={1}
                  max={20}
                  value={basicConfig.max_auth_tries || 6}
                  onChange={(e) => setBasicConfig({ ...basicConfig, max_auth_tries: Number(e.target.value) || 6 })}
                  className="font-mono text-xs"
                />
                <span className="text-[11px] text-muted-foreground">
                  {t("system.ssh.maxTriesHelp", "Maximum connection attempts before daemon disconnects.")}
                </span>
              </div>
            </div>

            <div className="grid grid-cols-1 gap-4 sm:grid-cols-3 pt-2">
              {/* Root Login */}
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">
                  {t("serverTools.permitRootLogin", "PermitRootLogin")}
                </label>
                <select
                  data-testid="select-permit-root-login"
                  value={basicConfig.permit_root_login}
                  onChange={(e) =>
                    setBasicConfig({
                      ...basicConfig,
                      permit_root_login: e.target.value as "yes" | "no" | "prohibit-password",
                    })
                  }
                  className="w-full rounded-md border bg-background px-3 py-1.5 text-xs font-mono shadow-sm focus:outline-none"
                >
                  <option value="no">{t("system.ssh.rootNo", "no (Recommended - Disabled)")}</option>
                  <option value="prohibit-password">{t("system.ssh.rootKeyOnly", "prohibit-password (Keys only)")}</option>
                  <option value="yes">{t("system.ssh.rootYes", "yes (Allow password & keys)")}</option>
                </select>
              </div>

              {/* Password Authentication */}
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">
                  {t("serverTools.passwordAuth", "PasswordAuthentication")}
                </label>
                <select
                  data-testid="select-password-auth"
                  value={basicConfig.password_authentication ? "yes" : "no"}
                  onChange={(e) =>
                    setBasicConfig({
                      ...basicConfig,
                      password_authentication: e.target.value === "yes",
                    })
                  }
                  className="w-full rounded-md border bg-background px-3 py-1.5 text-xs font-mono shadow-sm focus:outline-none"
                >
                  <option value="no">{t("common.disabled", "Disabled (Keys Only - Recommended)")}</option>
                  <option value="yes">{t("common.enabled", "Enabled (Allow Password Login)")}</option>
                </select>
              </div>

              {/* Pubkey Authentication */}
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">
                  {t("system.ssh.pubkeyAuth", "PubkeyAuthentication")}
                </label>
                <select
                  data-testid="select-pubkey-auth"
                  value={basicConfig.pubkey_authentication ? "yes" : "no"}
                  onChange={(e) =>
                    setBasicConfig({
                      ...basicConfig,
                      pubkey_authentication: e.target.value === "yes",
                    })
                  }
                  className="w-full rounded-md border bg-background px-3 py-1.5 text-xs font-mono shadow-sm focus:outline-none"
                >
                  <option value="yes">{t("common.enabled", "Enabled (Required for SSH Keys)")}</option>
                  <option value="no">{t("common.disabled", "Disabled")}</option>
                </select>
              </div>
            </div>
          </CardContent>

          <CardFooter className="flex items-center justify-between border-t px-6 py-3">
            <span className="text-[11px] text-muted-foreground">
              {hasBasicChanged
                ? t("common.unsavedChanges", "You have unsaved changes.")
                : t("system.ssh.configSynced", "Configuration is synchronized with node.")}
            </span>

            <Button
              size="sm"
              disabled={!hasBasicChanged || !canManage}
              data-testid="btn-save-ssh-basic"
              onClick={() => setPendingAction({ type: "basic", config: basicConfig })}
              className="gap-1.5 text-xs"
            >
              <Save className="h-3.5 w-3.5" />
              <span>{t("common.save", "Save Changes")}</span>
            </Button>
          </CardFooter>
        </Card>
      )}

      {/* 2. Authorized Keys Sub-tab */}
      {subTab === "keys" && (
        <Card data-testid="card-ssh-keys">
          <CardHeader>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <CardTitle className="text-base font-semibold flex items-center gap-2">
                  <Key className="h-4 w-4 text-primary" />
                  <span>{t("system.ssh.authorizedKeysTitle", "Authorized Public SSH Keys")}</span>
                </CardTitle>
                <CardDescription className="text-xs text-muted-foreground">
                  {t("system.ssh.authorizedKeysDesc", "Manage public keys authorized to log in directly into this host")}
                </CardDescription>
              </div>
              <Button
                size="sm"
                variant="outline"
                disabled={!canManage}
                data-testid="btn-open-add-key"
                onClick={() => setIsAddKeyOpen(!isAddKeyOpen)}
                className="gap-1.5 text-xs"
              >
                <Plus className="h-3.5 w-3.5" />
                <span>{isAddKeyOpen ? t("common.cancel", "Cancel") : t("system.ssh.addKey", "Add Public Key")}</span>
              </Button>
            </div>
          </CardHeader>

          <CardContent className="space-y-4">
            {/* Inline Add Key Form */}
            {isAddKeyOpen && (
              <div className="rounded-lg border bg-muted/40 p-4 space-y-3" data-testid="form-add-ssh-key">
                <h5 className="font-semibold text-xs text-foreground">
                  {t("system.ssh.addNewKeyHeading", "Authorize New Public Key")}
                </h5>
                <div className="space-y-2">
                  <Input
                    data-testid="input-new-key-name"
                    placeholder={t("system.ssh.keyNamePlaceholder", "Key identifier (e.g. dev-laptop-ed25519)")}
                    value={newKeyName}
                    onChange={(e) => setNewKeyName(e.target.value)}
                    className="text-xs"
                  />
                  <textarea
                    data-testid="textarea-new-public-key"
                    placeholder="ssh-ed25519 AAAAC3NzaC1lZDI1NTE5AAAAI... user@workstation"
                    rows={3}
                    value={newPublicKey}
                    onChange={(e) => setNewPublicKey(e.target.value)}
                    className="w-full rounded-md border bg-background p-2.5 font-mono text-xs focus:outline-none"
                  />
                </div>
                <div className="flex justify-end gap-2">
                  <Button
                    size="sm"
                    variant="outline"
                    onClick={() => {
                      setIsAddKeyOpen(false);
                      setNewKeyName("");
                      setNewPublicKey("");
                    }}
                  >
                    {t("common.cancel", "Cancel")}
                  </Button>
                  <Button
                    size="sm"
                    disabled={!newPublicKey.trim()}
                    data-testid="btn-submit-add-key"
                    onClick={() =>
                      setPendingAction({
                        type: "add_key",
                        name: newKeyName.trim() || "ssh-key-" + Date.now(),
                        public_key: newPublicKey.trim(),
                      })
                    }
                  >
                    {t("system.ssh.saveKeyButton", "Authorize Key")}
                  </Button>
                </div>
              </div>
            )}

            {/* Keys Table */}
            {keys.length === 0 ? (
              <p className="py-6 text-center text-xs text-muted-foreground">
                {t("system.ssh.noKeysFound", "No authorized keys installed on this server.")}
              </p>
            ) : (
              <div className="overflow-x-auto rounded-md border">
                <Table>
                  <TableHeader>
                    <TableRow>
                      <TableHead className="text-xs">{t("common.name", "Name")}</TableHead>
                      <TableHead className="text-xs">{t("system.ssh.keyType", "Type")}</TableHead>
                      <TableHead className="text-xs">{t("system.ssh.fingerprint", "Fingerprint")}</TableHead>
                      <TableHead className="text-xs text-right">{t("common.actions", "Actions")}</TableHead>
                    </TableRow>
                  </TableHeader>
                  <TableBody>
                    {keys.map((k) => (
                      <TableRow key={k.id} data-testid={`ssh-key-row-${k.id}`}>
                        <TableCell className="font-medium text-xs">
                          {k.name}
                        </TableCell>
                        <TableCell>
                          <Badge variant="outline" className="font-mono text-[10px]">
                            {k.key_type}
                          </Badge>
                        </TableCell>
                        <TableCell className="font-mono text-[11px] text-muted-foreground truncate max-w-xs">
                          {k.fingerprint}
                        </TableCell>
                        <TableCell className="text-right">
                          <Button
                            variant="destructive"
                            size="sm"
                            disabled={!canManage}
                            data-testid={`btn-delete-key-${k.id}`}
                            onClick={() => {
                              setKeyToDelete(k);
                              setPendingAction({ type: "delete_key", keyId: k.id });
                            }}
                            className="h-7 px-2 text-xs"
                          >
                            <Trash2 className="h-3.5 w-3.5" />
                          </Button>
                        </TableCell>
                      </TableRow>
                    ))}
                  </TableBody>
                </Table>
              </div>
            )}
          </CardContent>
        </Card>
      )}

      {/* 3. Advanced Sub-tab */}
      {subTab === "advanced" && (
        <Card data-testid="card-ssh-advanced">
          <CardHeader>
            <div className="flex flex-col gap-2 sm:flex-row sm:items-center sm:justify-between">
              <div>
                <CardTitle className="text-base font-semibold flex items-center gap-2">
                  <FileCode className="h-4 w-4 text-primary" />
                  <span>{t("system.ssh.rawConfigTitle", "Direct /etc/ssh/sshd_config Editor")}</span>
                </CardTitle>
                <CardDescription className="text-xs text-muted-foreground">
                  {t(
                    "system.ssh.rawConfigDesc",
                    "Direct text editing of the OpenSSH server configuration. Changes are strictly syntax-validated by sshd -t prior to applying.",
                  )}
                </CardDescription>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={() => setRawConfig(initialRawConfig)}
                disabled={!hasRawChanged}
                className="gap-1.5 text-xs"
              >
                <RotateCcw className="h-3.5 w-3.5" />
                <span>{t("common.reset", "Reset")}</span>
              </Button>
            </div>
          </CardHeader>

          <CardContent className="space-y-3">
            <textarea
              data-testid="textarea-sshd-config"
              rows={15}
              value={rawConfig}
              onChange={(e) => setRawConfig(e.target.value)}
              className="w-full rounded-md border bg-muted/20 p-3 font-mono text-xs leading-relaxed text-foreground focus:outline-none focus:ring-1 focus:ring-primary"
            />
          </CardContent>

          <CardFooter className="flex items-center justify-between border-t px-6 py-3">
            <span className="text-[11px] text-muted-foreground">
              {hasRawChanged
                ? t("common.unsavedChanges", "You have unsaved changes.")
                : t("system.ssh.configSynced", "Configuration is synchronized with node.")}
            </span>

            <Button
              size="sm"
              disabled={!hasRawChanged || !canManage}
              data-testid="btn-save-ssh-raw"
              onClick={() => setPendingAction({ type: "raw", raw: rawConfig })}
              className="gap-1.5 text-xs"
            >
              <Save className="h-3.5 w-3.5" />
              <span>{t("system.ssh.saveSshdConfig", "Validate & Save sshd_config")}</span>
            </Button>
          </CardFooter>
        </Card>
      )}

      {/* Sudo Mode Confirmation Modal */}
      <SudoConfirmModal
        open={Boolean(pendingAction)}
        onOpenChange={(open) => !open && setPendingAction(null)}
        title={t("system.ssh.sudoConfirmTitle", "Verify Administrative Sudo Authentication")}
        description={t(
          "system.ssh.sudoConfirmDesc",
          "SSH modifications alter core host authentication boundaries. Please provide administrator password to verify and apply changes.",
        )}
        confirmLabel={t("common.apply", "Apply Changes")}
        onConfirm={handleSudoConfirm}
      />
    </div>
  );
}
