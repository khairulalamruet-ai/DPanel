import React, { useState, useEffect } from "react";
import { Button } from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Save, RotateCcw } from "lucide-react";
import { JsonSchema } from "./types";
import { JsonSchemaField } from "./JsonSchemaField";

export interface JsonSchemaFormProps {
  schema: JsonSchema;
  initialValues?: Record<string, any>;
  onSubmit: (values: Record<string, any>) => Promise<void> | void;
  isSubmitting?: boolean;
  onReset?: () => void;
  className?: string;
}

export function JsonSchemaForm({
  schema,
  initialValues = {},
  onSubmit,
  isSubmitting = false,
  onReset,
  className = "",
}: JsonSchemaFormProps) {
  const { t } = useTranslation();
  const [values, setValues] = useState<Record<string, any>>(initialValues);
  const [errors, setErrors] = useState<Record<string, string>>({});

  useEffect(() => {
    // Fill defaults for any missing values
    const merged: Record<string, any> = { ...initialValues };
    if (schema?.properties) {
      for (const [key, prop] of Object.entries(schema.properties)) {
        if (merged[key] === undefined && prop.default !== undefined) {
          merged[key] = prop.default;
        }
      }
    }
    setValues(merged);
    setErrors({});
  }, [schema, initialValues]);

  const handleChange = (fieldName: string, val: any) => {
    setValues((prev) => ({
      ...prev,
      [fieldName]: val,
    }));
    // Clear field error on change
    if (errors[fieldName]) {
      setErrors((prev) => {
        const next = { ...prev };
        delete next[fieldName];
        return next;
      });
    }
  };

  const validate = (): boolean => {
    const newErrors: Record<string, string> = {};
    const required = schema.required || [];

    for (const [key, prop] of Object.entries(schema.properties || {})) {
      const val = values[key];

      // 1. Required check
      if (required.includes(key)) {
        if (val === undefined || val === null || val === "") {
          newErrors[key] = t("settings.form.requiredField");
          continue;
        }
      }

      // 2. Email format check
      if (prop.format === "email" && val) {
        const emailRegex = /^[^\s@]+@[^\s@]+\.[^\s@]+$/;
        if (!emailRegex.test(String(val))) {
          newErrors[key] = t("settings.form.invalidEmail");
          continue;
        }
      }

      // 3. Integer check
      if (prop.type === "integer" && val !== undefined && val !== null && val !== "") {
        if (!Number.isInteger(Number(val))) {
          newErrors[key] = t("settings.form.invalidInteger");
          continue;
        }
      }

      // 4. Min / Max checks
      if ((prop.type === "integer" || prop.type === "number") && val !== undefined && val !== "") {
        const num = Number(val);
        if (prop.minimum !== undefined && num < prop.minimum) {
          newErrors[key] = t("settings.form.minValue", { min: prop.minimum });
          continue;
        }
        if (prop.maximum !== undefined && num > prop.maximum) {
          newErrors[key] = t("settings.form.maxValue", { max: prop.maximum });
          continue;
        }
      }
    }

    setErrors(newErrors);
    return Object.keys(newErrors).length === 0;
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (validate()) {
      onSubmit(values);
    }
  };

  const handleReset = () => {
    setValues(initialValues);
    setErrors({});
    if (onReset) onReset();
  };

  const properties = schema?.properties || {};
  const requiredList = schema?.required || [];

  return (
    <form noValidate onSubmit={handleSubmit} className={`space-y-6 ${className}`}>
      <div className="space-y-5">
        {Object.entries(properties).map(([name, property]) => (
          <JsonSchemaField
            key={name}
            name={name}
            property={property}
            value={values[name]}
            onChange={(val) => handleChange(name, val)}
            error={errors[name]}
            disabled={isSubmitting}
            required={requiredList.includes(name)}
          />
        ))}
      </div>

      <div className="flex items-center gap-3 pt-4 border-t">
        <Button type="submit" disabled={isSubmitting} className="gap-2">
          <Save className="h-4 w-4" />
          {isSubmitting ? t("settings.form.saving") : t("settings.form.save")}
        </Button>
        <Button
          type="button"
          variant="outline"
          disabled={isSubmitting}
          onClick={handleReset}
          className="gap-2"
        >
          <RotateCcw className="h-4 w-4" />
          {t("settings.form.reset")}
        </Button>
      </div>
    </form>
  );
}
