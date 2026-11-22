import React from "react";
import { useTranslation } from "react-i18next";

export function LanguageSwitcher({ onChange }: { onChange?: (locale: string) => void } = {}) {
  const { i18n, t } = useTranslation();

  const toggleLanguage = () => {
    const nextLng = i18n.language.startsWith("bn") ? "en" : "bn";
    i18n.changeLanguage(nextLng);
    if (typeof window !== "undefined" && typeof window.localStorage !== "undefined") {
      window.localStorage?.setItem?.("builtpanel-locale", nextLng);
    }
    onChange?.(nextLng);
  };

  return (
    <button
      onClick={toggleLanguage}
      className="inline-flex items-center gap-1.5 rounded-md border border-input bg-background px-2.5 py-1.5 text-xs font-medium shadow-sm transition-colors hover:bg-accent hover:text-accent-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
      aria-label={t("common.language")}
    >
      <span className="font-semibold">{i18n.language.startsWith("bn") ? "বাং" : "EN"}</span>
    </button>
  );
}
