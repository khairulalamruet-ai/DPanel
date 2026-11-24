import React from "react";
import { Input } from "@builtpanel/ui";
import { JsonSchemaProperty } from "./types";

export interface JsonSchemaFieldProps {
  name: string;
  property: JsonSchemaProperty;
  value: any;
  onChange: (value: any) => void;
  error?: string;
  disabled?: boolean;
  required?: boolean;
}

export function JsonSchemaField({
  name,
  property,
  value,
  onChange,
  error,
  disabled = false,
  required = false,
}: JsonSchemaFieldProps) {
  const label = property.title || name;
  const description = property.description;

  // 1. Enum (Select Dropdown)
  if (property.enum && property.enum.length > 0) {
    return (
      <div className="space-y-1.5">
        <label
          htmlFor={`field-${name}`}
          className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 flex items-center gap-1 text-foreground"
        >
          <span>{label}</span>
          {required && <span className="text-destructive font-bold">*</span>}
        </label>
        <div className="relative">
          <select
            id={`field-${name}`}
            name={name}
            value={value !== undefined ? String(value) : ""}
            disabled={disabled || property.readOnly}
            onChange={(e) => {
              const val = e.target.value;
              if (property.type === "integer" || property.type === "number") {
                onChange(val === "" ? undefined : Number(val));
              } else {
                onChange(val);
              }
            }}
            className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-1 text-sm shadow-sm transition-colors focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 text-foreground"
          >
            {property.enum.map((optVal, idx) => {
              const optLabel =
                property.enumNames && property.enumNames[idx]
                  ? property.enumNames[idx]
                  : String(optVal);
              return (
                <option key={String(optVal)} value={String(optVal)}>
                  {optLabel}
                </option>
              );
            })}
          </select>
        </div>
        {description && <p className="text-xs text-muted-foreground">{description}</p>}
        {error && <p className="text-xs text-destructive font-medium">{error}</p>}
      </div>
    );
  }

  // 2. Boolean (Toggle Switch)
  if (property.type === "boolean") {
    const isChecked = Boolean(value);
    return (
      <div className="flex items-start justify-between rounded-lg border p-3 shadow-sm bg-card">
        <div className="space-y-0.5 pr-4">
          <label
            htmlFor={`field-${name}`}
            className="text-sm font-medium leading-none cursor-pointer flex items-center gap-1 text-foreground"
            onClick={() => !disabled && onChange(!isChecked)}
          >
            <span>{label}</span>
            {required && <span className="text-destructive font-bold">*</span>}
          </label>
          {description && (
            <p className="text-xs text-muted-foreground pt-1">{description}</p>
          )}
          {error && <p className="text-xs text-destructive font-medium pt-1">{error}</p>}
        </div>
        <button
          type="button"
          role="switch"
          id={`field-${name}`}
          aria-checked={isChecked}
          disabled={disabled || property.readOnly}
          onClick={() => onChange(!isChecked)}
          className={`relative inline-flex h-6 w-11 shrink-0 cursor-pointer rounded-full border-2 border-transparent transition-colors duration-200 ease-in-out focus:outline-none focus:ring-2 focus:ring-ring focus:ring-offset-2 disabled:cursor-not-allowed disabled:opacity-50 ${
            isChecked ? "bg-primary" : "bg-muted"
          }`}
        >
          <span
            className={`pointer-events-none inline-block h-5 w-5 transform rounded-full bg-white shadow-lg ring-0 transition duration-200 ease-in-out ${
              isChecked ? "translate-x-5" : "translate-x-0"
            }`}
          />
        </button>
      </div>
    );
  }

  // 3. Integer or Number
  if (property.type === "integer" || property.type === "number") {
    return (
      <div className="space-y-1.5">
        <label
          htmlFor={`field-${name}`}
          className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 flex items-center gap-1 text-foreground"
        >
          <span>{label}</span>
          {required && <span className="text-destructive font-bold">*</span>}
        </label>
        <Input
          id={`field-${name}`}
          name={name}
          type="number"
          min={property.minimum}
          max={property.maximum}
          step={property.type === "integer" ? 1 : "any"}
          value={value !== undefined ? value : ""}
          disabled={disabled || property.readOnly}
          placeholder={property.default !== undefined ? String(property.default) : ""}
          onChange={(e) => {
            const val = e.target.value;
            onChange(val === "" ? undefined : Number(val));
          }}
          className={error ? "border-destructive focus-visible:ring-destructive" : ""}
        />
        {description && <p className="text-xs text-muted-foreground">{description}</p>}
        {error && <p className="text-xs text-destructive font-medium">{error}</p>}
      </div>
    );
  }

  // 4. String (Textarea)
  if (property.type === "string" && property.format === "textarea") {
    return (
      <div className="space-y-1.5">
        <label
          htmlFor={`field-${name}`}
          className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 flex items-center gap-1 text-foreground"
        >
          <span>{label}</span>
          {required && <span className="text-destructive font-bold">*</span>}
        </label>
        <textarea
          id={`field-${name}`}
          name={name}
          rows={3}
          value={value !== undefined ? String(value) : ""}
          disabled={disabled || property.readOnly}
          placeholder={property.default !== undefined ? String(property.default) : ""}
          onChange={(e) => onChange(e.target.value)}
          className={`flex min-h-[80px] w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-sm placeholder:text-muted-foreground focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring disabled:cursor-not-allowed disabled:opacity-50 text-foreground ${
            error ? "border-destructive focus-visible:ring-destructive" : ""
          }`}
        />
        {description && <p className="text-xs text-muted-foreground">{description}</p>}
        {error && <p className="text-xs text-destructive font-medium">{error}</p>}
      </div>
    );
  }

  // 5. Default String (Text, Email, Password, URI)
  const inputType =
    property.format === "password"
      ? "password"
      : property.format === "email"
      ? "email"
      : property.format === "uri"
      ? "url"
      : "text";

  return (
    <div className="space-y-1.5">
      <label
        htmlFor={`field-${name}`}
        className="text-sm font-medium leading-none peer-disabled:cursor-not-allowed peer-disabled:opacity-70 flex items-center gap-1 text-foreground"
      >
        <span>{label}</span>
        {required && <span className="text-destructive font-bold">*</span>}
      </label>
      <Input
        id={`field-${name}`}
        name={name}
        type={inputType}
        value={value !== undefined ? String(value) : ""}
        disabled={disabled || property.readOnly}
        placeholder={property.default !== undefined ? String(property.default) : ""}
        onChange={(e) => onChange(e.target.value)}
        className={error ? "border-destructive focus-visible:ring-destructive" : ""}
      />
      {description && <p className="text-xs text-muted-foreground">{description}</p>}
      {error && <p className="text-xs text-destructive font-medium">{error}</p>}
    </div>
  );
}
