import * as React from "react";
import { createPortal } from "react-dom";
import { Check, ChevronDown } from "lucide-react";
import { cn } from "@/lib/utils";

export type SelectOption = {
  value: string;
  label: React.ReactNode;
  disabled?: boolean;
};

type SelectSize = "default" | "sm";

export interface SelectProps {
  value?: string;
  defaultValue?: string;
  /** Compatible with native `<select onChange>` — `e.target.value`. */
  onChange?: (event: { target: { value: string; name?: string } }) => void;
  onValueChange?: (value: string) => void;
  /** Prefer explicit options; `<option>` children are also parsed. */
  options?: SelectOption[];
  children?: React.ReactNode;
  placeholder?: string;
  disabled?: boolean;
  className?: string;
  /** Trigger height: default h-9, sm h-8 */
  size?: SelectSize;
  title?: string;
  id?: string;
  name?: string;
  /** Align dropdown panel */
  align?: "start" | "end";
}

function parseOptionChildren(children: React.ReactNode): SelectOption[] {
  const out: SelectOption[] = [];
  React.Children.forEach(children, (child) => {
    if (!React.isValidElement(child)) return;
    const type = child.type;
    const isOption =
      type === "option" ||
      (typeof type === "string" && type.toLowerCase() === "option");
    if (!isOption) return;
    const props = child.props as {
      value?: string | number;
      disabled?: boolean;
      children?: React.ReactNode;
    };
    out.push({
      value: props.value == null ? "" : String(props.value),
      label: props.children,
      disabled: Boolean(props.disabled),
    });
  });
  return out;
}

function labelText(label: React.ReactNode): string {
  if (label == null || label === false) return "";
  if (typeof label === "string" || typeof label === "number") return String(label);
  return "";
}

