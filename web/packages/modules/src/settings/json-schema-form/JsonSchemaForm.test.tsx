import React from "react";
import { describe, it, expect, vi } from "vitest";
import { render, screen, fireEvent, waitFor } from "@testing-library/react";
import { JsonSchemaForm } from "./JsonSchemaForm";
import { JsonSchema } from "./types";

describe("JsonSchemaForm (T03.13)", () => {
  const sampleSchema: JsonSchema = {
    type: "object",
    title: "Test Settings",
    required: ["site_title", "admin_email"],
    properties: {
      site_title: {
        type: "string",
        title: "Site Title",
        description: "Public name of the application",
        default: "My Panel",
      },
      admin_email: {
        type: "string",
        format: "email",
        title: "Admin Email",
      },
      max_upload_size: {
        type: "integer",
        title: "Max Upload Size (MB)",
        minimum: 1,
        maximum: 1000,
        default: 50,
      },
      enable_registration: {
        type: "boolean",
        title: "Enable Registration",
        description: "Allow new users to sign up",
        default: false,
      },
      theme: {
        type: "string",
        title: "Default Theme",
        enum: ["light", "dark", "system"],
        enumNames: ["Light Mode", "Dark Mode", "Follow System"],
        default: "system",
      },
    },
  };

  it("renders all fields based on JSON Schema types (string, int, bool, enum)", () => {
    render(
      <JsonSchemaForm
        schema={sampleSchema}
        initialValues={{ site_title: "Built Panel" }}
        onSubmit={vi.fn()}
      />
    );

    // 1. String field
    expect(screen.getByLabelText(/Site Title/i)).toBeInTheDocument();
    expect(screen.getByText("Public name of the application")).toBeInTheDocument();

    // 2. Email format field
    expect(screen.getByLabelText(/Admin Email/i)).toBeInTheDocument();

    // 3. Integer field
    expect(screen.getByLabelText(/Max Upload Size/i)).toBeInTheDocument();

    // 4. Boolean toggle
    expect(screen.getByLabelText(/Enable Registration/i)).toBeInTheDocument();
    expect(screen.getByRole("switch")).toBeInTheDocument();

    // 5. Enum select
    const select = screen.getByLabelText(/Default Theme/i);
    expect(select).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Light Mode" })).toBeInTheDocument();
    expect(screen.getByRole("option", { name: "Dark Mode" })).toBeInTheDocument();
  });

  it("DoD: dynamically renders newly added schema setting without writing UI code", () => {
    // Dynamically extend schema with a brand-new setting property
    const extendedSchema: JsonSchema = {
      ...sampleSchema,
      properties: {
        ...sampleSchema.properties,
        cloud_region: {
          type: "string",
          title: "Cloud Deployment Region",
          description: "Target datacenter for node provisioning",
          default: "us-east-1",
        },
        worker_threads: {
          type: "integer",
          title: "Worker Thread Count",
          default: 8,
        },
      },
    };

    render(
      <JsonSchemaForm
        schema={extendedSchema}
        initialValues={{}}
        onSubmit={vi.fn()}
      />
    );

    // Verify the new fields appear in the form automatically
    expect(screen.getByLabelText(/Cloud Deployment Region/i)).toBeInTheDocument();
    expect(screen.getByText("Target datacenter for node provisioning")).toBeInTheDocument();
    expect(screen.getByLabelText(/Worker Thread Count/i)).toBeInTheDocument();
  });

  it("validates required fields on submit", async () => {
    const handleSubmit = vi.fn();
    render(
      <JsonSchemaForm
        schema={sampleSchema}
        initialValues={{ site_title: "" }} // Required field left empty
        onSubmit={handleSubmit}
      />
    );

    const submitBtn = screen.getByRole("button", { name: /Save Changes|পরিবর্তন সংরক্ষণ করুন/i });
    fireEvent.click(submitBtn);

    // Validation error should show and submit not called
    await waitFor(() => {
      expect(screen.getAllByText(/This field is required|এই তথ্যটি পূরণ করা আবশ্যক/i).length).toBeGreaterThan(0);
    });
    expect(handleSubmit).not.toHaveBeenCalled();
  });

  it("validates email format on submit", async () => {
    const handleSubmit = vi.fn();
    render(
      <JsonSchemaForm
        schema={sampleSchema}
        initialValues={{ site_title: "Valid Title", admin_email: "not-an-email" }}
        onSubmit={handleSubmit}
      />
    );

    const submitBtn = screen.getByRole("button", { name: /Save Changes|পরিবর্তন সংরক্ষণ করুন/i });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(
        screen.getByText(/Please enter a valid email address|সঠিক ইমেইল ঠিকানা প্রদান করুন/i)
      ).toBeInTheDocument();
    });
    expect(handleSubmit).not.toHaveBeenCalled();
  });

  it("calls onSubmit with valid form data", async () => {
    const handleSubmit = vi.fn();
    render(
      <JsonSchemaForm
        schema={sampleSchema}
        initialValues={{
          site_title: "My Custom Built Panel",
          admin_email: "superadmin@example.com",
          max_upload_size: 100,
          enable_registration: true,
          theme: "dark",
        }}
        onSubmit={handleSubmit}
      />
    );

    const submitBtn = screen.getByRole("button", { name: /Save Changes|পরিবর্তন সংরক্ষণ করুন/i });
    fireEvent.click(submitBtn);

    await waitFor(() => {
      expect(handleSubmit).toHaveBeenCalledWith({
        site_title: "My Custom Built Panel",
        admin_email: "superadmin@example.com",
        max_upload_size: 100,
        enable_registration: true,
        theme: "dark",
      });
    });
  });

  it("toggles boolean value when switch is clicked", () => {
    const handleSubmit = vi.fn();
    render(
      <JsonSchemaForm
        schema={sampleSchema}
        initialValues={{
          site_title: "Test Panel",
          admin_email: "admin@test.com",
          enable_registration: false,
        }}
        onSubmit={handleSubmit}
      />
    );

    const switchBtn = screen.getByRole("switch");
    expect(switchBtn).toHaveAttribute("aria-checked", "false");

    fireEvent.click(switchBtn);
    expect(switchBtn).toHaveAttribute("aria-checked", "true");

    const submitBtn = screen.getByRole("button", { name: /Save Changes|পরিবর্তন সংরক্ষণ করুন/i });
    fireEvent.click(submitBtn);

    expect(handleSubmit).toHaveBeenCalledWith(
      expect.objectContaining({
        enable_registration: true,
      })
    );
  });
});
