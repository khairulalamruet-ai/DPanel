import React, { useState, useEffect, useCallback, useRef } from "react";
import {
  Button,
  Input,
  Badge,
  Table,
  TableHeader,
  TableRow,
  TableHead,
  TableBody,
  TableCell,
  Card,
  CardHeader,
  CardTitle,
  CardContent,
  useToast,
} from "@builtpanel/ui";
import { listFolder, listTrash, deleteItem, uploadFile } from "./api";
import { useTranslation } from "@builtpanel/i18n";
import {
  Folder,
  FolderPlus,
  FileCode,
  FilePlus,
  FileText,
  Archive,
  Upload,
  Trash2,
  RefreshCw,
  Edit,
  ChevronRight,
  Home,
  Search,
  CheckSquare,
  Square,
  HardDrive,
  FileArchive,
  ArrowUp,
} from "lucide-react";
import { FileItem, FileListResponse, TrashListResponse } from "./types";
import { CreateItemDialog } from "./CreateItemDialog";
import { RenameDialog } from "./RenameDialog";
import { PermissionsDialog } from "./PermissionsDialog";
import { FileEditorModal } from "./FileEditorModal";
import { ArchiveDialog } from "./ArchiveDialog";
import { ExtractDialog } from "./ExtractDialog";
import { TrashViewModal } from "./TrashViewModal";

const EDITABLE_EXTENSIONS = [
  "php",
  "html",
  "htm",
  "css",
  "js",
  "jsx",
  "ts",
  "tsx",
  "json",
  "txt",
  "md",
  "env",
  "htaccess",
  "conf",
  "ini",
  "sql",
  "yml",
  "yaml",
  "xml",
  "sh",
];

const ARCHIVE_EXTENSIONS = ["zip", "tar", "gz", "tgz", "rar", "7z"];

