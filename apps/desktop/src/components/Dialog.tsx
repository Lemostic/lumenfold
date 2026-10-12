import { useEffect, type ReactNode } from "react";
import { X } from "@phosphor-icons/react";

interface DialogProps {
  open: boolean;
  /** 无障碍与标题 */
  title: string;
  /** 描述段落,在标题之下。便于说明操作的副作用 */
  description?: string;
  /** 主操作区(按钮组)。放在底部右侧 */
  children?: ReactNode;
  /** 自定义底部内容,优先级低于 children(为同一位置) */
  footer?: ReactNode;
  /** 关闭回调。Escape 与遮罩点击都会调它 */
  onClose(): void;
  /** 阻止遮罩点击关闭(删除等危险操作必须开) */
  persistent?: boolean;
  /** 最大宽度档位 */
  size?: "sm" | "md" | "lg";
}

const SIZE: Record<NonNullable<DialogProps["size"]>, string> = {
  sm: "max-w-[360px]",
  md: "max-w-[460px]",
  lg: "max-w-[600px]",
};

/**
 * Dialog。v2 统一模态样式,替换 SettingsView 里内联的全屏遮罩。
 *
 * 设计原则:
 * - 背景模糊 + 60% 不透明遮罩,让用户保留对下方界面的位置感
 * - 标题、描述、操作三段式,与 Lumenfold 已有 EmptyState / ErrorState 一致
 * - Escape 关闭由 hook 处理;危险操作传 `persistent` 阻止点击遮罩关闭
 *
 * 不要在 Dialog 里再嵌一个 Dialog:叠加的模态会让焦点陷阱不可控。
 */
export function Dialog({
  open,
  title,
  description,
  children,
  footer,
  onClose,
  persistent = false,
  size = "md",
}: DialogProps) {
  useEscapeToClose(open, onClose);
  useLockBodyScroll(open);

  if (!open) return null;

  return (
    <div
      className="fixed inset-0 z-[var(--lf-z-dialog)] grid place-items-center bg-overlay p-6 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label={title}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return;
        if (persistent) return;
        onClose();
      }}
    >
      <div className={`chrome-modal w-full ${SIZE[size]} rounded-lg p-4`}>
        <div className="flex items-start justify-between gap-3">
          <div className="min-w-0">
            <h4 className="text-md font-medium tracking-tight text-ink">{title}</h4>
            {description ? (
              <p className="mt-1 text-sm leading-relaxed text-ink-2 text-pretty">
                {description}
              </p>
            ) : null}
          </div>
          <button
            type="button"
            onClick={onClose}
            aria-label="关闭"
            className="shrink-0 rounded-sm p-1 text-ink-3 transition-colors hover:bg-surface-2 hover:text-ink"
          >
            <X size={14} />
          </button>
        </div>

        {children ? <div className="mt-3">{children}</div> : null}

        {footer ? (
          <div className="mt-4 flex flex-wrap justify-end gap-2">{footer}</div>
        ) : null}
      </div>
    </div>
  );
}

function useEscapeToClose(open: boolean, onClose: () => void) {
  useEffect(() => {
    if (!open) return;
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);
}

function useLockBodyScroll(open: boolean) {
  useEffect(() => {
    if (!open) return;
    const previous = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    return () => {
      document.body.style.overflow = previous;
    };
  }, [open]);
}