export const Select = React.forwardRef<HTMLButtonElement, SelectProps>(
  (
    {
      value: valueProp,
      defaultValue = "",
      onChange,
      onValueChange,
      options: optionsProp,
      children,
      placeholder = "Select…",
      disabled,
      className,
      size = "default",
      title,
      id,
      name,
      align = "start",
    },
    ref
  ) => {
    const options = React.useMemo(() => {
      if (optionsProp && optionsProp.length > 0) return optionsProp;
      return parseOptionChildren(children);
    }, [optionsProp, children]);

    const isControlled = valueProp !== undefined;
    const [uncontrolled, setUncontrolled] = React.useState(defaultValue);
    const value = isControlled ? (valueProp ?? "") : uncontrolled;

    const [open, setOpen] = React.useState(false);
    const [highlight, setHighlight] = React.useState(-1);
    const rootRef = React.useRef<HTMLDivElement>(null);
    const triggerRef = React.useRef<HTMLButtonElement | null>(null);
    const listRef = React.useRef<HTMLDivElement>(null);
    /** Only mount portal after we have fixed coords — avoids first-frame body overflow / scrollbar flash. */
    const [panelStyle, setPanelStyle] = React.useState<React.CSSProperties | null>(null);

    const setTriggerRef = React.useCallback(
      (node: HTMLButtonElement | null) => {
        triggerRef.current = node;
        if (typeof ref === "function") ref(node);
        else if (ref) ref.current = node;
      },
      [ref]
    );

    const selected = options.find((o) => o.value === value);
    const display = selected ? selected.label : placeholder;
    const showPlaceholder = !selected;

    const enabledIndexes = React.useMemo(
      () =>
        options
          .map((o, i) => (o.disabled ? -1 : i))
          .filter((i) => i >= 0),
      [options]
    );

    const close = React.useCallback(() => {
      setOpen(false);
      setPanelStyle(null);
    }, []);

    const commit = React.useCallback(
      (next: string) => {
        if (!isControlled) setUncontrolled(next);
        onValueChange?.(next);
        onChange?.({ target: { value: next, name } });
        close();
        // Return focus to trigger for keyboard users
        requestAnimationFrame(() => triggerRef.current?.focus());
      },
      [close, isControlled, name, onChange, onValueChange]
    );

    const measurePanelStyle = React.useCallback((): React.CSSProperties | null => {
      const el = triggerRef.current;
      if (!el) return null;
      const rect = el.getBoundingClientRect();
      const viewportH = window.innerHeight;
      const viewportW = window.innerWidth;
      const gap = 4;
      const maxPanel = 280;
      const spaceBelow = Math.max(0, viewportH - rect.bottom - gap);
      const spaceAbove = Math.max(0, rect.top - gap);
      const openUp = spaceBelow < 160 && spaceAbove > spaceBelow;
      // Never force document growth: clamp height to available viewport space.
      const available = openUp ? spaceAbove : spaceBelow;
      const maxHeight = Math.max(80, Math.min(maxPanel, available || maxPanel));

      const width = Math.max(rect.width, 140);
      let left = rect.left;
      if (align === "end") {
        left = rect.right - width;
      }
      left = Math.min(Math.max(8, left), Math.max(8, viewportW - width - 8));

      return {
        position: "fixed",
        zIndex: 80,
        left,
        width,
        maxHeight,
        overflowY: "auto",
        overscrollBehavior: "contain",
        ...(openUp
          ? { bottom: viewportH - rect.top + gap }
          : { top: rect.bottom + gap }),
      };
    }, [align]);

    const updatePanelPosition = React.useCallback(() => {
      const next = measurePanelStyle();
      if (next) setPanelStyle(next);
    }, [measurePanelStyle]);

    const openSelect = React.useCallback(() => {
      // Measure synchronously in the user event so the portal never mounts
      // without fixed positioning (that flash caused page scrollbar flicker).
      const style = measurePanelStyle();
      if (style) setPanelStyle(style);
      setOpen(true);
    }, [measurePanelStyle]);

    React.useEffect(() => {
      if (!open) return;
      updatePanelPosition();
      const onScroll = () => updatePanelPosition();
      const onResize = () => updatePanelPosition();
      window.addEventListener("resize", onResize);
      // Capture scroll on any scrollable ancestor
      window.addEventListener("scroll", onScroll, true);
      return () => {
        window.removeEventListener("resize", onResize);
        window.removeEventListener("scroll", onScroll, true);
      };
    }, [open, updatePanelPosition, options.length]);

    React.useEffect(() => {
      if (!open) return;
      const idx = options.findIndex((o) => o.value === value && !o.disabled);
      setHighlight(idx >= 0 ? idx : enabledIndexes[0] ?? -1);
    }, [open, value, options, enabledIndexes]);

    React.useEffect(() => {
      if (!open) return;
      const onDoc = (e: MouseEvent) => {
        const t = e.target as Node;
        if (rootRef.current?.contains(t)) return;
        if (listRef.current?.contains(t)) return;
        close();
      };
      const onKey = (e: KeyboardEvent) => {
        if (e.key === "Escape") {
          e.preventDefault();
          close();
          triggerRef.current?.focus();
        }
      };
      document.addEventListener("mousedown", onDoc);
      document.addEventListener("keydown", onKey);
      return () => {
        document.removeEventListener("mousedown", onDoc);
        document.removeEventListener("keydown", onKey);
      };
    }, [close, open]);

    React.useEffect(() => {
      if (!open || highlight < 0) return;
      const list = listRef.current;
      const item = list?.querySelector<HTMLElement>(
        `[data-select-index="${highlight}"]`
      );
      if (!list || !item) return;
      // Scroll only inside the listbox — never the page (scrollIntoView would).
      const itemTop = item.offsetTop;
      const itemBottom = itemTop + item.offsetHeight;
      if (itemTop < list.scrollTop) {
        list.scrollTop = itemTop;
      } else if (itemBottom > list.scrollTop + list.clientHeight) {
        list.scrollTop = itemBottom - list.clientHeight;
      }
    }, [highlight, open]);

    function moveHighlight(delta: number) {
      if (enabledIndexes.length === 0) return;
      const curPos = enabledIndexes.indexOf(highlight);
      let nextPos: number;
      if (curPos < 0) {
        nextPos = delta > 0 ? 0 : enabledIndexes.length - 1;
      } else {
        nextPos = (curPos + delta + enabledIndexes.length) % enabledIndexes.length;
      }
      setHighlight(enabledIndexes[nextPos]!);
    }

    function onTriggerKeyDown(e: React.KeyboardEvent) {
      if (disabled) return;
      if (e.key === "ArrowDown" || e.key === "ArrowUp" || e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        if (!open) {
          openSelect();
          return;
        }
        if (e.key === "ArrowDown") moveHighlight(1);
        else if (e.key === "ArrowUp") moveHighlight(-1);
        else if (e.key === "Enter" || e.key === " ") {
          if (highlight >= 0 && options[highlight] && !options[highlight]!.disabled) {
            commit(options[highlight]!.value);
          }
        }
      } else if (e.key === "Escape" && open) {
        e.preventDefault();
        close();
      } else if (e.key === "Home" && open) {
        e.preventDefault();
        if (enabledIndexes[0] != null) setHighlight(enabledIndexes[0]);
      } else if (e.key === "End" && open) {
        e.preventDefault();
        const last = enabledIndexes[enabledIndexes.length - 1];
        if (last != null) setHighlight(last);
      }
    }

    function onListKeyDown(e: React.KeyboardEvent) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        moveHighlight(1);
      } else if (e.key === "ArrowUp") {
        e.preventDefault();
        moveHighlight(-1);
      } else if (e.key === "Enter" || e.key === " ") {
        e.preventDefault();
        if (highlight >= 0 && options[highlight] && !options[highlight]!.disabled) {
          commit(options[highlight]!.value);
        }
      } else if (e.key === "Escape") {
        e.preventDefault();
        close();
        triggerRef.current?.focus();
      } else if (e.key === "Tab") {
        close();
      }
    }

    const sizeCls =
      size === "sm"
        ? "h-8 px-2.5 text-xs gap-1.5"
        : "h-9 px-3 text-sm gap-2";

    const panel =
      open && panelStyle && typeof document !== "undefined"
        ? createPortal(
            <div
              ref={listRef}
              role="listbox"
              id={id ? `${id}-listbox` : undefined}
              tabIndex={-1}
              style={panelStyle}
              className={cn(
                "overflow-y-auto overscroll-contain rounded-md border border-neutral-200 bg-white py-1 shadow-lg outline-none",
                "ring-1 ring-black/5"
              )}
              onKeyDown={onListKeyDown}
              onWheel={(e) => {
                // Keep wheel events inside the panel (don't bubble to page scroll).
                e.stopPropagation();
              }}
            >
              {options.length === 0 ? (
                <div className="px-3 py-2 text-sm text-neutral-400">No options</div>
              ) : (
                options.map((opt, i) => {
                  const isSelected = opt.value === value;
                  const isHi = i === highlight;
                  return (
                    <div
                      key={`${opt.value}::${i}`}
                      role="option"
                      aria-selected={isSelected}
                      aria-disabled={opt.disabled || undefined}
                      data-select-index={i}
                      className={cn(
                        "flex cursor-pointer items-center gap-2 px-3 py-1.5 text-sm outline-none",
                        size === "sm" && "py-1 text-xs",
                        opt.disabled && "cursor-not-allowed opacity-40",
                        !opt.disabled && isHi && "bg-neutral-100",
                        !opt.disabled && !isHi && "hover:bg-neutral-50",
                        isSelected && "font-medium text-neutral-900"
                      )}
                      onMouseEnter={() => {
                        if (!opt.disabled) setHighlight(i);
                      }}
                      onMouseDown={(e) => {
                        // Prevent trigger blur-before-click
                        e.preventDefault();
                      }}
                      onClick={() => {
                        if (opt.disabled) return;
                        commit(opt.value);
                      }}
                    >
                      <span className="min-w-0 flex-1 truncate">{opt.label}</span>
                      <Check
                        className={cn(
                          "h-3.5 w-3.5 shrink-0 text-neutral-900",
                          isSelected ? "opacity-100" : "opacity-0"
                        )}
                        aria-hidden
                      />
                    </div>
                  );
                })
              )}
            </div>,
            document.body
          )
        : null;

    return (
      <div ref={rootRef} className={cn("relative w-full min-w-0", className)}>
        <button
          ref={setTriggerRef}
          type="button"
          id={id}
          title={title ?? (labelText(display) || undefined)}
          disabled={disabled}
          aria-haspopup="listbox"
          aria-expanded={open}
          aria-controls={open && id ? `${id}-listbox` : undefined}
          className={cn(
            "flex w-full min-w-0 items-center justify-between rounded-md border border-neutral-200 bg-white text-left shadow-sm outline-none transition-colors",
            "hover:border-neutral-300",
            "focus-visible:border-neutral-400 focus-visible:ring-2 focus-visible:ring-neutral-100",
            "disabled:cursor-not-allowed disabled:opacity-50",
            open && "border-neutral-400 ring-2 ring-neutral-100",
            sizeCls
          )}
          onClick={() => {
            if (disabled) return;
            if (open) close();
            else openSelect();
          }}
          onKeyDown={onTriggerKeyDown}
        >
          <span
            className={cn(
              "min-w-0 flex-1 truncate",
              showPlaceholder ? "text-neutral-400" : "text-neutral-900"
            )}
          >
            {display}
          </span>
          <ChevronDown
            className={cn(
              "h-3.5 w-3.5 shrink-0 text-neutral-500 transition-transform",
              open && "rotate-180"
            )}
            aria-hidden
          />
        </button>
        {/* Hidden input for form semantics if name is set */}
        {name ? <input type="hidden" name={name} value={value} readOnly /> : null}
        {panel}
      </div>
    );
  }
);
Select.displayName = "Select";
