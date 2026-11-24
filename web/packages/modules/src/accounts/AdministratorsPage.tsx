import React, { useCallback, useEffect, useState } from "react";
import {
  Badge,
  Button,
  Card,
  CardContent,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableHeader,
  TableRow,
  useToast,
} from "@builtpanel/ui";
import { apiFetch } from "@builtpanel/api";
import { useTranslation } from "@builtpanel/i18n";
import { RefreshCw } from "lucide-react";
import { AccountsTabs } from "./AccountsTabs";

interface Administrator {
  id: string;
  username: string;
  email: string;
  status: string;
  roles: string[];
  mfa_enabled: boolean;
  mfa_required: boolean;
  last_login_at?: string | null;
  created_at: string;
}

interface AdministratorList {
  items: Administrator[];
  next_after?: string | null;
}

/** The administrators of the panel (the third tab of the accounts lists). */
export function AdministratorsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const [admins, setAdmins] = useState<Administrator[]>([]);
  const [loading, setLoading] = useState(true);

  const load = useCallback(async () => {
    setLoading(true);
    try {
      const all: Administrator[] = [];
      let after: string | null | undefined;
      // The list is paged (100 at most): follow it to the end.
      do {
        const query = new URLSearchParams({ limit: "100" });
        if (after) query.set("after", after);
        const page = await apiFetch<AdministratorList>(`/api/v1/admin/administrators?${query}`);
        all.push(...(page.items ?? []));
        after = page.next_after;
      } while (after);
      setAdmins(all);
    } catch (err) {
      toast({
        title: t("common.error", "Error"),
        description: err instanceof Error ? err.message : t("accounts.administrators.loadError", "The administrators could not be loaded"),
        variant: "destructive",
      });
    } finally {
      setLoading(false);
    }
  }, [t, toast]);

  useEffect(() => {
    void load();
  }, [load]);

  return (
    <div className="space-y-6" data-testid="administrators-page">
      <AccountsTabs active="administrators" />
      <div className="flex items-center justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight">{t("accounts.administrators.title", "Administrators")}</h1>
          <p className="text-sm text-muted-foreground">
            {t("accounts.administrators.desc", "The people who can sign in to the admin area of this panel.")}
          </p>
        </div>
        <Button variant="outline" size="sm" className="gap-2" onClick={() => void load()} disabled={loading}>
          <RefreshCw className={`h-4 w-4 ${loading ? "animate-spin" : ""}`} />
          {t("common.refresh", "Refresh")}
        </Button>
      </div>
      <Card>
        <CardContent className="p-0">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("accounts.administrators.username", "Username")}</TableHead>
                <TableHead>{t("accounts.administrators.email", "Email")}</TableHead>
                <TableHead>{t("accounts.administrators.status", "Status")}</TableHead>
                <TableHead>{t("accounts.administrators.roles", "Roles")}</TableHead>
                <TableHead>{t("accounts.administrators.mfa", "Two-factor")}</TableHead>
                <TableHead>{t("accounts.administrators.lastLogin", "Last sign-in")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {admins.length === 0 && (
                <TableRow>
                  <TableCell colSpan={6} className="py-8 text-center text-muted-foreground">
                    {loading ? t("common.loading", "Loading...") : t("accounts.administrators.none", "No administrators.")}
                  </TableCell>
                </TableRow>
              )}
              {admins.map((a) => (
                <TableRow key={a.id}>
                  <TableCell className="font-medium">{a.username}</TableCell>
                  <TableCell>{a.email}</TableCell>
                  <TableCell>
                    <Badge variant={a.status === "active" ? "default" : "destructive"}>{a.status}</Badge>
                  </TableCell>
                  <TableCell>{a.roles.join(", ")}</TableCell>
                  <TableCell>
                    {a.mfa_enabled
                      ? t("accounts.administrators.mfaOn", "On")
                      : a.mfa_required
                        ? t("accounts.administrators.mfaRequired", "Required, not set up")
                        : t("accounts.administrators.mfaOff", "Off")}
                  </TableCell>
                  <TableCell>{a.last_login_at ? new Date(a.last_login_at).toLocaleString() : "—"}</TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>
    </div>
  );
}
