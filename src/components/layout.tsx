import { NavLink, Outlet } from "react-router-dom";
import {
  Cable,
  ChartColumn,
  LayoutDashboard,
  Logs,
  Settings,
  Shuffle,
  Users,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useI18n } from "@/i18n/context";
import { useBrand } from "@/components/brand-context";

export function AppLayout() {
  const { t } = useI18n();
  const { brandLogoSrc } = useBrand();
  const items = [
    { to: "/", label: t.nav.overview, icon: LayoutDashboard },
    { to: "/accounts", label: t.nav.accounts, icon: Users },
    { to: "/mapping", label: t.nav.mapping, icon: Shuffle },
    { to: "/integrations", label: t.nav.integrations, icon: Cable },
    { to: "/usage", label: t.nav.usage, icon: ChartColumn },
    { to: "/logs", label: t.nav.logs, icon: Logs },
    { to: "/settings", label: t.nav.settings, icon: Settings },
  ];

  return (
    <div className="flex h-full min-h-0 bg-neutral-50">
      <aside className="flex w-44 shrink-0 flex-col border-r border-neutral-200 bg-white">
        <div className="flex items-center gap-2 border-b border-neutral-200 px-3 py-3">
          <img
            src={brandLogoSrc}
            alt="GrokGo"
            className="h-7 w-7 shrink-0 rounded-md object-cover"
          />
          <div className="min-w-0 truncate text-sm font-semibold">{t.app.name}</div>
        </div>
        <nav className="flex flex-1 flex-col gap-0.5 p-1.5">
          {items.map((item) => {
            const Icon = item.icon;
            return (
              <NavLink
                key={item.to}
                to={item.to}
                end={item.to === "/"}
                className={({ isActive }) =>
                  cn(
                    "flex items-center gap-2 rounded-md px-2.5 py-1.5 text-sm text-neutral-600 transition hover:bg-neutral-100 hover:text-neutral-900",
                    isActive && "bg-neutral-900 text-white hover:bg-neutral-900 hover:text-white"
                  )
                }
              >
                <Icon className="h-4 w-4 shrink-0" />
                <span className="truncate">{item.label}</span>
              </NavLink>
            );
          })}
        </nav>
      </aside>
      <main className="min-h-0 min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto w-full max-w-5xl p-4">
          <Outlet />
        </div>
      </main>
    </div>
  );
}
