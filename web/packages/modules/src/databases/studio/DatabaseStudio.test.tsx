import React from "react";
import { describe, it, expect, beforeEach, vi } from "vitest";
import { render, screen, waitFor, fireEvent, within } from "@testing-library/react";
import { ToastProvider } from "@builtpanel/ui";
import { resetMockDatabases } from "@builtpanel/api";
import { DatabaseStudio } from "./DatabaseStudio";
import { toCsv } from "./api";

const db = { id: "db_01", name: "demo_wordpress" };

function renderStudio(onClose = vi.fn()) {
  return render(
    <ToastProvider>
      <DatabaseStudio database={db} onClose={onClose} />
    </ToastProvider>,
  );
}

async function openCustomers() {
  fireEvent.click(await screen.findByTestId("table-customers"));
  await screen.findByTestId("studio-row-0");
}

describe("DatabaseStudio", () => {
  beforeEach(() => {
    resetMockDatabases();
  });

  it("lists the tables and shows the rows of the one picked, NULL apart from text", async () => {
    renderStudio();
    expect(await screen.findByTestId("studio-title")).toHaveTextContent("demo_wordpress");
    expect(screen.getByTestId("pick-table")).toBeInTheDocument();
    await openCustomers();
    expect(screen.getByTestId("page-range")).toHaveTextContent("1–3 / 3");
    const second = screen.getByTestId("studio-row-1");
    expect(within(second).getByText("NULL")).toBeInTheDocument();
    expect(within(second).getByText("Alan Turing")).toBeInTheDocument();
  });

  it("sorts by a column and filters with a condition", async () => {
    renderStudio();
    await openCustomers();
    fireEvent.click(screen.getByTestId("sort-name"));
    fireEvent.click(screen.getByTestId("sort-name"));
    await waitFor(() => expect(within(screen.getByTestId("studio-row-0")).getByText("Grace Hopper")).toBeInTheDocument());

    fireEvent.change(screen.getByTestId("filter-column"), { target: { value: "email" } });
    fireEvent.change(screen.getByTestId("filter-op"), { target: { value: "is_null" } });
    fireEvent.click(screen.getByTestId("filter-add"));
    await waitFor(() => expect(screen.getByTestId("page-range")).toHaveTextContent("1–1 / 1"));
    expect(screen.getByTestId("filter-chips")).toHaveTextContent("email IS NULL");
    expect(screen.queryByText("Grace Hopper")).not.toBeInTheDocument();
  });

  it("adds, changes and deletes a row by its primary key", async () => {
    renderStudio();
    await openCustomers();

    fireEvent.click(screen.getByTestId("row-add"));
    fireEvent.change(await screen.findByTestId("field-name"), { target: { value: "Linus" } });
    fireEvent.click(screen.getByTestId("row-save"));
    await waitFor(() => expect(screen.getByTestId("page-range")).toHaveTextContent("1–4 / 4"));
    expect(screen.getByText("Linus")).toBeInTheDocument();

    fireEvent.click(screen.getByTestId("row-edit-0"));
    const name = await screen.findByTestId("field-name");
    expect(name).toHaveValue("Ada Lovelace");
    fireEvent.change(name, { target: { value: "Ada King" } });
    fireEvent.click(screen.getByTestId("row-save"));
    await waitFor(() => expect(screen.getByText("Ada King")).toBeInTheDocument());

    fireEvent.click(screen.getByTestId("row-delete-0"));
    fireEvent.click(await screen.findByTestId("row-delete-confirm"));
    await waitFor(() => expect(screen.getByTestId("page-range")).toHaveTextContent("1–3 / 3"));
    expect(screen.queryByText("Ada King")).not.toBeInTheDocument();
  });

  it("shows the structure with the key, the indexes and the CREATE statement", async () => {
    renderStudio();
    await openCustomers();
    fireEvent.click(screen.getByTestId("tab-structure"));
    expect(await screen.findByTestId("column-id")).toBeInTheDocument();
    expect(screen.getByTestId("column-email")).toHaveTextContent("varchar(120)");
    expect(screen.getByText("PRIMARY")).toBeInTheDocument();
    expect(screen.getByTestId("create-sql")).toHaveTextContent("CREATE TABLE `customers`");
  });

  it("runs SQL: rows come back as a grid, a change as a count, a server error in its words", async () => {
    renderStudio();
    fireEvent.click(await screen.findByTestId("tab-sql"));
    const input = screen.getByTestId("sql-input");

    fireEvent.change(input, { target: { value: "SELECT * FROM customers" } });
    fireEvent.keyDown(input, { key: "Enter", ctrlKey: true });
    expect(await screen.findByTestId("sql-grid")).toBeInTheDocument();
    expect(screen.getByTestId("sql-outcome")).toHaveTextContent("3 rows in 3 ms");
    expect(screen.getByTestId("sql-csv")).toBeInTheDocument();

    fireEvent.change(input, { target: { value: "DELETE FROM customers" } });
    fireEvent.click(screen.getByTestId("sql-run"));
    await waitFor(() => expect(screen.getByTestId("sql-outcome")).toHaveTextContent("1 rows affected"));

    fireEvent.change(input, { target: { value: "SELEC nonsense" } });
    fireEvent.click(screen.getByTestId("sql-run"));
    expect(await screen.findByTestId("sql-error")).toHaveTextContent("error 1064");
  });

  it("goes back to the databases", async () => {
    const onClose = vi.fn();
    renderStudio(onClose);
    fireEvent.click(await screen.findByTestId("studio-back"));
    expect(onClose).toHaveBeenCalled();
  });
});

describe("toCsv", () => {
  it("quotes what needs it and writes NULL as an empty field", () => {
    expect(
      toCsv({ columns: ["a", "b"], rows: [["x,y", 'say "hi"'], [null, "line\nbreak"]], truncated: false }),
    ).toBe('a,b\r\n"x,y","say ""hi"""\r\n,"line\nbreak"');
  });
});
