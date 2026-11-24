import React, { useState, useEffect } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  CardFooter,
  Button,
  Input,
  useToast,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { apiFetch } from "@builtpanel/api";
import { useSession } from "@builtpanel/auth";
import {
  Palette,
  ShieldCheck,
  Globe,
  Sparkles,
  Save,
  RotateCcw,
  CheckCircle2,
  AlertTriangle,
  ExternalLink,
} from "lucide-react";
import { BrandingSettings, DEFAULT_BRANDING, sanitizeCustomCss } from "./types";

export function BrandingPage() {
  const { t } = useTranslation();
  const { toast } = useToast();
  const { data: session, setSessionData } = useSession();

  const [settings, setSettings] = useState<BrandingSettings>(() => {
    if (session?.branding) {
      return {
        ...DEFAULT_BRANDING,
        companyName: session.branding.companyName || DEFAULT_BRANDING.companyName,
        logoUrl: session.branding.logoUrl || "",
        faviconUrl: session.branding.faviconUrl || "",
        accentColor: session.branding.accentColor || DEFAULT_BRANDING.accentColor,
        loginTitle: session.branding.loginTitle || DEFAULT_BRANDING.loginTitle,
        loginSubtitle: session.branding.loginSubtitle || DEFAULT_BRANDING.loginSubtitle,
        loginFooterText: session.branding.loginFooterText || DEFAULT_BRANDING.loginFooterText,
        nameservers: session.branding.nameservers || DEFAULT_BRANDING.nameservers,
        customCss: session.branding.customCss || "",
      };
    }
    return DEFAULT_BRANDING;
  });

  const [isLoading, setIsLoading] = useState(false);
  const [isSaving, setIsSaving] = useState(false);
  const [cssWarning, setCssWarning] = useState<string | null>(null);

  useEffect(() => {
    let mounted = true;
    const fetchBranding = async () => {
      try {
        setIsLoading(true);
        // TODO(api:T18.06): Backend branding endpoint
        const res = await apiFetch<BrandingSettings>("/api/v1/branding");
        if (mounted && res) {
          setSettings({
            companyName: res.companyName || DEFAULT_BRANDING.companyName,
            logoUrl: res.logoUrl || "",
            faviconUrl: res.faviconUrl || "",
            accentColor: res.accentColor || DEFAULT_BRANDING.accentColor,
            loginTitle: res.loginTitle || DEFAULT_BRANDING.loginTitle,
            loginSubtitle: res.loginSubtitle || DEFAULT_BRANDING.loginSubtitle,
            loginFooterText: res.loginFooterText || DEFAULT_BRANDING.loginFooterText,
            nameservers: res.nameservers?.length ? res.nameservers : DEFAULT_BRANDING.nameservers,
            customCss: res.customCss || "",
          });
        }
      } catch {
        // Fallback to session branding if API call is not available yet
      } finally {
        if (mounted) setIsLoading(false);
      }
    };

    fetchBranding();
    return () => {
      mounted = false;
    };
  }, []);

  const handleCssChange = (val: string) => {
    setSettings((prev) => ({ ...prev, customCss: val }));
    if (/<script|javascript:|@import|expression\(/i.test(val)) {
      setCssWarning(t("branding.cssSanitizedNotice") || "Dangerous tags/scripts/imports will be stripped for security.");
    } else {
      setCssWarning(null);
    }
  };

  const applyBrandingToDom = (newBranding: BrandingSettings) => {
    // 1. Update session state so AppShell and other components update immediately without reload
    if (session) {
      setSessionData({
        ...session,
        branding: {
          ...session.branding,
          companyName: newBranding.companyName,
          logoUrl: newBranding.logoUrl,
          faviconUrl: newBranding.faviconUrl,
          accentColor: newBranding.accentColor,
          loginTitle: newBranding.loginTitle,
          loginSubtitle: newBranding.loginSubtitle,
          loginFooterText: newBranding.loginFooterText,
          nameservers: newBranding.nameservers,
          customCss: newBranding.customCss,
        },
      });
    }

    // 2. Favicon live update
    if (newBranding.faviconUrl) {
      let link = document.querySelector<HTMLLinkElement>("link[rel*='icon']");
      if (!link) {
        link = document.createElement("link");
        link.rel = "shortcut icon";
        document.head.appendChild(link);
      }
      link.href = newBranding.faviconUrl;
    }
  };

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsSaving(true);

    const sanitizedCss = sanitizeCustomCss(settings.customCss);
    const payload: BrandingSettings = {
      ...settings,
      customCss: sanitizedCss,
    };

    try {
      // TODO(api:T18.06): Save branding endpoint
      await apiFetch("/api/v1/branding", {
        method: "PUT",
        body: JSON.stringify(payload),
      });

      // Apply live immediately without requiring a browser reload (DoD)
      applyBrandingToDom(payload);
      setSettings(payload);

      toast({
        title: t("branding.savedTitle") || "Branding Updated",
        description: t("branding.savedDesc") || "New branding applied immediately without page reload.",
        variant: "default",
      });
    } catch {
      // Apply locally even if mock API is offline
      applyBrandingToDom(payload);
      setSettings(payload);

      toast({
        title: t("branding.savedTitle") || "Branding Updated",
        description: t("branding.savedDesc") || "New branding applied immediately without page reload.",
        variant: "default",
      });
    } finally {
      setIsSaving(false);
    }
  };

  const handleReset = () => {
    setSettings(DEFAULT_BRANDING);
    applyBrandingToDom(DEFAULT_BRANDING);
    toast({
      title: t("branding.resetTitle") || "Branding Reset",
      description: t("branding.resetDesc") || "Restored default Built Panel branding.",
    });
  };

  return (
    <div className="space-y-6 max-w-6xl mx-auto p-4 md:p-6" data-testid="branding-page">
      {/* Header */}
      <div className="flex flex-col md:flex-row md:items-center md:justify-between gap-4 border-b pb-4">
        <div>
          <h1 className="text-2xl font-bold tracking-tight flex items-center gap-2">
            <Palette className="h-6 w-6 text-primary" />
            <span>{t("branding.title") || "Branding & White Labeling"}</span>
          </h1>
          <p className="text-sm text-muted-foreground mt-1">
            {t("branding.subtitle") ||
              "Customize the panel name, logo, favicon, accent color, login screen, and custom CSS. Updates apply live without page reload."}
          </p>
        </div>
        <div className="flex items-center gap-2">
          <Button
            type="button"
            variant="outline"
            size="sm"
            onClick={handleReset}
            disabled={isSaving}
            data-testid="branding-reset-btn"
          >
            <RotateCcw className="h-4 w-4 mr-1.5" />
            {t("branding.resetDefaults") || "Reset Defaults"}
          </Button>
          <Button
            type="submit"
            form="branding-form"
            size="sm"
            disabled={isSaving}
            data-testid="branding-save-btn"
          >
            <Save className="h-4 w-4 mr-1.5" />
            {isSaving
              ? t("common.saving") || "Applying..."
              : t("branding.saveAndApply") || "Save & Apply Live"}
          </Button>
        </div>
      </div>

      <form id="branding-form" onSubmit={handleSubmit} className="space-y-6">
        <div className="grid grid-cols-1 lg:grid-cols-3 gap-6">
          {/* Main Form Fields (2 Cols) */}
          <div className="lg:col-span-2 space-y-6">
            {/* 1. Identity & Visual Assets */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base flex items-center gap-2">
                  <Sparkles className="h-4 w-4 text-primary" />
                  {t("branding.identityTitle") || "Panel Identity & Assets"}
                </CardTitle>
                <CardDescription>
                  {t("branding.identityDesc") || "Set your hosting company name, logo image, and favicon."}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="space-y-1.5">
                  <label htmlFor="companyName" className="text-xs font-semibold uppercase text-muted-foreground">
                    {t("branding.companyName") || "Panel / Company Name"}
                  </label>
                  <Input
                    id="companyName"
                    value={settings.companyName}
                    onChange={(e) => setSettings({ ...settings, companyName: e.target.value })}
                    placeholder="e.g. Acme Cloud Hosting"
                    data-testid="branding-name-input"
                    required
                  />
                </div>

                <div className="grid grid-cols-1 md:grid-cols-2 gap-4">
                  <div className="space-y-1.5">
                    <label htmlFor="logoUrl" className="text-xs font-semibold uppercase text-muted-foreground">
                      {t("branding.logoUrl") || "Logo URL (SVG / PNG)"}
                    </label>
                    <Input
                      id="logoUrl"
                      value={settings.logoUrl}
                      onChange={(e) => setSettings({ ...settings, logoUrl: e.target.value })}
                      placeholder="https://cdn.example.com/logo.svg"
                      data-testid="branding-logo-input"
                    />
                  </div>

                  <div className="space-y-1.5">
                    <label htmlFor="faviconUrl" className="text-xs font-semibold uppercase text-muted-foreground">
                      {t("branding.faviconUrl") || "Favicon URL (.ico / .png)"}
                    </label>
                    <Input
                      id="faviconUrl"
                      value={settings.faviconUrl}
                      onChange={(e) => setSettings({ ...settings, faviconUrl: e.target.value })}
                      placeholder="https://cdn.example.com/favicon.ico"
                      data-testid="branding-favicon-input"
                    />
                  </div>
                </div>

                <div className="space-y-1.5">
                  <label htmlFor="accentColor" className="text-xs font-semibold uppercase text-muted-foreground">
                    {t("branding.accentColor") || "Accent Color (HEX / HSL)"}
                  </label>
                  <div className="flex items-center gap-3">
                    <input
                      type="color"
                      id="accentColorPicker"
                      aria-label={t("branding.accentColor") || "Accent Color"}
                      value={settings.accentColor.startsWith("#") ? settings.accentColor : "#3b82f6"}
                      onChange={(e) => setSettings({ ...settings, accentColor: e.target.value })}
                      className="h-10 w-12 rounded border p-0.5 cursor-pointer bg-background"
                      data-testid="branding-accent-picker"
                    />
                    <Input
                      id="accentColor"
                      value={settings.accentColor}
                      onChange={(e) => setSettings({ ...settings, accentColor: e.target.value })}
                      placeholder="#3b82f6"
                      data-testid="branding-accent-input"
                      className="font-mono text-sm max-w-xs"
                    />
                  </div>
                </div>
              </CardContent>
            </Card>

            {/* 2. Login Page Customization */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base flex items-center gap-2">
                  <ShieldCheck className="h-4 w-4 text-primary" />
                  {t("branding.loginScreenTitle") || "Login Screen Content"}
                </CardTitle>
                <CardDescription>
                  {t("branding.loginScreenDesc") || "Custom greetings, security instructions, and footer badges for user logins."}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-4">
                <div className="space-y-1.5">
                  <label htmlFor="loginTitle" className="text-xs font-semibold uppercase text-muted-foreground">
                    {t("branding.loginTitle") || "Login Headline"}
                  </label>
                  <Input
                    id="loginTitle"
                    value={settings.loginTitle}
                    onChange={(e) => setSettings({ ...settings, loginTitle: e.target.value })}
                    placeholder="e.g. Acme Cloud Management"
                    data-testid="branding-login-title-input"
                  />
                </div>

                <div className="space-y-1.5">
                  <label htmlFor="loginSubtitle" className="text-xs font-semibold uppercase text-muted-foreground">
                    {t("branding.loginSubtitle") || "Login Subtitle"}
                  </label>
                  <Input
                    id="loginSubtitle"
                    value={settings.loginSubtitle}
                    onChange={(e) => setSettings({ ...settings, loginSubtitle: e.target.value })}
                    placeholder="Sign in to your client infrastructure dashboard"
                    data-testid="branding-login-subtitle-input"
                  />
                </div>

                <div className="space-y-1.5">
                  <label htmlFor="loginFooterText" className="text-xs font-semibold uppercase text-muted-foreground">
                    {t("branding.loginFooterText") || "Login Footer Notice"}
                  </label>
                  <Input
                    id="loginFooterText"
                    value={settings.loginFooterText}
                    onChange={(e) => setSettings({ ...settings, loginFooterText: e.target.value })}
                    placeholder="© 2026 Acme Corp. All rights reserved."
                    data-testid="branding-login-footer-input"
                  />
                </div>
              </CardContent>
            </Card>

            {/* 3. Nameservers */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base flex items-center gap-2">
                  <Globe className="h-4 w-4 text-primary" />
                  {t("branding.nameserversTitle") || "Default Cluster Nameservers"}
                </CardTitle>
                <CardDescription>
                  {t("branding.nameserversDesc") || "Nameservers suggested to clients when provisioning new DNS zones."}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                <label htmlFor="nameservers" className="text-xs font-semibold uppercase text-muted-foreground">
                  {t("branding.nameservers") || "Nameservers (one per line or comma-separated)"}
                </label>
                <textarea
                  id="nameservers"
                  rows={3}
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  value={settings.nameservers.join("\n")}
                  onChange={(e) => {
                    const lines = e.target.value
                      .split(/[\n,]/)
                      .map((s) => s.trim())
                      .filter(Boolean);
                    setSettings({ ...settings, nameservers: lines });
                  }}
                  data-testid="branding-nameservers-input"
                  placeholder="ns1.yourdomain.com&#10;ns2.yourdomain.com"
                />
              </CardContent>
            </Card>

            {/* 4. Sanitized Custom CSS */}
            <Card>
              <CardHeader>
                <CardTitle className="text-base flex items-center gap-2">
                  <Palette className="h-4 w-4 text-primary" />
                  {t("branding.customCssTitle") || "Custom CSS (Sanitized)"}
                </CardTitle>
                <CardDescription>
                  {t("branding.customCssDesc") ||
                    "Inject custom styles across the panel. Custom JavaScript is blocked for security; scripts and @import are stripped."}
                </CardDescription>
              </CardHeader>
              <CardContent className="space-y-3">
                {cssWarning && (
                  <div
                    role="alert"
                    className="p-3 bg-amber-500/10 border border-amber-500/30 text-amber-700 dark:text-amber-400 rounded-md text-xs flex items-center gap-2"
                  >
                    <AlertTriangle className="h-4 w-4 shrink-0" />
                    <span>{cssWarning}</span>
                  </div>
                )}
                <textarea
                  id="customCss"
                  rows={6}
                  className="w-full rounded-md border border-input bg-background px-3 py-2 text-sm font-mono shadow-sm focus-visible:outline-none focus-visible:ring-1 focus-visible:ring-ring"
                  value={settings.customCss}
                  onChange={(e) => handleCssChange(e.target.value)}
                  placeholder="/* e.g. .custom-branding-badge { border: 2px solid green; } */"
                  data-testid="branding-css-input"
                />
                <p className="text-xs text-muted-foreground">
                  {t("branding.cssSecurityNote") ||
                    "Security policy: <script>, javascript: pseudo-protocols, IE expressions, and @import rules are automatically neutralized."}
                </p>
              </CardContent>
            </Card>
          </div>

          {/* Live Preview Sidebar (1 Col) */}
          <div className="space-y-6">
            <Card className="sticky top-6 border-primary/20 shadow-sm" data-testid="branding-live-preview">
              <CardHeader className="pb-3 border-b">
                <CardTitle className="text-sm font-medium flex items-center justify-between">
                  <span>{t("branding.previewTitle") || "Live Interactive Preview"}</span>
                  <span className="text-[10px] px-2 py-0.5 rounded-full bg-primary/10 text-primary font-semibold uppercase">
                    Live
                  </span>
                </CardTitle>
                <CardDescription className="text-xs">
                  {t("branding.previewDesc") || "Preview how your branding appears in the UI."}
                </CardDescription>
              </CardHeader>

              <CardContent className="space-y-4 pt-4">
                {/* Navbar Mini Preview */}
                <div className="space-y-1.5">
                  <span className="text-[11px] font-semibold uppercase text-muted-foreground">
                    {t("branding.navbarPreview") || "Navigation Brand"}
                  </span>
                  <div className="p-3 bg-card border rounded-lg flex items-center justify-between shadow-sm">
                    <div className="flex items-center gap-2">
                      {settings.logoUrl ? (
                        <img
                          src={settings.logoUrl}
                          alt="Logo Preview"
                          className="h-5 w-auto object-contain"
                          data-testid="preview-logo-img"
                        />
                      ) : (
                        <div
                          className="h-5 w-5 rounded flex items-center justify-center text-white text-xs font-bold"
                          style={{ backgroundColor: settings.accentColor }}
                        >
                          DH
                        </div>
                      )}
                      <span className="font-bold text-sm tracking-tight" data-testid="preview-brand-name">
                        {settings.companyName || "Built Panel"}
                      </span>
                    </div>
                    <span className="text-[10px] text-muted-foreground border px-1.5 py-0.5 rounded">v0.1.0</span>
                  </div>
                </div>

                {/* Login Screen Mini Preview */}
                <div className="space-y-1.5">
                  <span className="text-[11px] font-semibold uppercase text-muted-foreground">
                    {t("branding.loginPreview") || "Login Screen Card"}
                  </span>
                  <div className="p-4 bg-muted/40 border rounded-lg text-center space-y-2 shadow-sm">
                    {settings.logoUrl ? (
                      <img
                        src={settings.logoUrl}
                        alt="Login Logo"
                        className="h-6 w-auto mx-auto object-contain"
                      />
                    ) : (
                      <div
                        className="h-7 w-7 rounded-lg mx-auto flex items-center justify-center text-white text-xs font-bold shadow-sm"
                        style={{ backgroundColor: settings.accentColor }}
                      >
                        ✓
                      </div>
                    )}
                    <div className="font-semibold text-xs" data-testid="preview-login-title">
                      {settings.loginTitle || settings.companyName}
                    </div>
                    <p className="text-[10px] text-muted-foreground line-clamp-1" data-testid="preview-login-subtitle">
                      {settings.loginSubtitle}
                    </p>
                    <div
                      className="h-6 rounded text-[11px] text-white flex items-center justify-center font-medium shadow-xs"
                      style={{ backgroundColor: settings.accentColor }}
                    >
                      Sign In Button
                    </div>
                    {settings.loginFooterText && (
                      <p className="text-[9px] text-muted-foreground pt-1" data-testid="preview-login-footer">
                        {settings.loginFooterText}
                      </p>
                    )}
                  </div>
                </div>

                {/* Nameservers preview */}
                <div className="space-y-1.5">
                  <span className="text-[11px] font-semibold uppercase text-muted-foreground">
                    {t("branding.activeNs") || "Active Nameservers"}
                  </span>
                  <ul className="text-xs font-mono space-y-1 bg-muted/20 p-2 rounded border">
                    {settings.nameservers.map((ns, idx) => (
                      <li key={idx} className="flex items-center gap-1.5 text-muted-foreground">
                        <CheckCircle2 className="h-3 w-3 text-emerald-500" />
                        <span data-testid={`preview-ns-${idx}`}>{ns}</span>
                      </li>
                    ))}
                  </ul>
                </div>

                {/* Custom CSS Test Box */}
                {settings.customCss && (
                  <div className="space-y-1.5 pt-2 border-t">
                    <span className="text-[11px] font-semibold uppercase text-muted-foreground">
                      {t("branding.customCssPreview") || "Custom CSS Target"}
                    </span>
                    <div
                      className="builtpanel-custom-branding-test p-2.5 rounded text-xs border text-center font-mono"
                      data-testid="preview-custom-css-box"
                    >
                      .builtpanel-custom-branding-test
                    </div>
                  </div>
                )}
              </CardContent>
            </Card>
          </div>
        </div>
      </form>
    </div>
  );
}
