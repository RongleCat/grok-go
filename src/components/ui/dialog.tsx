import type { ReactNode } from "react";
import { Button } from "@/components/ui/button";
import { cn } from "@/lib/utils";

type DialogProps = {
  open: boolean;
  title: string;
  description?: string;
  children?: ReactNode;
  className?: string;
  onClose?: () => void;
};

export function Dialog({ open, title, description, children, className, onClose }: DialogProps) {
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
          "relative z-[1] flex max-h-[min(85vh,720px)] w-full max-w-md flex-col rounded-xl border border-neutral-200 bg-white p-4 shadow-xl",
          className
        )}
      >
        <div className="shrink-0 space-y-1">
          <h2 className="text-base font-semibold tracking-tight">{title}</h2>
          {description ? <p className="text-sm text-neutral-500">{description}</p> : null}
        </div>
        {children ? <div className="mt-4 min-h-0 flex-1 overflow-y-auto">{children}</div> : null}
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
    <Dialog open={open} title={title} description={description} onClose={busy ? undefined : onCancel}>
      <div className="flex flex-wrap justify-end gap-2">
        <Button type="button" variant="outline" disabled={busy} onClick={onCancel}>
          {cancelLabel}
        </Button>
        {secondaryLabel && onSecondary ? (
          <Button type="button" variant="secondary" disabled={busy} onClick={onSecondary}>
            {secondaryLabel}
          </Button>
        ) : null}
        <Button type="button" disabled={busy} onClick={onConfirm}>
          {confirmLabel}
        </Button>
      </div>
    </Dialog>
  );
}
