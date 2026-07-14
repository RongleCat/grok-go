import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function formatNumber(n: number) {
  return new Intl.NumberFormat().format(n || 0);
}

/** Format USD without locale-specific "US$" prefix (always `$x.xxxx`). */
export function formatUsd(n: number) {
  const v = Number.isFinite(n) ? n : 0;
  const body = new Intl.NumberFormat("en-US", {
    minimumFractionDigits: 2,
    maximumFractionDigits: 4,
  }).format(v);
  return `$${body}`;
}

/** Compact token count for dense tables (e.g. 327.0k). */
export function formatTokenCompact(n: number): string {
  const v = n || 0;
  if (v >= 1_000_000) return `${(v / 1_000_000).toFixed(v >= 10_000_000 ? 0 : 1)}M`;
  if (v >= 10_000) return `${(v / 1_000).toFixed(v >= 100_000 ? 0 : 1)}k`;
  return formatNumber(v);
}

/**
 * Prompt-cache hit rate when upstream reports cache_read as a subset of input.
 * `inputTokens` is total prompt (includes cache); rate = cache / input.
 */
export function cacheHitRatePercent(inputTokens: number, cacheTokens: number): number | null {
  const input = inputTokens || 0;
  const cache = cacheTokens || 0;
  if (cache <= 0 || input <= 0) return cache > 0 ? 100 : null;
  return Math.min(100, (cache / input) * 100);
}

/** Actual request tokens for display: prompt (incl. cache) + output. Never add cache twice. */
export function totalRequestTokens(
  inputTokens: number,
  outputTokens: number,
  _cacheTokens = 0
): number {
  return (inputTokens || 0) + (outputTokens || 0);
}

export function formatCacheHitRate(inputTokens: number, cacheTokens: number): string {
  const rate = cacheHitRatePercent(inputTokens, cacheTokens);
  if (rate == null) return "—";
  return `${rate.toFixed(rate >= 10 ? 0 : 1)}%`;
}
