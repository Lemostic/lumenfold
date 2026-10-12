import type { ReactNode } from "react";

interface Segment<T extends string> {
  value: T;
  label: string;
  icon?: ReactNode;
}

interface SegmentedProps<T extends string> {
  value: T;
  options: Array<Segment<T>>;
  onChange(value: T): void;
  label: string;
  /**
   * 写入进行中。
   *
   * 需要它是因为「点了没反应」是一种说谎：控件看上去还能点，实际上在吞掉这一次
   * 点击。与其在 `onChange` 里悄悄 return，不如让它真的变成 disabled。
   */
  disabled?: boolean;
}

/** 分段控件。胶囊形,符合圆角规则里「芯片与头像用 999px」这一条。 */
export function Segmented<T extends string>({ value, options, onChange, label, disabled }: SegmentedProps<T>) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      aria-disabled={disabled || undefined}
      className="inline-flex items-center gap-0.5 rounded-full border border-line bg-sunken p-0.5"
    >
      {options.map((option) => {
        const active = option.value === value;
        return (
          <button
            key={option.value}
            type="button"
            role="radio"
            aria-checked={active}
            disabled={disabled}
            onClick={() => onChange(option.value)}
            className={[
              "inline-flex h-6 items-center gap-1.5 rounded-full px-2.5 text-xs font-medium",
              "transition-colors duration-(--lf-duration-fast) ease-(--lf-ease)",
              active ? "bg-surface-3 text-ink" : "text-ink-3 hover:text-ink-2",
              disabled ? "cursor-not-allowed opacity-50 hover:text-ink-3" : "",
            ].join(" ")}
          >
            {option.icon}
            {option.label}
          </button>
        );
      })}
    </div>
  );
}
