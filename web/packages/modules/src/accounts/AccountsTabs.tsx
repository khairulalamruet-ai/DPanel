import React from "react";
import { useTranslation } from "@builtpanel/i18n";

export type AccountsTab = "users" | "resellers" | "administrators";

const TABS: { id: AccountsTab; href: string; key: string; fallback: string }[] = [
  { id: "users", href: "/users", key: "accounts.tabs.users", fallback: "Users" },
  { id: "resellers", href: "/admin/resellers", key: "accounts.tabs.resellers", fallback: "Resellers" },
  { id: "administrators", href: "/administrators", key: "accounts.tabs.administrators", fallback: "Administrators" },
];

/** The tab bar shared by the three account lists: users, resellers and administrators. */
export function AccountsTabs({ active }: { active: AccountsTab }) {
  const { t } = useTranslation();
  return (
    <nav aria-label={t("accounts.title", "Accounts")} className="border-b" data-testid="accounts-tabs">
      <ul className="flex gap-6">
        {TABS.map((tab) => {
          const current = tab.id === active;
          return (
            <li key={tab.id}>
              <a
                href={tab.href}
                aria-current={current ? "page" : undefined}
                className={`-mb-px inline-block border-b-2 py-3 text-sm font-medium transition-colors ${
                  current
                    ? "border-primary text-primary"
                    : "border-transparent text-muted-foreground hover:text-foreground"
                }`}
              >
                {t(tab.key, tab.fallback)}
              </a>
            </li>
          );
        })}
      </ul>
    </nav>
  );
}
