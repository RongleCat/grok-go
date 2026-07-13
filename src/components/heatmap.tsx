import { useEffect, useMemo, useRef, useState } from "react";
import type { HeatmapDay } from "@/lib/api";
import { cn, formatNumber, formatUsd } from "@/lib/utils";
import { useI18n } from "@/i18n/context";

type Metric = "requests" | "tokens" | "cost";

type Cell = {
  date: string | null;
  day: HeatmapDay | null;
  value: number;
  level: 0 | 1 | 2 | 3 | 4;
  empty: boolean;
};

const GAP = 3;
const LABEL_COL = 22;
const MONTH_ROW = 16;
const MIN_CELL = 9;
const MAX_CELL = 13;
/** Tooltip panel width — keep in sync with `w-56` (14rem ≈ 224px). */
const TIP_WIDTH = 224;
const TIP_HALF = TIP_WIDTH / 2;
const TIP_EDGE_PAD = 8;
const ARROW_HALF = 5;

const LEVEL_COLORS = [
  "#ebedf0",
  "#9be9a8",
  "#40c463",
  "#30a14e",
  "#216e39",
] as const;

function metricValue(day: HeatmapDay, metric: Metric): number {
  if (metric === "tokens") return day.tokens;
  if (metric === "cost") return day.costUsd;
  return day.requests;
}

function computeLevel(value: number, thresholds: number[]): 0 | 1 | 2 | 3 | 4 {
  if (value <= 0) return 0;
  if (value <= thresholds[0]) return 1;
  if (value <= thresholds[1]) return 2;
  if (value <= thresholds[2]) return 3;
  return 4;
}

function levelThresholds(values: number[]): number[] {
  const positive = values.filter((v) => v > 0).sort((a, b) => a - b);
  if (positive.length === 0) return [1, 2, 3];
  const at = (p: number) => {
    const i = Math.min(positive.length - 1, Math.floor(p * (positive.length - 1)));
    return positive[i];
  };
  const t1 = Math.max(at(0.25), Number.EPSILON);
  const t2 = Math.max(at(0.5), t1);
  const t3 = Math.max(at(0.75), t2);
  return [t1, t2, t3];
}

function parseYmd(date: string): Date {
  const [y, m, d] = date.split("-").map(Number);
  return new Date(y, m - 1, d);
}

function formatYmd(d: Date): string {
  const y = d.getFullYear();
  const m = String(d.getMonth() + 1).padStart(2, "0");
  const day = String(d.getDate()).padStart(2, "0");
  return `${y}-${m}-${day}`;
}

function weekdaySun0(d: Date): number {
  return d.getDay();
}

function buildGrid(days: HeatmapDay[], metric: Metric): { weeks: Cell[][]; monthLabels: { week: number; label: string }[] } {
  if (days.length === 0) return { weeks: [], monthLabels: [] };

  const byDate = new Map(days.map((d) => [d.date, d]));
  const first = parseYmd(days[0].date);
  const last = parseYmd(days[days.length - 1].date);

  const start = new Date(first);
  start.setDate(start.getDate() - weekdaySun0(start));

  const end = new Date(last);
  end.setDate(end.getDate() + (6 - weekdaySun0(end)));

  const raw: { date: string | null; day: HeatmapDay | null; value: number; empty: boolean }[] = [];
  for (let cur = new Date(start); cur <= end; cur.setDate(cur.getDate() + 1)) {
    const key = formatYmd(cur);
    const inRange = cur >= first && cur <= last;
    const day = byDate.get(key) ?? null;
    raw.push({
      date: inRange ? key : null,
      day: inRange ? day : null,
      value: day ? metricValue(day, metric) : 0,
      empty: !inRange,
    });
  }

  const thresholds = levelThresholds(raw.filter((c) => !c.empty).map((c) => c.value));
  const cells: Cell[] = raw.map((c) => ({
    ...c,
    level: c.empty ? 0 : computeLevel(c.value, thresholds),
  }));

  const weeks: Cell[][] = [];
  for (let i = 0; i < cells.length; i += 7) {
    weeks.push(cells.slice(i, i + 7));
  }

  const monthLabels: { week: number; label: string }[] = [];
  let lastMonth = -1;
  weeks.forEach((week, wi) => {
    const sample = week.find((c) => c.date)?.date;
    if (!sample) return;
    const m = parseYmd(sample).getMonth();
    if (m !== lastMonth) {
      lastMonth = m;
      monthLabels.push({ week: wi, label: sample.slice(5, 7) });
    }
  });

  return { weeks, monthLabels };
}

