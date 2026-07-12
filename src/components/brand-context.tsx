import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { api, type AppIconStyle } from "@/lib/api";
// Full-bleed brand marks for sidebar / loading — no dock transparent margin.
// Dock/taskbar icons stay in `app-icons/` + `src-tauri/icons/variants/`.
import brandLogoDark from "@/assets/brand-logo/dark.png";
import brandLogoLight from "@/assets/brand-logo/light.png";

export function brandLogoSrc(style: AppIconStyle): string {
  return style === "light" ? brandLogoLight : brandLogoDark;
}

type BrandContextValue = {
  appIcon: AppIconStyle;
  brandLogoSrc: string;
  setAppIcon: (style: AppIconStyle) => void;
  refreshAppIcon: () => Promise<void>;
};

const BrandContext = createContext<BrandContextValue | null>(null);

export function BrandProvider({ children }: { children: ReactNode }) {
  const [appIcon, setAppIconState] = useState<AppIconStyle>("dark");

  const refreshAppIcon = useCallback(async () => {
    try {
      const cfg = await api.getConfig();
      setAppIconState(cfg.appIcon ?? "dark");
    } catch {
      // Keep previous style when config is unavailable (e.g. non-Tauri preview).
    }
  }, []);

  useEffect(() => {
    refreshAppIcon();
  }, [refreshAppIcon]);

  const setAppIcon = useCallback((style: AppIconStyle) => {
    setAppIconState(style);
  }, []);

  const value = useMemo(
    () => ({
      appIcon,
      brandLogoSrc: brandLogoSrc(appIcon),
      setAppIcon,
      refreshAppIcon,
    }),
    [appIcon, setAppIcon, refreshAppIcon]
  );

  return <BrandContext.Provider value={value}>{children}</BrandContext.Provider>;
}

export function useBrand() {
  const ctx = useContext(BrandContext);
  if (!ctx) {
    throw new Error("useBrand must be used within BrandProvider");
  }
  return ctx;
}
