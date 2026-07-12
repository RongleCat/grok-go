import { useI18n } from "@/i18n/context";
import { useBrand } from "@/components/brand-context";

/** Full-page loading: centered brand logo + loading copy (screenshot-friendly). */
export function PageLoading({ label }: { label?: string }) {
  const { t } = useI18n();
  const { brandLogoSrc } = useBrand();

  return (
    <div className="flex h-full min-h-[280px] flex-1 flex-col items-center justify-center gap-3">
      <img
        src={brandLogoSrc}
        alt=""
        className="h-14 w-14 rounded-2xl border border-neutral-200 object-cover shadow-sm"
      />
      <div className="text-sm text-neutral-500">{label ?? t.common.loading}</div>
    </div>
  );
}
