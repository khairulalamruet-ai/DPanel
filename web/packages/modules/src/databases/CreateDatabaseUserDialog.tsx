import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  Button,
  useToast,
} from "@builtpanel/ui";
import { createDbUser } from "./api";
import { UserCheck, Key, RefreshCw, Eye, EyeOff, ShieldAlert } from "lucide-react";
import { DatabaseItem, DatabaseUserItem } from "./types";
import { useTranslation } from "@builtpanel/i18n";

interface CreateDatabaseUserDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  prefix: string;
  databases: DatabaseItem[];
  onUserCreated: (newUser: DatabaseUserItem) => void;
  isLimitReached: boolean;
}

export function CreateDatabaseUserDialog({
  open,
  onOpenChange,
  prefix,
  databases,
  onUserCreated,
  isLimitReached,
}: CreateDatabaseUserDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [username, setUsername] = useState("");
  const [password, setPassword] = useState("");
  const [showPassword, setShowPassword] = useState(false);
  const [selectedDatabases, setSelectedDatabases] = useState<string[]>([]);
  const [isSubmitting, setIsSubmitting] = useState(false);
  const [validationError, setValidationError] = useState<string | null>(null);

  const cleanUsername = username.trim().toLowerCase().replace(/[^a-z0-9_]/g, "");
  const fullUsername = `${prefix}${cleanUsername}`;

  const generateRandomPassword = () => {
    const chars = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789!@#$%^&*()_+";
    let pass = "";
    for (let i = 0; i < 18; i++) {
      pass += chars.charAt(Math.floor(Math.random() * chars.length));
    }
    setPassword(pass);
    setShowPassword(true);
  };

  const handleToggleDb = (dbName: string) => {
    setSelectedDatabases((prev) =>
      prev.includes(dbName) ? prev.filter((d) => d !== dbName) : [...prev, dbName]
    );
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!cleanUsername) {
      setValidationError(t("databases.validation.usernameRequired", "Username is required"));
      return;
    }
    if (cleanUsername.length > 16) {
      setValidationError(t("databases.validation.usernameTooLong", "Username must be 16 characters or fewer"));
      return;
    }
    if (!password || password.length < 10) {
      setValidationError(t("databases.validation.passwordLength", "Password must be at least 10 characters"));
      return;
    }

    try {
      setIsSubmitting(true);
      setValidationError(null);

      const res = await createDbUser({
        name: fullUsername,
        prefix,
        password,
        grants: selectedDatabases.flatMap((dbName) => {
          const db = databases.find((d) => d.name === dbName);
          return db ? [{ dbId: db.id, access: "all" as const }] : [];
        }),
      });

      toast({
        title: t("common.success"),
        description: t("databases.toast.userCreated", "Database user {{username}} created successfully", {
          username: res.username,
        }),
        variant: "default",
      });

      onUserCreated(res);
      setUsername("");
      setPassword("");
      setSelectedDatabases([]);
      onOpenChange(false);
    } catch (err: any) {
      setValidationError(err?.message || t("databases.errors.createUserFailed", "Failed to create database user"));
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="max-w-md" onClose={() => onOpenChange(false)}>
        <DialogHeader>
          <div className="flex items-center gap-3">
            <div className="rounded-full bg-primary/10 p-2.5 text-primary">
              <UserCheck className="h-5 w-5" />
            </div>
            <div>
              <DialogTitle>{t("databases.createUserDialog.title", "Create Database User")}</DialogTitle>
              <DialogDescription className="mt-0.5 text-xs">
                {t("databases.createUserDialog.desc", "Create SQL credentials to connect applications to databases.")}
              </DialogDescription>
            </div>
          </div>
        </DialogHeader>

        {isLimitReached ? (
          <div className="mt-4 rounded-lg border border-destructive/20 bg-destructive/10 p-4 text-xs text-destructive">
            {t(
              "databases.createUserDialog.limitReached",
              "You have reached the maximum number of database users allocated for your hosting plan."
            )}
          </div>
        ) : (
          <form onSubmit={handleSubmit} className="mt-4 space-y-4">
            {validationError && (
              <div className="rounded-md border border-destructive/20 bg-destructive/10 p-3 text-xs text-destructive">
                {validationError}
              </div>
            )}

            {/* Username */}
            <div className="space-y-1.5">
              <label className="text-xs font-medium text-foreground">
                {t("databases.fields.username", "Database Username")}
              </label>
              <div className="flex items-center rounded-md border border-input bg-background focus-within:ring-1 focus-within:ring-ring overflow-hidden">
                <span className="bg-muted px-3 py-2 text-xs font-mono text-muted-foreground select-none border-r border-input">
                  {prefix}
                </span>
                <input
                  type="text"
                  value={username}
                  onChange={(e) => {
                    setUsername(e.target.value);
                    setValidationError(null);
                  }}
                  placeholder="dbuser"
                  maxLength={16}
                  className="flex-1 bg-transparent px-3 py-2 text-xs font-mono text-foreground focus:outline-none"
                  data-testid="input-db-username"
                  autoFocus
                />
              </div>
              <p className="text-[11px] text-muted-foreground font-mono">
                {t("databases.fields.fullUsername", "Full username")}: <strong>{fullUsername}</strong>
              </p>
            </div>

            {/* Password with generator */}
            <div className="space-y-1.5">
              <div className="flex items-center justify-between">
                <label className="text-xs font-medium text-foreground">
                  {t("databases.fields.password", "Password")}
                </label>
                <button
                  type="button"
                  onClick={generateRandomPassword}
                  className="text-xs text-primary hover:underline flex items-center gap-1"
                  data-testid="btn-generate-db-password"
                >
                  <RefreshCw className="h-3 w-3" />
                  <span>{t("databases.actions.generatePassword", "Generate")}</span>
                </button>
              </div>
              <div className="relative">
                <input
                  type={showPassword ? "text" : "password"}
                  value={password}
                  onChange={(e) => setPassword(e.target.value)}
                  placeholder="••••••••••••"
                  className="w-full h-9 rounded-md border border-input bg-background px-3 pr-10 text-xs font-mono text-foreground focus:outline-none focus:ring-1 focus:ring-ring"
                  data-testid="input-db-password"
                />
                <button
                  type="button"
                  onClick={() => setShowPassword(!showPassword)}
                  className="absolute right-2.5 top-2.5 text-muted-foreground hover:text-foreground"
                >
                  {showPassword ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                </button>
              </div>
            </div>

            {/* Assign Databases */}
            {databases.length > 0 && (
              <div className="space-y-1.5">
                <label className="text-xs font-medium text-foreground">
                  {t("databases.fields.assignDatabases", "Assign to Databases")}
                </label>
                <div className="max-h-32 overflow-y-auto rounded-md border border-input bg-muted/20 p-2 space-y-1.5">
                  {databases.map((db) => (
                    <label
                      key={db.id}
                      className="flex items-center gap-2 text-xs font-mono cursor-pointer hover:bg-muted/40 p-1 rounded"
                    >
                      <input
                        type="checkbox"
                        checked={selectedDatabases.includes(db.name)}
                        onChange={() => handleToggleDb(db.name)}
                        className="rounded border-input text-primary focus:ring-primary"
                        data-testid={`checkbox-assign-db-${db.name}`}
                      />
                      <span>{db.name}</span>
                    </label>
                  ))}
                </div>
              </div>
            )}

            <DialogFooter className="mt-6 border-t border-border pt-3">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => onOpenChange(false)}
                disabled={isSubmitting}
                data-testid="btn-cancel-create-user"
              >
                {t("common.cancel")}
              </Button>
              <Button
                type="submit"
                size="sm"
                disabled={!cleanUsername || !password || isSubmitting}
                data-testid="btn-submit-create-user"
              >
                {isSubmitting ? t("common.creating") : t("databases.actions.createUser", "Create User")}
              </Button>
            </DialogFooter>
          </form>
        )}
      </DialogContent>
    </Dialog>
  );
}