export function FileManagerPage() {
  const { t } = useTranslation();
  const { toast } = useToast();

  const [currentPath, setCurrentPath] = useState<string>("/");
  const [data, setData] = useState<FileListResponse | null>(null);
  const [trashCount, setTrashCount] = useState<number>(0);
  const [searchQuery, setSearchQuery] = useState("");
  const [selectedIds, setSelectedIds] = useState<Set<string>>(new Set());
  const [isLoading, setIsLoading] = useState(false);
  const [isUploading, setIsUploading] = useState(false);

  // Dialog states
  const [createType, setCreateType] = useState<"file" | "directory" | null>(null);
  const [renameTarget, setRenameTarget] = useState<FileItem | null>(null);
  const [chmodTarget, setChmodTarget] = useState<FileItem | null>(null);
  const [editorTarget, setEditorTarget] = useState<FileItem | null>(null);
  const [archiveTargets, setArchiveTargets] = useState<FileItem[]>([]);
  const [extractTarget, setExtractTarget] = useState<FileItem | null>(null);
  const [trashOpen, setTrashOpen] = useState(false);

  const fileInputRef = useRef<HTMLInputElement>(null);

  const fetchFiles = useCallback(async (path: string) => {
    try {
      setIsLoading(true);
      const res = await listFolder(path);
      setData(res);
      setCurrentPath(res.current_path);
      setSelectedIds(new Set());
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

  const fetchTrashCount = useCallback(async () => {
    try {
      const res = await listTrash();
      setTrashCount(res.total_items ?? (res.items ? res.items.length : 0));
    } catch {
      // Ignore count fetch errors
    }
  }, []);

  useEffect(() => {
    fetchFiles(currentPath);
    fetchTrashCount();
  }, [fetchFiles, fetchTrashCount]);

  const handleNavigate = (path: string) => {
    fetchFiles(path);
  };

  const handleParentNavigate = () => {
    if (data?.parent_path) {
      handleNavigate(data.parent_path);
    }
  };

  const handleSelectAll = () => {
    if (!data?.items) return;
    if (selectedIds.size === data.items.length) {
      setSelectedIds(new Set());
    } else {
      setSelectedIds(new Set(data.items.map((i) => i.id)));
    }
  };

  const handleToggleSelect = (id: string) => {
    const next = new Set(selectedIds);
    if (next.has(id)) {
      next.delete(id);
    } else {
      next.add(id);
    }
    setSelectedIds(next);
  };

  const handleDeleteItem = async (item: FileItem) => {
    if (!window.confirm(t("files.confirmDelete", { name: item.name }))) {
      return;
    }

    try {
      await deleteItem(item.path);

      toast({
        title: t("common.success"),
        description: t("files.deletedSuccess"),
      });
      fetchFiles(currentPath);
      fetchTrashCount();
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    }
  };

  const handleFileUpload = async (files: FileList | null) => {
    if (!files || files.length === 0) return;

    try {
      setIsUploading(true);
      for (let i = 0; i < files.length; i++) {
        const file = files[i];
        await uploadFile(currentPath, file);
      }

      toast({
        title: t("common.success"),
        description: t("files.uploadSuccess"),
      });
      fetchFiles(currentPath);
    } catch (err: any) {
      toast({
        title: t("common.error"),
        description: err.message || t("common.unknownError"),
        variant: "destructive",
      });
    } finally {
      setIsUploading(false);
      if (fileInputRef.current) {
        fileInputRef.current.value = "";
      }
    }
  };

  const handleDrop = (e: React.DragEvent) => {
    e.preventDefault();
    e.stopPropagation();
    if (e.dataTransfer.files && e.dataTransfer.files.length > 0) {
      handleFileUpload(e.dataTransfer.files);
    }
  };

  const isEditable = (item: FileItem) => {
    if (item.type !== "file") return false;
    const ext = item.name.split(".").pop()?.toLowerCase();
    return !!ext && EDITABLE_EXTENSIONS.includes(ext);
  };

  const isArchive = (item: FileItem) => {
    if (item.type !== "file") return false;
    const ext = item.name.split(".").pop()?.toLowerCase();
    return !!ext && ARCHIVE_EXTENSIONS.includes(ext);
  };

  // Breadcrumbs calculation
  const pathParts = currentPath.split("/").filter(Boolean);
  const breadcrumbSegments = pathParts.map((part, index) => {
    const full = "/" + pathParts.slice(0, index + 1).join("/");
    return { name: part, path: full };
  });

  const filteredItems = (data?.items || []).filter((item) =>
    item.name.toLowerCase().includes(searchQuery.toLowerCase())
  );

  return (
    <div
      className="p-6 space-y-6"
      onDragOver={(e) => e.preventDefault()}
      onDrop={handleDrop}
      data-testid="file-manager-page"
    >
      {/* Page Title & Stats */}
      <div className="flex flex-col sm:flex-row sm:items-center sm:justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight" data-testid="file-manager-title">
            {t("files.title")}
          </h1>
          <p className="text-sm text-muted-foreground">
            {t("files.description")}
          </p>
        </div>

        <div className="flex items-center gap-2">
          <Button
            variant="outline"
            size="sm"
            className="gap-2"
            onClick={() => {
              window.location.href = "/disk-usage";
            }}
            data-testid="disk-usage-nav-button"
          >
            <HardDrive className="w-4 h-4 text-primary" />
            <span>{t("diskUsage.title")}</span>
          </Button>

          <Button
            variant="outline"
            size="sm"
            className="gap-2 relative"
            onClick={() => setTrashOpen(true)}
            data-testid="trash-view-button"
          >
            <Trash2 className="w-4 h-4 text-destructive" />
            <span>{t("files.trash")}</span>
            {trashCount > 0 && (
              <Badge variant="secondary" className="ml-1 text-xs px-1.5 py-0 h-4">
                {trashCount}
              </Badge>
            )}
          </Button>

          <Button
            variant="outline"
            size="sm"
            className="gap-2"
            onClick={() => {
              fetchFiles(currentPath);
              fetchTrashCount();
            }}
            disabled={isLoading}
            data-testid="refresh-files-button"
          >
            <RefreshCw className={`w-4 h-4 ${isLoading ? "animate-spin" : ""}`} />
            <span>{t("common.refresh")}</span>
          </Button>
        </div>
      </div>

      {/* Main Workspace Layout */}
      <div className="grid grid-cols-1 md:grid-cols-4 gap-6">
        {/* Quick Directory Shortcuts Sidebar */}
        <div className="md:col-span-1 space-y-4">
          <Card>
            <CardHeader className="py-3 px-4 border-b">
              <CardTitle className="text-sm font-semibold flex items-center gap-2">
                <HardDrive className="w-4 h-4 text-primary" />
                {t("files.directories")}
              </CardTitle>
            </CardHeader>
            <CardContent className="p-2 space-y-1">
              <button
                type="button"
                onClick={() => handleNavigate("/")}
                className={`w-full text-left px-3 py-2 rounded-md text-sm flex items-center gap-2 hover:bg-muted/80 transition-colors ${
                  currentPath === "/" ? "bg-muted font-semibold text-primary" : "text-muted-foreground"
                }`}
                data-testid="shortcut-home"
              >
                <Home className="w-4 h-4 shrink-0" />
                <span>Home (/)</span>
              </button>
              <button
                type="button"
                onClick={() => handleNavigate("/public_html")}
                className={`w-full text-left px-3 py-2 rounded-md text-sm flex items-center gap-2 hover:bg-muted/80 transition-colors ${
                  currentPath === "/public_html" ? "bg-muted font-semibold text-primary" : "text-muted-foreground"
                }`}
                data-testid="shortcut-public-html"
              >
                <Folder className="w-4 h-4 text-amber-500 shrink-0" />
                <span>public_html</span>
              </button>
              <button
                type="button"
                onClick={() => handleNavigate("/logs")}
                className={`w-full text-left px-3 py-2 rounded-md text-sm flex items-center gap-2 hover:bg-muted/80 transition-colors ${
                  currentPath === "/logs" ? "bg-muted font-semibold text-primary" : "text-muted-foreground"
                }`}
                data-testid="shortcut-logs"
              >
                <Folder className="w-4 h-4 text-blue-500 shrink-0" />
                <span>logs</span>
              </button>
              <button
                type="button"
                onClick={() => handleNavigate("/ssl")}
                className={`w-full text-left px-3 py-2 rounded-md text-sm flex items-center gap-2 hover:bg-muted/80 transition-colors ${
                  currentPath === "/ssl" ? "bg-muted font-semibold text-primary" : "text-muted-foreground"
                }`}
                data-testid="shortcut-ssl"
              >
                <Folder className="w-4 h-4 text-emerald-500 shrink-0" />
                <span>ssl</span>
              </button>
            </CardContent>
          </Card>

          {data && (
            <Card className="text-xs text-muted-foreground p-3 space-y-1.5 bg-muted/30">
              <div className="flex justify-between">
                <span>{t("files.totalItems")}:</span>
                <span className="font-medium text-foreground">{data.items.length}</span>
              </div>
              <div className="flex justify-between">
                <span>{t("files.directories")}:</span>
                <span className="font-medium text-foreground">{data.total_dirs}</span>
              </div>
              <div className="flex justify-between">
                <span>{t("files.files")}:</span>
                <span className="font-medium text-foreground">{data.total_files}</span>
              </div>
              <div className="flex justify-between border-t pt-1.5">
                <span>{t("files.totalSize")}:</span>
                <span className="font-medium text-foreground">{data.total_size_formatted}</span>
              </div>
            </Card>
          )}
        </div>

        {/* Files Explorer Area */}
        <div className="md:col-span-3 space-y-4">
          {/* Action Toolbar */}
          <div className="flex flex-wrap items-center justify-between gap-3 p-3 bg-card border rounded-lg">
            <div className="flex flex-wrap items-center gap-2">
              <Button
                size="sm"
                className="gap-1.5"
                onClick={() => setCreateType("file")}
                data-testid="new-file-button"
              >
                <FilePlus className="w-4 h-4" />
                <span>{t("files.newFile")}</span>
              </Button>

              <Button
                size="sm"
                variant="outline"
                className="gap-1.5"
                onClick={() => setCreateType("directory")}
                data-testid="new-folder-button"
              >
                <FolderPlus className="w-4 h-4" />
                <span>{t("files.newFolder")}</span>
              </Button>

              <Button
                size="sm"
                variant="outline"
                className="gap-1.5 cursor-pointer"
                disabled={isUploading}
                onClick={() => fileInputRef.current?.click()}
                data-testid="upload-button"
              >
                <Upload className="w-4 h-4" />
                <span>{isUploading ? t("common.submitting") : t("files.upload")}</span>
              </Button>
              <input
                ref={fileInputRef}
                type="file"
                multiple
                className="hidden"
                data-testid="hidden-file-input"
                onChange={(e) => handleFileUpload(e.target.files)}
              />

              {selectedIds.size > 0 && (
                <Button
                  size="sm"
                  variant="outline"
                  className="gap-1.5"
                  onClick={() => {
                    const selected = (data?.items || []).filter((i) => selectedIds.has(i.id));
                    setArchiveTargets(selected);
                  }}
                  data-testid="archive-selected-button"
                >
                  <Archive className="w-4 h-4" />
                  <span>{t("files.compress")} ({selectedIds.size})</span>
                </Button>
              )}
            </div>

            {/* Filter */}
            <div className="relative w-full sm:w-48">
              <Search className="w-4 h-4 absolute left-2.5 top-2.5 text-muted-foreground" />
              <Input
                placeholder={t("common.search")}
                value={searchQuery}
                onChange={(e) => setSearchQuery(e.target.value)}
                className="pl-8 h-9 text-xs"
                data-testid="search-files-input"
              />
            </div>
          </div>

          {/* Breadcrumbs Navigation */}
          <div className="flex items-center gap-1.5 px-3 py-2 bg-muted/40 border rounded-lg text-sm font-mono overflow-x-auto">
            <button
              onClick={() => handleNavigate("/")}
              className="hover:text-primary transition-colors flex items-center gap-1"
              title="Home"
            >
              <Home className="w-3.5 h-3.5" />
            </button>
            <ChevronRight className="w-3.5 h-3.5 text-muted-foreground shrink-0" />

            {breadcrumbSegments.map((segment, index) => {
              const isLast = index === breadcrumbSegments.length - 1;
              return (
                <React.Fragment key={segment.path}>
                  <button
                    onClick={() => handleNavigate(segment.path)}
                    className={`hover:underline truncate max-w-[150px] ${
                      isLast ? "font-bold text-foreground" : "text-muted-foreground"
                    }`}
                  >
                    {segment.name}
                  </button>
                  {!isLast && (
                    <ChevronRight className="w-3.5 h-3.5 text-muted-foreground shrink-0" />
                  )}
                </React.Fragment>
              );
            })}
          </div>

          {/* Files List Table */}
          <div className="border rounded-lg overflow-hidden bg-card">
            <Table>
              <TableHeader>
                <TableRow>
                  <TableHead className="w-10 text-center">
                    <button
                      type="button"
                      onClick={handleSelectAll}
                      className="p-1 hover:text-foreground text-muted-foreground"
                    >
                      {data?.items && selectedIds.size > 0 && selectedIds.size === data.items.length ? (
                        <CheckSquare className="w-4 h-4 text-primary" />
                      ) : (
                        <Square className="w-4 h-4" />
                      )}
                    </button>
                  </TableHead>
                  <TableHead>{t("files.name")}</TableHead>
                  <TableHead className="w-28">{t("files.size")}</TableHead>
                  <TableHead className="w-24">{t("files.permissions")}</TableHead>
                  <TableHead className="w-36">{t("files.modified")}</TableHead>
                  <TableHead className="w-32 text-right">{t("common.actions")}</TableHead>
                </TableRow>
              </TableHeader>
              <TableBody>
                {/* Parent directory navigation row */}
                {data?.parent_path && (
                  <TableRow
                    className="cursor-pointer hover:bg-muted/50 transition-colors"
                    onClick={handleParentNavigate}
                    data-testid="parent-directory-row"
                  >
                    <TableCell className="text-center">
                      <ArrowUp className="w-4 h-4 text-muted-foreground mx-auto" />
                    </TableCell>
                    <TableCell colSpan={5} className="font-medium text-sm text-primary">
                      ..
                    </TableCell>
                  </TableRow>
                )}

                {isLoading ? (
                  <TableRow>
                    <TableCell colSpan={6} className="text-center py-10 text-muted-foreground">
                      {t("common.loading")}
                    </TableCell>
                  </TableRow>
                ) : filteredItems.length === 0 ? (
                  <TableRow>
                    <TableCell colSpan={6} className="text-center py-12 text-muted-foreground">
                      <Folder className="w-10 h-10 stroke-1 mx-auto mb-2 opacity-50" />
                      <p className="text-sm">{t("files.folderEmpty")}</p>
                      <p className="text-xs text-muted-foreground mt-1">
                        {t("files.dragDropToUpload")}
                      </p>
                    </TableCell>
                  </TableRow>
                ) : (
                  filteredItems.map((item) => {
                    const isDir = item.type === "directory";
                    const isSel = selectedIds.has(item.id);

                    return (
                      <TableRow
                        key={item.id}
                        className={`hover:bg-muted/50 transition-colors ${
                          isSel ? "bg-muted/40" : ""
                        }`}
                        data-testid={`file-row-${item.name}`}
                      >
                        <TableCell className="text-center" onClick={(e) => e.stopPropagation()}>
                          <button
                            type="button"
                            onClick={() => handleToggleSelect(item.id)}
                            className="p-1 text-muted-foreground hover:text-foreground"
                          >
                            {isSel ? (
                              <CheckSquare className="w-4 h-4 text-primary" />
                            ) : (
                              <Square className="w-4 h-4" />
                            )}
                          </button>
                        </TableCell>

                        <TableCell>
                          <div className="flex items-center gap-2.5">
                            {isDir ? (
                              <Folder className="w-4 h-4 text-amber-500 shrink-0" />
                            ) : isArchive(item) ? (
                              <FileArchive className="w-4 h-4 text-purple-500 shrink-0" />
                            ) : isEditable(item) ? (
                              <FileCode className="w-4 h-4 text-blue-500 shrink-0" />
                            ) : (
                              <FileText className="w-4 h-4 text-muted-foreground shrink-0" />
                            )}

                            {isDir ? (
                              <button
                                type="button"
                                onClick={() => handleNavigate(item.path)}
                                className="font-medium text-sm text-primary hover:underline truncate max-w-xs text-left"
                                data-testid={`dir-link-${item.name}`}
                              >
                                {item.name}
                              </button>
                            ) : (
                              <span
                                className="text-sm font-medium truncate max-w-xs"
                                data-testid={`file-name-${item.name}`}
                              >
                                {item.name}
                              </span>
                            )}
                          </div>
                        </TableCell>

                        <TableCell className="text-xs font-mono text-muted-foreground">
                          {isDir ? "-" : item.size_formatted}
                        </TableCell>

                        <TableCell>
                          <button
                            type="button"
                            onClick={() => setChmodTarget(item)}
                            className="text-xs font-mono px-1.5 py-0.5 rounded bg-muted hover:bg-muted-foreground/20 transition-colors"
                            title={t("files.changePermissions")}
                            data-testid={`chmod-trigger-${item.name}`}
                          >
                            {item.permissions}
                          </button>
                        </TableCell>

                        <TableCell className="text-xs text-muted-foreground">
                          {new Date(item.updated_at).toLocaleDateString()}
                        </TableCell>

                        <TableCell className="text-right">
                          <div className="flex items-center justify-end gap-1">
                            {isEditable(item) && (
                              <Button
                                size="sm"
                                variant="outline"
                                className="h-7 px-2 text-xs gap-1"
                                onClick={() => setEditorTarget(item)}
                                data-testid={`edit-button-${item.name}`}
                              >
                                <Edit className="w-3 h-3" />
                                <span>{t("common.edit")}</span>
                              </Button>
                            )}

                            {isArchive(item) && (
                              <Button
                                size="sm"
                                variant="outline"
                                className="h-7 px-2 text-xs gap-1"
                                onClick={() => setExtractTarget(item)}
                                data-testid={`extract-button-${item.name}`}
                              >
                                <Archive className="w-3 h-3" />
                                <span>{t("files.extract")}</span>
                              </Button>
                            )}

                            <Button
                              size="sm"
                              variant="outline"
                              className="h-7 px-1.5 text-xs"
                              title={t("files.rename")}
                              onClick={() => setRenameTarget(item)}
                              data-testid={`rename-button-${item.name}`}
                            >
                              <Edit className="w-3 h-3" />
                            </Button>

                            <Button
                              size="sm"
                              variant="destructive"
                              className="h-7 px-1.5 text-xs"
                              title={t("common.delete")}
                              onClick={() => handleDeleteItem(item)}
                              data-testid={`delete-button-${item.name}`}
                            >
                              <Trash2 className="w-3 h-3" />
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
        </div>
      </div>

      {/* Dialogs */}
      {createType && (
        <CreateItemDialog
          open={!!createType}
          onOpenChange={(open) => !open && setCreateType(null)}
          currentPath={currentPath}
          type={createType}
          onCreated={() => fetchFiles(currentPath)}
        />
      )}

      {renameTarget && (
        <RenameDialog
          open={!!renameTarget}
          onOpenChange={(open) => !open && setRenameTarget(null)}
          file={renameTarget}
          onRenamed={() => fetchFiles(currentPath)}
        />
      )}

      {chmodTarget && (
        <PermissionsDialog
          open={!!chmodTarget}
          onOpenChange={(open) => !open && setChmodTarget(null)}
          file={chmodTarget}
          onPermissionsUpdated={() => fetchFiles(currentPath)}
        />
      )}

      {editorTarget && (
        <FileEditorModal
          open={!!editorTarget}
          onOpenChange={(open) => !open && setEditorTarget(null)}
          file={editorTarget}
          onSaved={() => fetchFiles(currentPath)}
        />
      )}

      {archiveTargets.length > 0 && (
        <ArchiveDialog
          open={archiveTargets.length > 0}
          onOpenChange={(open) => !open && setArchiveTargets([])}
          items={archiveTargets}
          currentPath={currentPath}
          onArchived={() => fetchFiles(currentPath)}
        />
      )}

      {extractTarget && (
        <ExtractDialog
          open={!!extractTarget}
          onOpenChange={(open) => !open && setExtractTarget(null)}
          file={extractTarget}
          currentPath={currentPath}
          onExtracted={() => fetchFiles(currentPath)}
        />
      )}

      <TrashViewModal
        open={trashOpen}
        onOpenChange={setTrashOpen}
        onRestored={() => {
          fetchFiles(currentPath);
          fetchTrashCount();
        }}
      />
    </div>
  );
}
