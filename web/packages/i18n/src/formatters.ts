import i18n from "./i18n";

const BENGALI_DIGITS = ["০", "১", "২", "৩", "৪", "৫", "৬", "৭", "৮", "৯"];

/**
 * Convert ASCII numbers/digits to Bengali digits
 */
export function toBengaliDigits(input: number | string): string {
  return String(input).replace(/[0-9]/g, (digit) => BENGALI_DIGITS[Number(digit)] || digit);
}

/**
 * Get current normalized locale ('bn-BD' or 'en-US')
 */
export function getCurrentLocale(locale?: string): string {
  const current = locale || i18n.language || "en";
  return current.startsWith("bn") ? "bn-BD" : "en-US";
}

/**
 * Format a number according to the active or specified locale
 */
export function formatNumber(
  value: number,
  options?: Intl.NumberFormatOptions,
  locale?: string
): string {
  const targetLocale = getCurrentLocale(locale);
  try {
    return new Intl.NumberFormat(targetLocale, options).format(value);
  } catch {
    return targetLocale.startsWith("bn") ? toBengaliDigits(value) : String(value);
  }
}

/**
 * Format currency amount (defaults to BDT)
 */
export function formatCurrency(
  value: number,
  currency: string = "BDT",
  locale?: string
): string {
  return formatNumber(
    value,
    {
      style: "currency",
      currency,
      maximumFractionDigits: 2,
    },
    locale
  );
}

/**
 * Format byte units (B, KB, MB, GB, TB) with localized numbers and units
 */
export function formatBytes(bytes: number, locale?: string): string {
  if (bytes === 0) {
    const isBn = getCurrentLocale(locale).startsWith("bn");
    return isBn ? "০ বাইট" : "0 Bytes";
  }

  const k = 1024;
  const sizesEn = ["Bytes", "KB", "MB", "GB", "TB", "PB"];
  const sizesBn = ["বাইট", "কেবি", "এমবি", "জিবি", "টিবি", "পিবি"];

  const i = Math.floor(Math.log(bytes) / Math.log(k));
  const val = Number((bytes / Math.pow(k, i)).toFixed(2));
  const isBn = getCurrentLocale(locale).startsWith("bn");

  const formattedVal = formatNumber(val, { maximumFractionDigits: 2 }, locale);
  const unit = isBn ? (sizesBn[i] || "বাইট") : (sizesEn[i] || "Bytes");

  return `${formattedVal} ${unit}`;
}

/**
 * Format Date according to locale
 */
export function formatDate(
  date: Date | string | number,
  options?: Intl.DateTimeFormatOptions,
  locale?: string
): string {
  const d = typeof date === "object" ? date : new Date(date);
  if (isNaN(d.getTime())) return "";

  const targetLocale = getCurrentLocale(locale);
  const defaultOptions: Intl.DateTimeFormatOptions = options || {
    year: "numeric",
    month: "long",
    day: "numeric",
  };

  try {
    return new Intl.DateTimeFormat(targetLocale, defaultOptions).format(d);
  } catch {
    return d.toISOString();
  }
}

/**
 * Format relative time (e.g. 5 minutes ago / ৫ মিনিট আগে)
 */
export function formatRelativeTime(
  date: Date | string | number,
  locale?: string
): string {
  const d = typeof date === "object" ? date : new Date(date);
  if (isNaN(d.getTime())) return "";

  const now = new Date();
  const elapsedSec = Math.round((d.getTime() - now.getTime()) / 1000);
  const targetLocale = getCurrentLocale(locale);

  const units: { unit: Intl.RelativeTimeFormatUnit; seconds: number }[] = [
    { unit: "year", seconds: 31536000 },
    { unit: "month", seconds: 2592000 },
    { unit: "week", seconds: 604800 },
    { unit: "day", seconds: 86400 },
    { unit: "hour", seconds: 3600 },
    { unit: "minute", seconds: 60 },
    { unit: "second", seconds: 1 },
  ];

  try {
    const rtf = new Intl.RelativeTimeFormat(targetLocale, { numeric: "auto" });
    for (const { unit, seconds } of units) {
      if (Math.abs(elapsedSec) >= seconds || unit === "second") {
        const count = Math.round(elapsedSec / seconds);
        return rtf.format(count, unit);
      }
    }
  } catch {
    return formatDate(d, undefined, locale);
  }

  return formatDate(d, undefined, locale);
}

/**
 * React Hook providing localized formatters bound to the current language
 */
export function useFormatters() {
  const currentLocale = i18n.language;

  return {
    locale: currentLocale,
    formatNumber: (val: number, opts?: Intl.NumberFormatOptions) =>
      formatNumber(val, opts, currentLocale),
    formatCurrency: (val: number, currency?: string) =>
      formatCurrency(val, currency, currentLocale),
    formatBytes: (bytes: number) => formatBytes(bytes, currentLocale),
    formatDate: (date: Date | string | number, opts?: Intl.DateTimeFormatOptions) =>
      formatDate(date, opts, currentLocale),
    formatRelativeTime: (date: Date | string | number) =>
      formatRelativeTime(date, currentLocale),
    toBengaliDigits,
  };
}
