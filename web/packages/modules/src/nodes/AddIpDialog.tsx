import React, { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogFooter,
  DialogTitle,
  DialogDescription,
  Button,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Globe, Plus, Network } from "lucide-react";
import { AddNodeIpPayload, IpAddressType, NodeItem } from "./types";

export interface AddIpDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  nodes: NodeItem[];
  defaultNodeId?: string;
  onSave: (payload: AddNodeIpPayload) => Promise<void>;
  isSaving?: boolean;
}

export function AddIpDialog({
  open,
  onOpenChange,
  nodes,
  defaultNodeId,
  onSave,
  isSaving = false,
}: AddIpDialogProps) {
  const { t } = useTranslation();

  const [nodeId, setNodeId] = useState(defaultNodeId || (nodes[0]?.id ?? ""));
  const [ipAddress, setIpAddress] = useState("");
  const [subnetMask, setSubnetMask] = useState("255.255.255.0");
  const [interfaceName, setInterfaceName] = useState("eth0");
  const [type, setType] = useState<IpAddressType>("dedicated");
  const [error, setError] = useState<string | null>(null);

  React.useEffect(() => {
    if (defaultNodeId) {
      setNodeId(defaultNodeId);
    } else if (nodes.length > 0 && !nodeId) {
      setNodeId(nodes[0].id);
    }
  }, [defaultNodeId, nodes]);

  const handleReset = () => {
    setIpAddress("");
    setSubnetMask("255.255.255.0");
    setInterfaceName("eth0");
    setType("dedicated");
    setError(null);
  };

  const handleClose = () => {
    handleReset();
    onOpenChange(false);
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!nodeId) {
      setError(t("nodes.ipPools.dialog.nodeRequired"));
      return;
    }
    if (!ipAddress.trim()) {
      setError(t("nodes.ipPools.dialog.ipRequired"));
      return;
    }

    try {
      await onSave({
        node_id: nodeId,
        ip_address: ipAddress.trim(),
        subnet_mask: subnetMask.trim(),
        interface_name: interfaceName.trim() || "eth0",
        type,
      });
      handleClose();
    } catch (err: any) {
      setError(err?.message || "Failed to add IP address");
    }
  };

  return (
    <Dialog open={open} onOpenChange={handleClose}>
      <DialogContent onClose={handleClose} className="max-w-md p-6">
        <DialogHeader>
          <div className="flex items-center gap-2 text-primary mb-1">
            <Globe className="h-5 w-5" />
            <DialogTitle>{t("nodes.ipPools.dialog.addTitle")}</DialogTitle>
          </div>
          <DialogDescription>{t("nodes.ipPools.dialog.addDesc")}</DialogDescription>
        </DialogHeader>

        <form onSubmit={handleSubmit} className="space-y-4 my-3">
          {error && (
            <div className="p-2.5 rounded-md bg-destructive/10 text-destructive text-xs">
              {error}
            </div>
          )}

          {/* Node Selector */}
          <div className="space-y-1">
            <label htmlFor="ip-node" className="text-xs font-medium text-foreground">
              {t("nodes.ipPools.dialog.targetNode")}
            </label>
            <select
              id="ip-node"
              data-testid="select-ip-node"
              value={nodeId}
              onChange={(e) => setNodeId(e.target.value)}
              disabled={isSaving}
              className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs shadow-sm text-foreground"
            >
              {nodes.map((n) => (
                <option key={n.id} value={n.id}>
                  {n.name} ({n.hostname} - {n.ip_address})
                </option>
              ))}
            </select>
          </div>

          {/* IP Address */}
          <div className="space-y-1">
            <label htmlFor="ip-address" className="text-xs font-medium text-foreground flex items-center gap-1">
              <span>{t("nodes.ipPools.dialog.ipLabel")}</span>
              <span className="text-destructive font-bold">*</span>
            </label>
            <Input
              id="ip-address"
              data-testid="input-ip-address"
              value={ipAddress}
              onChange={(e) => {
                setIpAddress(e.target.value);
                if (error) setError(null);
              }}
              placeholder="e.g. 198.51.100.40"
              disabled={isSaving}
              autoFocus
            />
          </div>

          <div className="grid grid-cols-2 gap-3">
            <div className="space-y-1">
              <label htmlFor="ip-subnet" className="text-xs font-medium text-foreground">
                {t("nodes.ipPools.dialog.subnetLabel")}
              </label>
              <Input
                id="ip-subnet"
                value={subnetMask}
                onChange={(e) => setSubnetMask(e.target.value)}
                placeholder="255.255.255.0"
                disabled={isSaving}
              />
            </div>

            <div className="space-y-1">
              <label htmlFor="ip-interface" className="text-xs font-medium text-foreground">
                {t("nodes.ipPools.dialog.interfaceLabel")}
              </label>
              <Input
                id="ip-interface"
                value={interfaceName}
                onChange={(e) => setInterfaceName(e.target.value)}
                placeholder="eth0"
                disabled={isSaving}
              />
            </div>
          </div>

          {/* IP Type Selector: Shared vs Dedicated */}
          <div className="space-y-1">
            <label htmlFor="ip-type" className="text-xs font-medium text-foreground">
              {t("nodes.ipPools.dialog.typeLabel")}
            </label>
            <select
              id="ip-type"
              data-testid="select-ip-type"
              value={type}
              onChange={(e) => setType(e.target.value as IpAddressType)}
              disabled={isSaving}
              className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-xs shadow-sm text-foreground"
            >
              <option value="dedicated">{t("nodes.ipPools.type.dedicated")}</option>
              <option value="shared">{t("nodes.ipPools.type.shared")}</option>
            </select>
            <p className="text-[11px] text-muted-foreground mt-1">
              {type === "dedicated"
                ? t("nodes.ipPools.dialog.dedicatedNote")
                : t("nodes.ipPools.dialog.sharedNote")}
            </p>
          </div>

          <DialogFooter className="pt-3 border-t gap-2">
            <Button
              type="button"
              variant="outline"
              disabled={isSaving}
              onClick={handleClose}
            >
              {t("common.cancel")}
            </Button>
            <Button
              type="submit"
              disabled={isSaving}
              data-testid="btn-save-ip"
              className="gap-2"
            >
              <Plus className="h-4 w-4" />
              {isSaving ? t("common.processing") : t("nodes.ipPools.dialog.saveBtn")}
            </Button>
          </DialogFooter>
        </form>
      </DialogContent>
    </Dialog>
  );
}
