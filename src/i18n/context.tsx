import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import {
  getDictionary,
  loadLocalePreference,
  resolveLocale,
  saveLocalePreference,
  type Dictionary,
  type LocalePreference,
  type ResolvedLocale,
} from "./index";

type I18nContextValue = {
  preference: LocalePreference;
  locale: ResolvedLocale;
  t: Dictionary;
  setPreference: (pref: LocalePreference) => void;
};

const I18nContext = createContext<I18nContextValue | null>(null);

export function I18nProvider({ children }: { children: ReactNode }) {
  const [preference, setPreferenceState] = useState<LocalePreference>(() => loadLocalePreference());

  const locale = useMemo(() => resolveLocale(preference), [preference]);
  const t = useMemo(() => getDictionary(locale), [locale]);

  const setPreference = useCallback((pref: LocalePreference) => {
    saveLocalePreference(pref);
    setPreferenceState(pref);
  }, []);

  const value = useMemo(
    () => ({ preference, locale, t, setPreference }),
    [preference, locale, t, setPreference]
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n() {
  const ctx = useContext(I18nContext);
  if (!ctx) {
    throw new Error("useI18n must be used within I18nProvider");
  }
  return ctx;
}
