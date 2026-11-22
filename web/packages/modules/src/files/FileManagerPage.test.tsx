import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, waitFor, fireEvent } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { FileManagerPage } from "./FileManagerPage";

/** The file manager opens at the top of the account's files; the web files are in public_html. */
async function openPublicHtml() {
  await waitFor(() => {
    expect(screen.getByTestId("shortcut-public-html")).toBeInTheDocument();
  });
  fireEvent.click(screen.getByTestId("shortcut-public-html"));
}

function renderWithProviders(ui: React.ReactElement) {
  return render(<ToastProvider>{ui}</ToastProvider>);
}

describe("FileManagerPage (T08.09)", () => {
  it("renders file manager page title, toolbar, shortcuts, and initial files", async () => {
    renderWithProviders(<FileManagerPage />);

    await waitFor(() => {
      expect(screen.getByTestId("file-manager-title")).toBeInTheDocument();
    });

    // Check action buttons
    expect(screen.getByTestId("new-file-button")).toBeInTheDocument();
    expect(screen.getByTestId("new-folder-button")).toBeInTheDocument();
    expect(screen.getByTestId("upload-button")).toBeInTheDocument();
    expect(screen.getByTestId("trash-view-button")).toBeInTheDocument();

    // Check directory shortcuts
    expect(screen.getByTestId("shortcut-home")).toBeInTheDocument();
    expect(screen.getByTestId("shortcut-public-html")).toBeInTheDocument();
    expect(screen.getByTestId("shortcut-logs")).toBeInTheDocument();

    // Check seed items in public_html
    fireEvent.click(screen.getByTestId("shortcut-public-html"));
    await waitFor(() => {
      expect(screen.getByTestId("file-name-index.php")).toBeInTheDocument();
      expect(screen.getByTestId("file-name-style.css")).toBeInTheDocument();
      expect(screen.getByTestId("file-name-.htaccess")).toBeInTheDocument();
      expect(screen.getByTestId("file-name-backup.zip")).toBeInTheDocument();
    });
  });

  it("filters file list with search input", async () => {
    renderWithProviders(<FileManagerPage />);
    await openPublicHtml();

    await waitFor(() => {
      expect(screen.getByTestId("file-name-index.php")).toBeInTheDocument();
    });

    const searchInput = screen.getByTestId("search-files-input");
    fireEvent.change(searchInput, { target: { value: "style" } });

    expect(screen.getByTestId("file-name-style.css")).toBeInTheDocument();
    expect(screen.queryByTestId("file-name-index.php")).not.toBeInTheDocument();
  });

  it("opens code editor modal for editable file and allows saving", async () => {
    renderWithProviders(<FileManagerPage />);
    await openPublicHtml();

    await waitFor(() => {
      expect(screen.getByTestId("edit-button-index.php")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("edit-button-index.php"));

    await waitFor(() => {
      expect(screen.getByTestId("file-editor-modal")).toBeInTheDocument();
      expect(screen.getByTestId("file-editor-filename")).toHaveTextContent("index.php");
    });

    const textarea = screen.getByTestId("textarea-file-editor");
    expect(textarea).toHaveValue("<?php\necho 'Hello from Built Panel!';\n");

    fireEvent.change(textarea, {
      target: { value: "<?php\necho 'Updated Built Panel code!';\n" },
    });

    const saveBtn = screen.getByTestId("btn-save-file-editor");
    fireEvent.click(saveBtn);

    await waitFor(() => {
      expect(saveBtn).toBeDisabled();
    });
  });

  it("opens chmod permissions dialog and updates octal permissions", async () => {
    renderWithProviders(<FileManagerPage />);
    await openPublicHtml();

    await waitFor(() => {
      expect(screen.getByTestId("chmod-trigger-index.php")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("chmod-trigger-index.php"));

    await waitFor(() => {
      expect(screen.getByTestId("permissions-dialog")).toBeInTheDocument();
    });

    // Toggle execute permission on owner
    const ownerExecCheckbox = screen.getByTestId("chmod-owner-exec");
    fireEvent.click(ownerExecCheckbox);

    // Should update mode display to 0744
    expect(screen.getByTestId("chmod-octal-value")).toHaveTextContent("0744");

    fireEvent.click(screen.getByTestId("btn-save-chmod"));

    await waitFor(() => {
      expect(screen.queryByTestId("permissions-dialog")).not.toBeInTheDocument();
    });
  });

  it("navigates into directory shortcut logs and parent directory", async () => {
    renderWithProviders(<FileManagerPage />);

    await waitFor(() => {
      expect(screen.getByTestId("shortcut-logs")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("shortcut-logs"));

    await waitFor(() => {
      expect(screen.getByTestId("file-name-access.log")).toBeInTheDocument();
    });

    // Parent row should be present
    expect(screen.getByTestId("parent-directory-row")).toBeInTheDocument();
    fireEvent.click(screen.getByTestId("parent-directory-row"));

    await waitFor(() => {
      expect(screen.getByTestId("dir-link-public_html")).toBeInTheDocument();
    });
  });

  it("creates a new file and displays it in the table", async () => {
    renderWithProviders(<FileManagerPage />);

    await waitFor(() => {
      expect(screen.getByTestId("new-file-button")).toBeInTheDocument();
    });

    fireEvent.click(screen.getByTestId("new-file-button"));

    await waitFor(() => {
      expect(screen.getByTestId("create-dialog-title")).toBeInTheDocument();
    });

    const nameInput = screen.getByTestId("create-item-name-input");
    fireEvent.change(nameInput, { target: { value: "test-new.txt" } });

    fireEvent.click(screen.getByTestId("create-item-submit-button"));

    await waitFor(() => {
      expect(screen.getByTestId("file-name-test-new.txt")).toBeInTheDocument();
    });
  });

  it("deletes a file to trash and restores it from trash modal (DoD requirement)", async () => {
    // Mock window.confirm
    const confirmSpy = vi.spyOn(window, "confirm").mockImplementation(() => true);

    renderWithProviders(<FileManagerPage />);
    await openPublicHtml();

    await waitFor(() => {
      expect(screen.getByTestId("delete-button-style.css")).toBeInTheDocument();
    });

    // Delete style.css
    fireEvent.click(screen.getByTestId("delete-button-style.css"));

    await waitFor(() => {
      expect(screen.queryByTestId("file-name-style.css")).not.toBeInTheDocument();
    });

    // Open trash modal
    fireEvent.click(screen.getByTestId("trash-view-button"));

    await waitFor(() => {
      expect(screen.getByTestId("trash-modal-title")).toBeInTheDocument();
      expect(document.querySelector('[data-testid^="trash-item-"]')).not.toBeNull();
    });

    // Click restore
    fireEvent.click(document.querySelector('[data-testid^="restore-button-"]') as Element);

    // Close trash modal
    fireEvent.click(screen.getByTestId("close-trash-modal"));

    // style.css is back in public_html (the listing is read again once the modal reports a restore)
    fireEvent.click(screen.getByTestId("shortcut-public-html"));
    await waitFor(() => {
      expect(screen.getByTestId("file-name-style.css")).toBeInTheDocument();
    });

    confirmSpy.mockRestore();
  });
});
