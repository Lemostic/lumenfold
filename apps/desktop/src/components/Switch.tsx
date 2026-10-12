import type { ButtonHTMLAttributes } from "react";

interface SwitchProps extends Omit<ButtonHTMLAttributes<HTMLButtonElement>, "onChange"> {
  /** 是否开启 */
  on: boolean;
  /** 状态切换回调 */
  onChange(next: boolean): void;
  /** 无障碍标签 */
  label: string;
  /** 选中状态下的描述(可选) */
  onLabel?: string;
  /** 未选中状态下的描述(可选) */
  offLabel?: string;
}

/**
 * Switch 开关。
 *
 * 用 button + role="switch" 而不是 checkbox,因为这是一个动作开关:
 * - 点击立即切换状态,而不是「勾上之后再确认」
 * - 选中态用琥珀填充,未选中用 surface-3 灰
 * - 拇指在轨道上滑动 12px,110ms 完成
 *
 * 一致性原则:用 Switch 的场景是「两种互斥模式之间的切换」,
 * 比如「标记模式」开/关。如果是要给一个属性赋值,仍然用 checkbox。
 */
export function Switch({
  on,
  onChange,
  label,
  onLabel,
  offLabel,
  disabled = false,
  className = "",
  ...rest
}: SwitchProps) {
  return (
    <span
      className={`inline-flex items-center gap-2 ${className}`}
      data-switch-state={on ? "on" : "off"}
    >
      <button
        type="button"
        role="switch"
        aria-checked={on}
        aria-label={label}
        title={on ? onLabel ?? label : offLabel ?? label}
        data-on={on ? "true" : "false"}
        data-disabled={disabled ? "true" : "false"}
        disabled={disabled}
        onClick={() => onChange(!on)}
        className="lf-switch"
        {...rest}
      >
        <span className="lf-switch__thumb" aria-hidden="true" />
      </button>
      <span className="text-xs text-ink-2 select-none">{on ? onLabel : offLabel}</span>
    </span>
  );
}
