import React, { useState } from "react";
import {
  Button,
  Input,
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Badge,
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
  ConfirmDialog,
  Table,
  TableHeader,
  TableBody,
  TableRow,
  TableHead,
  TableCell,
  useToast,
} from "@builtpanel/ui";
import { ShieldAlert, Trash2, PlusCircle, CheckCircle2, Globe, Calendar, HardDrive, DollarSign, Hash } from "lucide-react";
import { useFormatters } from "@builtpanel/i18n";

export function UiDemoPage() {
  const { toast } = useToast();
  const formatters = useFormatters();
  const sampleDate = new Date("2026-05-15T10:30:00Z");
  const samplePastDate = new Date(Date.now() - 1000 * 60 * 45); // 45 mins ago

  const [standardDialogOpen, setStandardDialogOpen] = useState(false);
  const [confirmDialogOpen, setConfirmDialogOpen] = useState(false);
  const [demoInput, setDemoInput] = useState("");
  const [inputError, setInputError] = useState("");

  const mockDomains = [
    { id: "1", name: "example.com", status: "active", ssl: true, php: "8.3" },
    { id: "2", name: "api.builtpanel.io", status: "active", ssl: true, php: "8.2" },
    { id: "3", name: "staging.test", status: "pending", ssl: false, php: "8.1" },
  ];

  return (
    <div className="space-y-8 max-w-6xl mx-auto">
      <div>
        <h1 className="text-3xl font-bold tracking-tight">UI Foundation Demo (T00.12)</h1>
        <p className="text-muted-foreground">
          Showcase of core design system components across light and dark themes.
        </p>
      </div>

      {/* 1. Buttons */}
      <Card>
        <CardHeader>
          <CardTitle>1. Buttons</CardTitle>
          <CardDescription>Primary, secondary, outline, destructive, and sizes.</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-3 items-center">
          <Button variant="default">Default Primary</Button>
          <Button variant="secondary">Secondary</Button>
          <Button variant="outline">Outline</Button>
          <Button variant="destructive" className="gap-1.5">
            <Trash2 className="h-4 w-4" />
            Destructive
          </Button>
          <Button variant="ghost">Ghost</Button>
          <Button size="sm">Small</Button>
          <Button size="lg">Large</Button>
          <Button disabled>Disabled</Button>
        </CardContent>
      </Card>

      {/* 2. Inputs */}
      <Card>
        <CardHeader>
          <CardTitle>2. Form Inputs</CardTitle>
          <CardDescription>Labels, validation error states, and helper descriptions.</CardDescription>
        </CardHeader>
        <CardContent className="grid gap-4 sm:grid-cols-2">
          <Input
            label="Domain Name"
            placeholder="e.g. mysite.com"
            helperText="Enter the primary fully-qualified domain name."
            value={demoInput}
            onChange={(e) => setDemoInput(e.target.value)}
          />
          <Input
            label="Server Hostname (Error example)"
            value="invalid hostname!"
            error={inputError || "Hostname contains invalid characters or symbols"}
            onChange={(e) => setInputError(e.target.value ? "" : "Invalid input")}
          />
        </CardContent>
      </Card>

      {/* 3. Toasts */}
      <Card>
        <CardHeader>
          <CardTitle>3. Global Toasts</CardTitle>
          <CardDescription>Trigger accessible toast alerts with auto-dismiss.</CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-3">
          <Button
            variant="outline"
            className="text-emerald-600 border-emerald-600 hover:bg-emerald-50 dark:hover:bg-emerald-950/20"
            onClick={() =>
              toast({
                title: "Operation Successful",
                description: "SSL Certificate issued and applied for example.com",
                variant: "success",
              })
            }
          >
            Success Toast
          </Button>
          <Button
            variant="outline"
            className="text-destructive border-destructive hover:bg-destructive/10"
            onClick={() =>
              toast({
                title: "Validation Error",
                description: "Unable to allocate port 80: already in use by node-agent.",
                variant: "destructive",
              })
            }
          >
            Destructive Toast
          </Button>
          <Button
            variant="outline"
            onClick={() =>
              toast({
                title: "System Notification",
                description: "Node synchronization background job queued.",
                variant: "info",
              })
            }
          >
            Info Toast
          </Button>
        </CardContent>
      </Card>

      {/* 4. Dialogs */}
      <Card>
        <CardHeader>
          <CardTitle>4. Modal Dialogs & Confirm Dialog</CardTitle>
          <CardDescription>
            Standard accessible modal dialogs and typed-confirmation safety dialogs for destructive actions.
          </CardDescription>
        </CardHeader>
        <CardContent className="flex flex-wrap gap-4">
          <Button variant="outline" onClick={() => setStandardDialogOpen(true)}>
            Open Standard Dialog
          </Button>

          <Button
            variant="destructive"
            className="gap-2"
            onClick={() => setConfirmDialogOpen(true)}
          >
            <ShieldAlert className="h-4 w-4" />
            Delete Tenant (Typed Confirm)
          </Button>

          {/* Standard Dialog */}
          <Dialog open={standardDialogOpen} onOpenChange={setStandardDialogOpen}>
            <DialogContent onClose={() => setStandardDialogOpen(false)}>
              <DialogHeader>
                <DialogTitle>Add New Domain</DialogTitle>
                <DialogDescription>
                  Configure a new domain mapping and document root in the container.
                </DialogDescription>
              </DialogHeader>
              <div className="py-4">
                <Input label="Domain FQDN" placeholder="app.example.com" />
              </div>
              <DialogFooter>
                <Button variant="outline" onClick={() => setStandardDialogOpen(false)}>
                  Cancel
                </Button>
                <Button
                  onClick={() => {
                    setStandardDialogOpen(false);
                    toast({ title: "Domain Created", variant: "success" });
                  }}
                >
                  Create Domain
                </Button>
              </DialogFooter>
            </DialogContent>
          </Dialog>

          {/* Confirm Dialog */}
          <ConfirmDialog
            open={confirmDialogOpen}
            onOpenChange={setConfirmDialogOpen}
            title="Delete Tenant 'tenant-acme-corp'"
            description="This will permanently delete the tenant container, web root, and databases. This action is irreversible."
            confirmText="delete-acme-corp"
            confirmPrompt="Please type the confirmation key to proceed:"
            onConfirm={() => {
              toast({
                title: "Tenant Deleted",
                description: "Tenant 'tenant-acme-corp' and its resources were purged.",
                variant: "destructive",
              });
            }}
          />
        </CardContent>
      </Card>

      {/* 5. Responsive Data Table */}
      <Card>
        <CardHeader className="flex flex-row items-center justify-between">
          <div>
            <CardTitle>5. Data Table</CardTitle>
            <CardDescription>Accessible tabular representation with status badges.</CardDescription>
          </div>
          <Button size="sm" className="gap-1.5">
            <PlusCircle className="h-4 w-4" />
            Add Row
          </Button>
        </CardHeader>
        <CardContent>
          <Table>
            <TableHeader>
              <TableRow>
                <TableHead>Domain</TableHead>
                <TableHead>Status</TableHead>
                <TableHead>SSL</TableHead>
                <TableHead>PHP Version</TableHead>
                <TableHead className="text-right">Actions</TableHead>
              </TableRow>
            </TableHeader>
            <TableBody>
              {mockDomains.map((dom) => (
                <TableRow key={dom.id}>
                  <TableCell className="font-medium flex items-center gap-2">
                    <Globe className="h-4 w-4 text-primary" />
                    {dom.name}
                  </TableCell>
                  <TableCell>
                    <Badge variant={dom.status === "active" ? "success" : "secondary"}>
                      {dom.status}
                    </Badge>
                  </TableCell>
                  <TableCell>
                    {dom.ssl ? (
                      <span className="inline-flex items-center gap-1 text-xs text-emerald-600 font-medium">
                        <CheckCircle2 className="h-3.5 w-3.5" />
                        Enabled
                      </span>
                    ) : (
                      <span className="text-xs text-muted-foreground">None</span>
                    )}
                  </TableCell>
                  <TableCell className="font-mono text-xs">{dom.php}</TableCell>
                  <TableCell className="text-right">
                    <Button variant="ghost" size="sm">
                      Manage
                    </Button>
                  </TableCell>
                </TableRow>
              ))}
            </TableBody>
          </Table>
        </CardContent>
      </Card>

      {/* 7. Localized Formats (i18n - T00.15) */}
      <Card>
        <CardHeader>
          <CardTitle>7. Localized Formats (i18n - T00.15)</CardTitle>
          <CardDescription>
            Bilingual number formatting, currency, storage bytes, dates, and relative time ({formatters.locale}).
          </CardDescription>
        </CardHeader>
        <CardContent>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-4 gap-4">
            <div className="p-4 rounded-lg border bg-muted/20 space-y-1">
              <span className="text-xs text-muted-foreground flex items-center gap-1.5 font-medium">
                <Hash className="h-3.5 w-3.5 text-primary" />
                Number (সংখ্যা)
              </span>
              <p className="text-xl font-bold font-mono">
                {formatters.formatNumber(1250450.75)}
              </p>
            </div>

            <div className="p-4 rounded-lg border bg-muted/20 space-y-1">
              <span className="text-xs text-muted-foreground flex items-center gap-1.5 font-medium">
                <DollarSign className="h-3.5 w-3.5 text-emerald-600" />
                Currency (মুদ্রা)
              </span>
              <p className="text-xl font-bold font-mono">
                {formatters.formatCurrency(2500, "BDT")}
              </p>
            </div>

            <div className="p-4 rounded-lg border bg-muted/20 space-y-1">
              <span className="text-xs text-muted-foreground flex items-center gap-1.5 font-medium">
                <HardDrive className="h-3.5 w-3.5 text-blue-600" />
                Disk / Bandwidth (তথ্য ধারণক্ষমতা)
              </span>
              <p className="text-xl font-bold font-mono">
                {formatters.formatBytes(1024 * 1024 * 1024 * 42.5)}
              </p>
            </div>

            <div className="p-4 rounded-lg border bg-muted/20 space-y-1">
              <span className="text-xs text-muted-foreground flex items-center gap-1.5 font-medium">
                <Calendar className="h-3.5 w-3.5 text-purple-600" />
                Date & Time (তারিখ ও সময়)
              </span>
              <p className="text-sm font-semibold">
                {formatters.formatDate(sampleDate)}
              </p>
              <p className="text-xs text-muted-foreground">
                {formatters.formatRelativeTime(samplePastDate)}
              </p>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
