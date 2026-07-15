import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type DialogProps = {
  open: boolean;
  title: string;
  description?: string;
  children?: ReactNode;
  /** Sticky bottom actions (e.g. Cancel / Done). */
  footer?: ReactNode;
  className?: string;
  onClose?: () => void;
};

export function Dialog({
  open,
  title,
  description,
  children,
  footer,
  className,
  onClose,
}: DialogProps) {
  if (!open) return null;
  return (
    <div className="fixed inset-0 z-[90] flex items-center justify-center p-4">
      <button
        type="button"
        aria-label="Close"
        className="absolute inset-0 bg-black/40"
        onClick={onClose}
      />
      <div
        role="dialog"
        aria-modal="true"
        className={cn(
          "relative z-[1] flex max-h-[min(85vh,720px)] w-full max-w-md flex-col overflow-hidden rounded-xl border border-neutral-200 bg-white shadow-xl",
          className
        )}
      >
        <div className="shrink-0 space-y-1 border-b border-neutral-100 px-5 py-4">
          <h2 className="text-base font-semibold tracking-tight text-neutral-900">{title}</h2>
          {description ? (
            <p className="truncate text-sm text-neutral-500" title={description}>
              {description}
            </p>
          ) : null}
        </div>
        {children ? (
          <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">{children}</div>
        ) : null}
        {footer ? (
          <div className="shrink-0 border-t border-neutral-100 px-5 py-3">{footer}</div>
        ) : null}
      </div>
    </div>
  );
}

type ConfirmDialogProps = {
  open: boolean;
  title: string;
  description?: string;
  cancelLabel: string;
  confirmLabel: string;
  secondaryLabel?: string;
  onCancel: () => void;
  onConfirm: () => void;
  onSecondary?: () => void;
  busy?: boolean;
};

/** Primary = confirm, optional secondary action, cancel closes. */
export function ConfirmDialog({
  open,
  title,
  description,
  cancelLabel,
  confirmLabel,
  secondaryLabel,
  onCancel,
  onConfirm,
  onSecondary,
  busy,
}: ConfirmDialogProps) {
  return (
    <Dialog
      open={open}
      title={title}
      description={description}
      onClose={busy ? undefined : onCancel}
      footer={
        <div className="flex flex-wrap justify-end gap-2">
          <Button type="button" variant="outline" disabled={busy} onClick={onCancel}>
            {cancelLabel}
          </Button>
          {secondaryLabel && onSecondary ? (
            <Button type="button" variant="secondary" disabled={busy} onClick={onSecondary}>
              {secondaryLabel}
            </Button>
          ) : null}
          <Button type="button" variant="destructive" disabled={busy} onClick={onConfirm}>
            {confirmLabel}
          </Button>
        </div>
      }
    />
  );
}
