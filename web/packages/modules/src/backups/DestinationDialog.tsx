import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogFooter,
  Button,
  Input,
  useToast,
} from "@builtpanel/ui";
import { createDestination } from "./adminApi";
import { HardDrive, Cloud, Server, CheckCircle2, AlertTriangle, RefreshCw } from "lucide-react";
import { useTranslation } from "@builtpanel/i18n";
import { BackupDestinationType } from "./types";

interface DestinationDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  onSaved: () => void;
}

export function DestinationDialog({
  open,
  onOpenChange,
  onSaved,
}: DestinationDialogProps) {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [type, setType] = useState<BackupDestinationType>("s3");
  const [name, setName] = useState("");
  const [localPath, setLocalPath] = useState("/var/backups/builtpanel");

  // S3 fields
  const [s3Bucket, setS3Bucket] = useState("");
  const [s3Region, setS3Region] = useState("us-east-1");
  const [s3Endpoint, setS3Endpoint] = useState("");
  const [s3AccessKey, setS3AccessKey] = useState("");
  const [s3SecretKey, setS3SecretKey] = useState("");

  // SFTP fields
  const [sftpHost, setSftpHost] = useState("");
  const [sftpPort, setSftpPort] = useState(22);
  const [sftpUser, setSftpUser] = useState("");
  const [sftpPath, setSftpPath] = useState("/backups");
  const [sftpPassword, setSftpPassword] = useState("");

  const [isSubmitting, setIsSubmitting] = useState(false);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!name.trim()) return;

    try {
      setIsSubmitting(true);
      await createDestination({
        type,
        name: name.trim(),
        local: { path: localPath },
        s3: { endpoint: s3Endpoint, bucket: s3Bucket, region: s3Region, accessKey: s3AccessKey, secretKey: s3SecretKey },
        sftp: { host: sftpHost, port: Number(sftpPort), user: sftpUser, path: sftpPath, password: sftpPassword },
      });

      toast({
        title: t("common.success"),
        description: t("adminBackups.destinationSaved"),
      });

      setName("");
      onOpenChange(false);
      onSaved();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsSubmitting(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent className="sm:max-w-[500px]">
        <DialogHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-lg bg-primary/10 text-primary">
              <Cloud className="w-5 h-5" />
            </div>
            <DialogTitle data-testid="destination-dialog-title">
              {t("adminBackups.addDestination")}
            </DialogTitle>
          </div>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 py-2">
          {/* Destination Type */}
          <div className="space-y-2">
            <label className="text-sm font-medium">
              {t("adminBackups.destinationType")}
            </label>
            <div className="grid grid-cols-3 gap-2">
              <button
                type="button"
                onClick={() => setType("local")}
                className={`p-2.5 rounded-lg border text-left flex flex-col items-center gap-1 transition-colors ${
                  type === "local" ? "border-primary bg-primary/5 font-semibold text-primary" : "border-border hover:bg-muted/50"
                }`}
                data-testid="dest-type-local"
              >
                <HardDrive className="w-4 h-4" />
                <span className="text-xs">Local</span>
              </button>

              <button
                type="button"
                onClick={() => setType("s3")}
                className={`p-2.5 rounded-lg border text-left flex flex-col items-center gap-1 transition-colors ${
                  type === "s3" ? "border-primary bg-primary/5 font-semibold text-primary" : "border-border hover:bg-muted/50"
                }`}
                data-testid="dest-type-s3"
              >
                <Cloud className="w-4 h-4" />
                <span className="text-xs">S3 Compatible</span>
              </button>

              <button
                type="button"
                onClick={() => setType("sftp")}
                className={`p-2.5 rounded-lg border text-left flex flex-col items-center gap-1 transition-colors ${
                  type === "sftp" ? "border-primary bg-primary/5 font-semibold text-primary" : "border-border hover:bg-muted/50"
                }`}
                data-testid="dest-type-sftp"
              >
                <Server className="w-4 h-4" />
                <span className="text-xs">SFTP Server</span>
              </button>
            </div>
          </div>

          {/* Name */}
          <div className="space-y-1.5">
            <label className="text-sm font-medium">
              {t("adminBackups.destinationName")}
            </label>
            <Input
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Production Wasabi S3"
              data-testid="input-dest-name"
              autoFocus
            />
          </div>

          {/* Type-specific inputs */}
          {type === "local" && (
            <div className="space-y-1.5">
              <label className="text-sm font-medium">{t("adminBackups.storagePath")}</label>
              <Input
                value={localPath}
                onChange={(e) => setLocalPath(e.target.value)}
                placeholder="/var/backups/builtpanel"
                className="font-mono text-xs"
                data-testid="input-local-path"
              />
            </div>
          )}

          {type === "s3" && (
            <div className="space-y-3">
              <div className="grid grid-cols-2 gap-2">
                <div className="space-y-1">
                  <label className="text-xs font-medium">Bucket</label>
                  <Input
                    value={s3Bucket}
                    onChange={(e) => setS3Bucket(e.target.value)}
                    placeholder="my-backup-bucket"
                    className="font-mono text-xs"
                    data-testid="input-s3-bucket"
                  />
                </div>
                <div className="space-y-1">
                  <label className="text-xs font-medium">Region</label>
                  <Input
                    value={s3Region}
                    onChange={(e) => setS3Region(e.target.value)}
                    placeholder="us-east-1"
                    className="font-mono text-xs"
                    data-testid="input-s3-region"
                  />
                </div>
              </div>

              <div className="space-y-1">
                <label className="text-xs font-medium">Endpoint (Optional for MinIO/Wasabi/B2)</label>
                <Input
                  value={s3Endpoint}
                  onChange={(e) => setS3Endpoint(e.target.value)}
                  placeholder="https://s3.wasabisys.com"
                  className="font-mono text-xs"
                  data-testid="input-s3-endpoint"
                />
              </div>

              <div className="grid grid-cols-2 gap-2">
                <div className="space-y-1">
                  <label className="text-xs font-medium">Access Key</label>
                  <Input
                    value={s3AccessKey}
                    onChange={(e) => setS3AccessKey(e.target.value)}
                    placeholder="AKIAIOSFODNN7EXAMPLE"
                    className="font-mono text-xs"
                    data-testid="input-s3-access-key"
                  />
                </div>
                <div className="space-y-1">
                  <label className="text-xs font-medium">Secret Key</label>
                  <Input
                    type="password"
                    value={s3SecretKey}
                    onChange={(e) => setS3SecretKey(e.target.value)}
                    placeholder="••••••••••••••••••••••••"
                    className="font-mono text-xs"
                    data-testid="input-s3-secret-key"
                  />
                </div>
              </div>
            </div>
          )}

          {type === "sftp" && (
            <div className="space-y-3">
              <div className="grid grid-cols-3 gap-2">
                <div className="col-span-2 space-y-1">
                  <label className="text-xs font-medium">Host</label>
                  <Input
                    value={sftpHost}
                    onChange={(e) => setSftpHost(e.target.value)}
                    placeholder="backup.example.com"
                    className="font-mono text-xs"
                    data-testid="input-sftp-host"
                  />
                </div>
                <div className="space-y-1">
                  <label className="text-xs font-medium">Port</label>
                  <Input
                    type="number"
                    value={sftpPort}
                    onChange={(e) => setSftpPort(Number(e.target.value))}
                    className="font-mono text-xs"
                    data-testid="input-sftp-port"
                  />
                </div>
              </div>

              <div className="grid grid-cols-2 gap-2">
                <div className="space-y-1">
                  <label className="text-xs font-medium">Username</label>
                  <Input
                    value={sftpUser}
                    onChange={(e) => setSftpUser(e.target.value)}
                    placeholder="backupuser"
                    className="font-mono text-xs"
                    data-testid="input-sftp-user"
                  />
                </div>
                <div className="space-y-1">
                  <label className="text-xs font-medium">Remote Path</label>
                  <Input
                    value={sftpPath}
                    onChange={(e) => setSftpPath(e.target.value)}
                    placeholder="/backups"
                    className="font-mono text-xs"
                    data-testid="input-sftp-path"
                  />
                </div>
              </div>
              <div className="space-y-1">
                <label className="text-xs font-medium">Password</label>
                <Input
                  type="password"
                  value={sftpPassword}
                  onChange={(e) => setSftpPassword(e.target.value)}
                  className="font-mono text-xs"
                  data-testid="input-sftp-password"
                />
              </div>
            </div>
          )}

          <DialogFooter className="pt-2">
            <div className="flex gap-2">
              <Button
                type="button"
                variant="outline"
                size="sm"
                onClick={() => onOpenChange(false)}
                disabled={isSubmitting}
              >
                {t("common.cancel")}
              </Button>
              <Button
                type="submit"
                size="sm"
                disabled={!name.trim() || isSubmitting}
                data-testid="btn-submit-destination"
              >
                {isSubmitting ? t("common.submitting") : t("common.save")}
              </Button>
            </div>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
