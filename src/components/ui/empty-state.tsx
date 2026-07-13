import type { LucideIcon } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/utils";

export type EmptyStateProps = {
  icon: LucideIcon;
  title: string;
  description?: string;
  /** Optional CTA under the copy (button, link, …). */
  action?: ReactNode;
  /** Fill parent flex area and center content (default true). */
  fill?: boolean;
  className?: string;
  /** Compact padding for narrow cards. */
  size?: "default" | "sm";
};

/**
 * Centered empty-state: soft icon well + title + optional description/action.
 * Use inside a list panel that already has a defined height (`min-h-0 flex-1`).
 */
export function EmptyState({
  icon: Icon,
  title,
  description,
  action,
  fill = true,
  className,
  size = "default",
}: EmptyStateProps) {
  return (
    <div
      className={cn(
        "flex flex-col items-center justify-center text-center",
        fill && "min-h-[12rem] flex-1",
        size === "default" ? "gap-3 px-6 py-12" : "gap-2 px-4 py-8",
        className
      )}
      role="status"
    >
      <div
        className={cn(
          "flex items-center justify-center rounded-2xl border border-neutral-200/80 bg-neutral-50 text-neutral-400 shadow-sm",
          size === "default" ? "h-14 w-14" : "h-11 w-11"
        )}
      >
        <Icon
          className={cn(size === "default" ? "h-6 w-6" : "h-5 w-5")}
          strokeWidth={1.5}
          aria-hidden
        />
      </div>
      <div className="max-w-sm space-y-1">
        <div
          className={cn(
            "font-medium text-neutral-800",
            size === "default" ? "text-sm" : "text-xs"
          )}
        >
          {title}
        </div>
        {description ? (
          <p
            className={cn(
              "leading-relaxed text-neutral-500",
              size === "default" ? "text-sm" : "text-xs"
            )}
          >
            {description}
          </p>
        ) : null}
      </div>
      {action ? <div className="mt-1">{action}</div> : null}
    </div>
  );
}
