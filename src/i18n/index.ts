import { en } from "./locales/en";
import { zhCN, type Dictionary } from "./locales/zh-CN";

export type LocalePreference = "system" | "zh-CN" | "en";
export type ResolvedLocale = "zh-CN" | "en";

export type { Dictionary };

const STORAGE_KEY = "grok-go.locale";

const dictionaries: Record<ResolvedLocale, Dictionary> = {
  "zh-CN": zhCN,
  en,
};

/** Detect OS/browser language. Defaults to Simplified Chinese. */
export function detectSystemLocale(): ResolvedLocale {
  if (typeof navigator === "undefined") return "zh-CN";
  const langs = [navigator.language, ...(navigator.languages || [])]
    .filter(Boolean)
    .map((l) => l.toLowerCase());
  for (const lang of langs) {
    if (lang.startsWith("zh")) return "zh-CN";
    if (lang.startsWith("en")) return "en";
  }
  // Unrecognized system language → Simplified Chinese by default
  return "zh-CN";
}

export function resolveLocale(pref: LocalePreference): ResolvedLocale {
  if (pref === "system") return detectSystemLocale();
  return pref;
}

export function getDictionary(locale: ResolvedLocale): Dictionary {
  return dictionaries[locale] ?? zhCN;
}

export function loadLocalePreference(): LocalePreference {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    if (raw === "system" || raw === "zh-CN" || raw === "en") return raw;
  } catch {
    /* ignore */
  }
  // Default: Simplified Chinese. Users can switch to “Follow system” in Settings.
  return "zh-CN";
}

export function saveLocalePreference(pref: LocalePreference) {
  try {
    localStorage.setItem(STORAGE_KEY, pref);
  } catch {
    /* ignore */
  }
}
