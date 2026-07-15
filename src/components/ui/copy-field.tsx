import { Copy } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { cn } from "@/lib/utils";

type CopyFieldProps = {
  label?: string;
  value: string;
  /** Accessible / button title when no text label is shown on the button. */
  copyLabel?: string;
  /** Show "复制" text next to the icon (default: icon-only ghost button). */
  showCopyText?: boolean;
  mono?: boolean;
  className?: string;
  onCopy: () => void;
};

/**
 * Read-only value + copy control using shared Input / Button / Label.
 * Prefer this over ad-hoc gray boxes for copyable endpoints / tokens.
 */
export function CopyField({
  label,
  value,
  copyLabel,
  showCopyText = false,
  mono = true,
  className,
  onCopy,
}: CopyFieldProps) {
  return (
    <div className={cn("space-y-1.5", className)}>
      {label ? <Label>{label}</Label> : null}
      <div className="flex items-center gap-2">
        <Input
          readOnly
          value={value || "—"}
          title={value || undefined}
          className={cn("min-w-0 flex-1", mono && "font-mono text-xs")}
        />
        {showCopyText ? (
          <Button
            type="button"
            size="sm"
            variant="outline"
            className="shrink-0"
            onClick={onCopy}
          >
            <Copy className="h-3.5 w-3.5" />
            {copyLabel}
          </Button>
        ) : (
          <Button
            type="button"
            size="icon"
            variant="outline"
            className="h-9 w-9 shrink-0"
            title={copyLabel}
            onClick={onCopy}
          >
            <Copy className="h-4 w-4" />
          </Button>
        )}
      </div>
    </div>
  );
}
