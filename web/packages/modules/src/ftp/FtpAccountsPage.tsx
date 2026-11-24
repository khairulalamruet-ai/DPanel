import React, { useState, useEffect, useCallback } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  Button,
  Badge,
  Input,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import { listFtp, deleteFtp } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  UserPlus,
  RefreshCw,
  Search,
  Key,
  HardDrive,
  Trash2,
  Folder,
  User,
} from "lucide-react";
import { FtpAccount, FtpConnectionInfo } from "./types";
import { FtpConnectionInfoCard } from "./FtpConnectionInfoCard";
import { CreateFtpAccountDialog } from "./CreateFtpAccountDialog";
import { ChangeFtpPasswordDialog } from "./ChangeFtpPasswordDialog";
import { ChangeFtpQuotaDialog } from "./ChangeFtpQuotaDialog";

export function FtpAccountsPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [accounts, setAccounts] = useState<FtpAccount[]>([]);
  const [connectionInfo, setConnectionInfo] = useState<FtpConnectionInfo | null>(null);
  const [prefix, setPrefix] = useState("");
  const [searchQuery, setSearchQuery] = useState("");
  const [isLoading, setIsLoading] = useState(false);

  // Dialog states
  const [isCreateOpen, setIsCreateOpen] = useState(false);
  const [passwordTarget, setPasswordTarget] = useState<FtpAccount | null>(null);
  const [quotaTarget, setQuotaTarget] = useState<FtpAccount | null>(null);

  const fetchFtpData = useCallback(async () => {
    try {
      setIsLoading(true);
      const res = await listFtp();
      setAccounts(res.accounts);
      setConnectionInfo(res.info);
      setPrefix(res.prefix);
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
    fetchFtpData();
  }, [fetchFtpData]);

  const handleDelete = async (account: FtpAccount) => {
    if (!window.confirm(t("ftp.confirmDelete", { username: account.username }))) {
      return;
    }

    try {
      await deleteFtp(account.id);

      toast({
        title: t("common.success"),
        description: t("ftp.deletedSuccess", { username: account.username }),
      });
      fetchFtpData();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  const filteredAccounts = accounts.filter(
    (acc) =>
      acc.username.toLowerCase().includes(searchQuery.toLowerCase()) ||
      acc.home_directory.toLowerCase().includes(searchQuery.toLowerCase())
  );

  return (
    <div className="p-6 space-y-6" data-testid="ftp-accounts-page">
      {/* Header */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="ftp-page-title">
            {t("ftp.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("ftp.description")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            size="sm"
            className="gap-2"
            onClick={() => setIsCreateOpen(true)}
            data-testid="btn-open-create-ftp"
          >
            <UserPlus className="w-4 h-4" />
            <span>{t("ftp.newAccount")}</span>
          </Button>

          <Button
            variant="outline"
            size="sm"
            className="gap-2"
            onClick={fetchFtpData}
            disabled={isLoading}
            data-testid="btn-refresh-ftp"
          >
            <RefreshCw className={`w-4 h-4 ${isLoading ? "animate-spin" : ""}`} />
            <span>{t("common.refresh")}</span>
          </Button>
        </div>
      </div>

      {/* Connection Info Banner */}
      <FtpConnectionInfoCard info={connectionInfo} />

      {/* Accounts List Card */}
      <Card>
        <CardHeader className="py-4 px-6 border-b flex flex-col sm:flex-row sm:items-center sm:justify-between gap-3">
          <CardTitle className="text-base font-semibold flex items-center gap-2">
            <User className="w-5 h-5 text-primary" />
            <span data-testid="ftp-accounts-heading">{t("ftp.accountsList")}</span>
            <Badge variant="secondary" className="ml-1 text-xs">
              {accounts.length}
            </Badge>
          </CardTitle>

          {/* Search */}
          <div className="relative w-full sm:w-64">
            <Search className="w-4 h-4 absolute left-2.5 top-2.5 text-muted-foreground" />
            <Input
              placeholder={t("common.search")}
              value={searchQuery}
              onChange={(e) => setSearchQuery(e.target.value)}
              className="pl-8 h-9 text-xs"
              data-testid="input-search-ftp"
            />
          </div>
        </CardHeader>

        <div className="overflow-x-auto">
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>{t("ftp.username")}</TableHead>
                <TableHead>{t("ftp.homeDirectory")}</TableHead>
                <TableHead className="w-48">{t("ftp.quota")}</TableHead>
                <TableHead className="w-24 text-center">{t("common.status")}</TableHead>
                <TableHead className="w-40">{t("ftp.access")}</TableHead>
                <TableHead className="w-36 text-right">{t("common.actions")}</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {isLoading ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-10 text-muted-foreground">
                    {t("common.loading")}
                  </TableCell>
                </TableRow>
              ) : filteredAccounts.length === 0 ? (
                <TableRow>
                  <TableCell colSpan={6} className="text-center py-10 text-muted-foreground">
                    {t("ftp.noAccountsFound")}
                  </TableCell>
                </TableRow>
              ) : (
                filteredAccounts.map((acc) => {
                  const isUnlimited = acc.quota_bytes === -1;
                  const pct = isUnlimited || acc.quota_bytes === 0
                    ? 0
                    : Math.round((acc.used_bytes / acc.quota_bytes) * 100);

                  return (
                    <TableRow
                      key={acc.id}
                      className="hover:bg-muted/50 transition-colors"
                      data-testid={`ftp-account-row-${acc.username}`}
                    >
                      <TableCell>
                        <div className="flex items-center gap-2">
                          <User className="w-4 h-4 text-primary shrink-0" />
                          <span
                            className="font-mono text-sm font-semibold text-foreground truncate"
                            data-testid={`ftp-username-${acc.username}`}
                          >
                            {acc.username}
                          </span>
                        </div>
                      </TableCell>

                      <TableCell>
                        <div className="flex items-center gap-1.5 font-mono text-xs text-muted-foreground">
                          <Folder className="w-3.5 h-3.5 text-amber-500 shrink-0" />
                          <span className="truncate max-w-xs">{acc.home_directory}</span>
                        </div>
                      </TableCell>

                      <TableCell>
                        <div className="space-y-1">
                          <div className="flex justify-between text-xs font-mono">
                            <span>{acc.used_formatted}</span>
                            <span className="text-muted-foreground">/ {acc.quota_formatted}</span>
                          </div>
                          {!isUnlimited && (
                            <div className="w-full bg-muted rounded-full h-1.5 overflow-hidden">
                              <div
                                className={`h-1.5 rounded-full ${pct > 90 ? "bg-destructive" : "bg-primary"}`}
                                style={{ width: `${Math.min(pct, 100)}%` }}
                              />
                            </div>
                          )}
                        </div>
                      </TableCell>

                      <TableCell className="text-center">
                        <Badge
                          variant={acc.status === "active" ? "success" : acc.status === "failed" ? "destructive" : "secondary"}
                          className="text-xs"
                          data-testid={`ftp-status-${acc.username}`}
                        >
                          {acc.status === "active" ? t("common.active") : acc.status === "suspended" ? t("common.suspended") : acc.status}
                        </Badge>
                      </TableCell>

                      <TableCell className="text-xs text-muted-foreground">
                        {acc.access}
                      </TableCell>

                      <TableCell className="text-right">
                        <div className="flex items-center justify-end gap-1">
                          <Button
                            size="sm"
                            variant="outline"
                            className="h-7 w-7 p-0"
                            title={t("ftp.changePassword")}
                            onClick={() => setPasswordTarget(acc)}
                            data-testid={`btn-change-pw-${acc.username}`}
                          >
                            <Key className="w-3.5 h-3.5" />
                          </Button>

                          <Button
                            size="sm"
                            variant="outline"
                            className="h-7 w-7 p-0"
                            title={t("ftp.changeQuota")}
                            onClick={() => setQuotaTarget(acc)}
                            data-testid={`btn-change-quota-${acc.username}`}
                          >
                            <HardDrive className="w-3.5 h-3.5" />
                          </Button>

                          <Button
                            size="sm"
                            variant="destructive"
                            className="h-7 w-7 p-0"
                            title={t("common.delete")}
                            onClick={() => handleDelete(acc)}
                            data-testid={`btn-delete-ftp-${acc.username}`}
                          >
                            <Trash2 className="w-3.5 h-3.5" />
                          </Button>
                        </div>
                      </TableCell>
                    </TableRow>
                  );
                })
              )}
            </TableBody>
          </Table>
        </div>
      </Card>

      {/* Dialogs */}
      <CreateFtpAccountDialog
        open={isCreateOpen}
        onOpenChange={setIsCreateOpen}
        prefix={prefix}
        onCreated={fetchFtpData}
      />

      <ChangeFtpPasswordDialog
        open={!!passwordTarget}
        onOpenChange={(open) => !open && setPasswordTarget(null)}
        account={passwordTarget}
        onChanged={fetchFtpData}
      />

      <ChangeFtpQuotaDialog
        open={!!quotaTarget}
        onOpenChange={(open) => !open && setQuotaTarget(null)}
        account={quotaTarget}
        onChanged={fetchFtpData}
      />
    </div>
  );
}