export function Heatmap({
  days,
  metric = "requests",
}: {
  days: HeatmapDay[];
  metric?: Metric;
}) {
  const { t, locale } = useI18n();
  const containerRef = useRef<HTMLDivElement>(null);
  const [containerWidth, setContainerWidth] = useState(0);
  const [selected, setSelected] = useState<{
    cell: Cell;
    x: number;
    y: number;
  } | null>(null);

  const { weeks, monthLabels } = useMemo(() => buildGrid(days, metric), [days, metric]);

  useEffect(() => {
    const el = containerRef.current;
    if (!el) return;
    const ro = new ResizeObserver((entries) => {
      const w = entries[0]?.contentRect.width ?? 0;
      setContainerWidth(w);
    });
    ro.observe(el);
    setContainerWidth(el.clientWidth);
    return () => ro.disconnect();
  }, []);

  // Close tooltip on outside / blank clicks, but do NOT swallow clicks on other
  // heatmap cells — those should switch the tip in one press (no full-screen
  // overlay that eats the first click).
  useEffect(() => {
    if (!selected?.cell.date) return;
    const onPointerDown = (e: PointerEvent) => {
      const target = e.target as HTMLElement | null;
      if (!target) return;
      // Clicking another enabled day cell: leave it for the button's onClick.
      const cellBtn = target.closest("button[role='gridcell']") as HTMLButtonElement | null;
      if (cellBtn && !cellBtn.disabled && containerRef.current?.contains(cellBtn)) {
        return;
      }
      // Keep tip when interacting with the tip itself.
      if (target.closest("[data-heatmap-tip]")) return;
      setSelected(null);
    };
    document.addEventListener("pointerdown", onPointerDown, true);
    return () => document.removeEventListener("pointerdown", onPointerDown, true);
  }, [selected?.cell.date]);

  // Stretch cells to fill available width (no large empty right margin).
  const cell = useMemo(() => {
    if (weeks.length === 0 || containerWidth <= 0) return MIN_CELL;
    const available = containerWidth - LABEL_COL;
    const size = Math.floor((available - (weeks.length - 1) * GAP) / weeks.length);
    return Math.max(MIN_CELL, Math.min(MAX_CELL, size));
  }, [containerWidth, weeks.length]);

  const dayLabels = useMemo(() => {
    if (locale === "zh-CN") return ["日", "一", "二", "三", "四", "五", "六"];
    return ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
  }, [locale]);

  const monthName = (mm: string) => {
    const idx = Number(mm) - 1;
    if (locale === "zh-CN") return `${idx + 1}月`;
    return ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"][idx] ?? mm;
  };

  const formatDateLabel = (date: string) => {
    const d = parseYmd(date);
    return d.toLocaleDateString(locale === "zh-CN" ? "zh-CN" : "en-US", {
      year: "numeric",
      month: "long",
      day: "numeric",
      weekday: "short",
    });
  };

  if (weeks.length === 0) {
    return <div className="text-sm text-neutral-500">{t.heatmap.noData}</div>;
  }

  const graphWidth = weeks.length * (cell + GAP) - GAP;
  const graphHeight = 7 * (cell + GAP) - GAP;

  // Tooltip placement: clamp panel in container, keep arrow aimed at cell center.
  let tipLeft = 0;
  let tipTop = 0;
  let arrowLeft = TIP_HALF;
  if (selected?.cell.date) {
    const containerW = containerRef.current?.clientWidth ?? 300;
    const idealCenter = selected.x;
    const minCenter = TIP_HALF + TIP_EDGE_PAD;
    const maxCenter = Math.max(minCenter, containerW - TIP_HALF - TIP_EDGE_PAD);
    const tipCenter = Math.min(maxCenter, Math.max(minCenter, idealCenter));
    tipLeft = tipCenter - TIP_HALF;
    tipTop = Math.max(8, selected.y - 8);
    // Nudge 1px left — optical alignment with the small heatmap cell.
    arrowLeft = Math.min(
      TIP_WIDTH - ARROW_HALF - 6,
      Math.max(ARROW_HALF + 6, idealCenter - tipLeft - 1)
    );
  }

  const blockWidth = LABEL_COL + graphWidth;

  return (
    <div ref={containerRef} className="relative w-full select-none">
      <div className="flex w-full flex-col items-center">
        <div style={{ width: blockWidth }}>
        <div className="relative" style={{ height: MONTH_ROW, marginLeft: LABEL_COL }}>
          {monthLabels.map(({ week, label }) => (
            <span
              key={`${week}-${label}`}
              className="absolute text-[10px] leading-none text-neutral-500"
              style={{ left: week * (cell + GAP), top: 1 }}
            >
              {monthName(label)}
            </span>
          ))}
        </div>

        <div className="flex items-start">
          <div
            className="flex shrink-0 flex-col text-[10px] leading-none text-neutral-500"
            style={{ width: LABEL_COL, height: graphHeight, gap: GAP }}
          >
            {dayLabels.map((label, i) => (
              <div
                key={label + i}
                className="flex items-center justify-end pr-1"
                style={{ height: cell, visibility: i % 2 === 1 ? "visible" : "hidden" }}
              >
                {label}
              </div>
            ))}
          </div>

          <div
            className="grid"
            style={{
              gridTemplateColumns: `repeat(${weeks.length}, ${cell}px)`,
              gridTemplateRows: `repeat(7, ${cell}px)`,
              columnGap: GAP,
              rowGap: GAP,
              width: graphWidth,
              height: graphHeight,
            }}
            role="grid"
            aria-label={t.heatmap.aria}
          >
            {weeks.map((week, wi) =>
              week.map((cellItem, di) => {
                const isSelected = selected?.cell.date === cellItem.date && !!cellItem.date;
                return (
                  <button
                    key={`${wi}-${di}`}
                    type="button"
                    role="gridcell"
                    disabled={cellItem.empty || !cellItem.date}
                    data-level={cellItem.level}
                    data-date={cellItem.date ?? undefined}
                    aria-label={cellItem.date ? `${cellItem.date}: ${cellItem.value}` : undefined}
                    className={cn(
                      "rounded-[2px] border border-black/[0.06] p-0 outline-none transition",
                      !cellItem.empty && cellItem.date && "cursor-pointer hover:ring-1 hover:ring-neutral-400",
                      isSelected && "ring-2 ring-neutral-900 ring-offset-1",
                      cellItem.empty && "pointer-events-none opacity-0"
                    )}
                    style={{
                      gridColumn: wi + 1,
                      gridRow: di + 1,
                      width: cell,
                      height: cell,
                      backgroundColor: cellItem.empty ? "transparent" : LEVEL_COLORS[cellItem.level],
                    }}
                    onClick={(e) => {
                      if (!cellItem.date || cellItem.empty) return;
                      const rect = (e.currentTarget as HTMLElement).getBoundingClientRect();
                      const parent = containerRef.current?.getBoundingClientRect();
                      if (!parent) return;
                      setSelected({
                        cell: cellItem,
                        x: rect.left - parent.left + rect.width / 2,
                        y: rect.top - parent.top,
                      });
                    }}
                  />
                );
              })
            )}
          </div>
        </div>
        </div>

        <div className="mt-2 flex items-center justify-center gap-1 text-[10px] text-neutral-500">
          <span className="mr-1">{t.heatmap.less}</span>
          {LEVEL_COLORS.map((color, level) => (
            <span
              key={level}
              className="inline-block rounded-[2px] border border-black/[0.06]"
              style={{ width: cell, height: cell, backgroundColor: color }}
            />
          ))}
          <span className="ml-1">{t.heatmap.more}</span>
        </div>
      </div>

      {selected?.cell.date ? (
          <div
            data-heatmap-tip
            className="absolute z-30 rounded-md border border-neutral-200 bg-neutral-900 px-3 py-2 text-xs text-white shadow-lg"
            style={{
              left: tipLeft,
              top: tipTop,
              width: TIP_WIDTH,
              transform: "translateY(-100%)",
            }}
            role="dialog"
          >
            <div className="font-medium">{formatDateLabel(selected.cell.date)}</div>
            <div className="mt-1.5 space-y-1 text-neutral-300">
              <div className="flex justify-between gap-3">
                <span>{t.heatmap.requests}</span>
                <span className="text-white">{formatNumber(selected.cell.day?.requests ?? 0)}</span>
              </div>
              <div className="flex justify-between gap-3">
                <span>{t.heatmap.tokens}</span>
                <span className="text-white">{formatNumber(selected.cell.day?.tokens ?? 0)}</span>
              </div>
              <div className="flex justify-between gap-3">
                <span>{t.heatmap.cost}</span>
                <span className="text-white">{formatUsd(selected.cell.day?.costUsd ?? 0)}</span>
              </div>
            </div>
            <div
              className="absolute top-full h-0 w-0 border-x-[5px] border-t-[5px] border-x-transparent border-t-neutral-900"
              style={{
                left: arrowLeft,
                transform: "translateX(-50%)",
              }}
              aria-hidden
            />
          </div>
      ) : null}
    </div>
  );
}
