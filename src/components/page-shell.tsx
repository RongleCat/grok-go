import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

/**
 * Full-height page column. Parent main is `overflow-hidden`;
 * put sticky headers as direct children, scrollable body in {@link PageBody}.
 */
export function PageShell({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex h-full min-h-0 flex-col gap-4", className)}>
      {children}
    </div>
  );
}

/** Scrollable region that fills remaining page height (lists, long forms). */
export function PageBody({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("min-h-0 flex-1 overflow-y-auto", className)}>
      {children}
    </div>
  );
}

/** Non-scrolling header strip (title + actions). */
export function PageHeader({
  children,
  className,
}: {
  children: ReactNode;
  className?: string;
}) {
  return (
    <div className={cn("flex shrink-0 flex-wrap items-start justify-between gap-3", className)}>
      {children}
    </div>
  );
}
