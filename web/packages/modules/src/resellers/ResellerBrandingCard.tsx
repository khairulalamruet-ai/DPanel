import React, { useState, useEffect } from "react";
import {
  Card,
  CardHeader,
  CardTitle,
  CardDescription,
  CardContent,
  Button,
  Input,
} from "@builtpanel/ui";
import { useTranslation } from "@builtpanel/i18n";
import { Palette, Eye, Check } from "lucide-react";
import { ResellerBranding } from "./types";

interface ResellerBrandingCardProps {
  branding: ResellerBranding;
  onSaveBranding: (branding: ResellerBranding) => Promise<void>;
}

const PRESET_COLORS = [
  "#2563eb", // Royal Blue
  "#7c3aed", // Violet
  "#059669", // Emerald
  "#d97706", // Amber
  "#dc2626", // Crimson
  "#0f172a", // Slate
];

export function ResellerBrandingCard({
  branding,
  onSaveBranding,
}: ResellerBrandingCardProps) {
  const { t } = useTranslation();

  const [brandName, setBrandName] = useState(branding.brand_name || "Apex Cloud Solutions");
  const [logoUrl, setLogoUrl] = useState(branding.logo_url || "https://example.com/logo.png");
  const [primaryColor, setPrimaryColor] = useState(branding.primary_color || "#2563eb");
  const [supportEmail, setSupportEmail] = useState(branding.support_email || "support@apexcloud.net");
  const [loginMsg, setLoginMsg] = useState(
    branding.custom_login_message || "Welcome to your high performance cloud control panel."
  );

  const [isSaving, setIsSaving] = useState(false);
  const [savedSuccess, setSavedSuccess] = useState(false);

  useEffect(() => {
    if (branding) {
      setBrandName(branding.brand_name || "Apex Cloud Solutions");
      setLogoUrl(branding.logo_url || "https://example.com/logo.png");
      setPrimaryColor(branding.primary_color || "#2563eb");
      setSupportEmail(branding.support_email || "support@apexcloud.net");
      setLoginMsg(branding.custom_login_message || "Welcome to your high performance cloud control panel.");
    }
  }, [branding]);

  const handleSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setIsSaving(true);
    setSavedSuccess(false);
    try {
      await onSaveBranding({
        brand_name: brandName.trim(),
        logo_url: logoUrl.trim(),
        primary_color: primaryColor,
        support_email: supportEmail.trim(),
        custom_login_message: loginMsg.trim(),
      });
      setSavedSuccess(true);
      setTimeout(() => setSavedSuccess(false), 3000);
    } finally {
      setIsSaving(false);
    }
  };

  return (
    <div className="grid grid-cols-1 lg:grid-cols-2 gap-6" data-testid="reseller-branding-section">
      {/* Branding Configuration Form */}
      <Card className="border shadow-xs">
        <CardHeader>
          <div className="flex items-center gap-2">
            <div className="p-2 rounded-full bg-primary/10 text-primary">
              <Palette className="h-5 w-5" />
            </div>
            <div>
              <CardTitle className="text-base font-bold">
                {t("reseller.brandingTitle", "White-Label Brand Settings")}
              </CardTitle>
              <CardDescription className="text-xs">
                {t("reseller.brandingSubtitle", "Customize the logo, colors, and support contacts your clients see when logging into their control panel.")}
              </CardDescription>
            </div>
          </div>
        </CardHeader>

        <CardContent>
          <form onSubmit={handleSubmit} className="space-y-4">
            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.brandNameLabel", "Brand / Company Name")}
              </label>
              <Input
                value={brandName}
                onChange={(e) => setBrandName(e.target.value)}
                placeholder="e.g. Apex Cloud Solutions"
                required
                data-testid="input-brand-name"
              />
            </div>

            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.logoUrlLabel", "Brand Logo Image URL")}
              </label>
              <Input
                value={logoUrl}
                onChange={(e) => setLogoUrl(e.target.value)}
                placeholder="https://cdn.yourdomain.com/logo.svg"
                data-testid="input-brand-logo"
              />
            </div>

            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.primaryColorLabel", "Primary Theme Color")}
              </label>
              <div className="flex items-center gap-3">
                <div className="flex items-center gap-1.5">
                  {PRESET_COLORS.map((hex) => (
                    <button
                      key={hex}
                      type="button"
                      onClick={() => setPrimaryColor(hex)}
                      className="h-7 w-7 rounded-full border-2 transition-transform hover:scale-110 flex items-center justify-center"
                      style={{
                        backgroundColor: hex,
                        borderColor: primaryColor === hex ? "#000" : "transparent",
                      }}
                    >
                      {primaryColor === hex && <Check className="h-3.5 w-3.5 text-white" />}
                    </button>
                  ))}
                </div>
                <Input
                  type="text"
                  value={primaryColor}
                  onChange={(e) => setPrimaryColor(e.target.value)}
                  className="w-28 font-mono text-xs uppercase"
                  data-testid="input-brand-color"
                />
              </div>
            </div>

            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.supportEmailLabel", "Support Email Address")}
              </label>
              <Input
                type="email"
                value={supportEmail}
                onChange={(e) => setSupportEmail(e.target.value)}
                placeholder="support@yourbrand.com"
                required
                data-testid="input-brand-support-email"
              />
            </div>

            <div>
              <label className="text-xs font-semibold block mb-1">
                {t("reseller.loginMsgLabel", "Client Welcome Message")}
              </label>
              <Input
                value={loginMsg}
                onChange={(e) => setLoginMsg(e.target.value)}
                placeholder="Welcome to our cloud platform"
                data-testid="input-brand-welcome-msg"
              />
            </div>

            <div className="pt-2 flex items-center justify-between">
              {savedSuccess ? (
                <span className="text-xs font-semibold text-emerald-600 flex items-center gap-1">
                  <Check className="h-4 w-4" />
                  {t("reseller.brandingSaved", "Branding saved successfully!")}
                </span>
              ) : (
                <span />
              )}

              <Button
                type="submit"
                disabled={isSaving}
                data-testid="btn-save-branding"
                className="gap-2"
              >
                {isSaving ? t("common.saving", "Saving...") : t("reseller.saveBrandingBtn", "Save Branding")}
              </Button>
            </div>
          </form>
        </CardContent>
      </Card>

      {/* Live Branding Preview */}
      <Card className="border shadow-xs" data-testid="branding-preview-card">
        <CardHeader>
          <div className="flex items-center gap-2">
            <Eye className="h-4 w-4 text-muted-foreground" />
            <CardTitle className="text-sm font-semibold">
              {t("reseller.livePreviewTitle", "Live Client View Preview")}
            </CardTitle>
          </div>
          <CardDescription className="text-xs">
            {t("reseller.livePreviewSubtitle", "This is how the control panel appears to your sub-tenants.")}
          </CardDescription>
        </CardHeader>

        <CardContent className="space-y-4">
          <div className="border rounded-lg p-4 bg-muted/20 space-y-4">
            {/* Mock Header */}
            <div
              className="p-3 rounded-lg text-white flex items-center justify-between shadow-sm transition-colors"
              style={{ backgroundColor: primaryColor }}
              data-testid="preview-header-bar"
            >
              <div className="flex items-center gap-2">
                <span className="font-bold text-sm tracking-tight" data-testid="preview-brand-name">
                  {brandName}
                </span>
              </div>
              <span className="text-xs opacity-90 font-mono">Client Portal</span>
            </div>

            {/* Mock Welcome Card */}
            <div className="p-4 bg-card border rounded-lg space-y-2">
              <h4 className="font-bold text-sm" data-testid="preview-welcome-heading">
                {brandName} Dashboard
              </h4>
              <p className="text-xs text-muted-foreground" data-testid="preview-welcome-desc">
                {loginMsg}
              </p>
              <div className="pt-2 flex items-center justify-between text-[11px] text-muted-foreground border-t">
                <span>{t("reseller.needHelp", "Need assistance?")}</span>
                <span className="font-medium text-foreground underline" data-testid="preview-support-email">
                  {supportEmail}
                </span>
              </div>
            </div>
          </div>
        </CardContent>
      </Card>
    </div>
  );
}
