import type { ButtonHTMLAttributes, ReactNode } from "react";

type Variant = "primary" | "secondary" | "ghost" | "danger" | "link";
type Size = "sm" | "md" | "lg";

/**
 * 圆角规则:控件一律 6px,不随尺寸变化。
 * 尺寸只改内边距与字号,圆角是令牌里的一条规则,不允许在组件里另开一套。
 */
const SIZE_CLASS: Record<Size, string> = {
  sm: "h-7 px-2.5 text-xs gap-1.5",
  md: "h-8 px-3 text-sm gap-2",
  lg: "h-10 px-4 text-md gap-2",
};

const VARIANT_CLASS: Record<Variant, string> = {
  // 主按钮:实心强调色 + 深色文字
  primary:
    "bg-accent text-accent-ink hover:bg-accent-hover active:bg-accent-active active:translate-y-px disabled:hover:bg-accent",
  secondary:
    "bg-surface-2 text-ink border border-line hover:bg-surface-3 hover:border-line-strong active:translate-y-px",
  ghost: "text-ink-2 hover:bg-surface-2 hover:text-ink active:translate-y-px",
  danger: "bg-danger-soft text-danger hover:bg-danger hover:text-accent-ink active:translate-y-px",
  link: "text-link hover:text-link underline-offset-4 hover:underline active:translate-y-px",
};

interface ButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  variant?: Variant;
  size?: Size;
  icon?: ReactNode;
  trailing?: ReactNode;
}

export function Button({
  variant = "secondary",
  size = "md",
  icon,
  trailing,
  children,
  className = "",
  type = "button",
  ...rest
}: ButtonProps) {
  return (
    <button
      type={type}
      className={[
        "inline-flex shrink-0 items-center justify-center rounded-md font-medium",
        "transition-colors duration-(--lf-duration-fast) ease-(--lf-ease)",
        "disabled:cursor-not-allowed disabled:opacity-45 disabled:active:translate-y-0",
        SIZE_CLASS[size],
        VARIANT_CLASS[variant],
        className,
      ].join(" ")}
      {...rest}
    >
      {icon}
      {children}
      {trailing}
    </button>
  );
}

interface IconButtonProps extends ButtonHTMLAttributes<HTMLButtonElement> {
  label: string;
  active?: boolean;
  /** 尺寸档位。sm = 24px / md = 28px(默认)/ lg = 32px */
  size?: "sm" | "md" | "lg";
}

/** 图标按钮。方形 6px 圆角,标签走 aria-label。 */
const ICON_SIZE_CLASS: Record<NonNullable<IconButtonProps["size"]>, string> = {
  sm: "h-6 w-6",
  md: "h-7 w-7",
  lg: "h-8 w-8",
};

export function IconButton({
  label,
  active = false,
  size = "md",
  className = "",
  children,
  ...rest
}: IconButtonProps) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      aria-pressed={active}
      className={[
        "inline-flex shrink-0 items-center justify-center rounded-md",
        "transition-colors duration-(--lf-duration-fast) ease-(--lf-ease) active:translate-y-px",
        active ? "bg-accent-soft text-accent" : "text-ink-2 hover:bg-surface-2 hover:text-ink",
        "disabled:cursor-not-allowed disabled:opacity-45",
        ICON_SIZE_CLASS[size],
        className,
      ].join(" ")}
      {...rest}
    >
      {children}
    </button>
  );
}
