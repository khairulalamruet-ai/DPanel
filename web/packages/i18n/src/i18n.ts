import i18n from "i18next";
import { initReactI18next } from "react-i18next";
import en from "./locales/en.json";
import bn from "./locales/bn.json";

const savedLng =
  typeof window !== "undefined" && typeof window.localStorage !== "undefined"
    ? window.localStorage?.getItem?.("builtpanel-locale") || "en"
    : "en";

i18n.use(initReactI18next).init({
  resources: {
    en: { translation: en },
    bn: { translation: bn },
  },
  lng: savedLng,
  fallbackLng: "en",
  interpolation: {
    escapeValue: false,
  },
});

export default i18n;
