import type { ReactNode } from "react";

interface TooltipProps {
  /** 提示文本。空字符串或 null 时不渲染气泡 */
  text: string | null | undefined;
  children: ReactNode;
  /** 渲染为 inline 元素还是 block 元素 */
  inline?: boolean;
}

/**
 * Tooltip 包装器。
 *
 * 用 CSS hover/focus 而非 JS:磷光图标本身有 title 属性做兜底,
 * 加 CSS 气泡只是为了让有字面意义的工具栏按钮(如密度滑块)也有视觉提示。
 *
 * 关键原则:tooltip 必须是装饰性的、可有可无的。重要信息不能只放在 tooltip 里。
 */
export function Tooltip({ text, children, inline = true }: TooltipProps) {
  if (!text) {
    return <>{children}</>;
  }
  return (
    <span
      className={inline ? "lf-tooltip" : "lf-tooltip block"}
      tabIndex={0}
    >
      {children}
      <span role="tooltip" className="lf-tooltip__bubble">
        {text}
      </span>
    </span>
  );
}
