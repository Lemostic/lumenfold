import type { ReactNode } from "react";
import { WarningCircle } from "@phosphor-icons/react";
import { Button } from "./Button";

interface EmptyStateProps {
  icon: ReactNode;
  title: string;
  description: string;
  action?: ReactNode;
}

/**
 * 空状态。必须说清「为什么空」和「下一步做什么」，
 * 只有一句「暂无数据」的空状态等于把问题推回给用户。
 */
export function EmptyState({ icon, title, description, action }: EmptyStateProps) {
  return (
    <div className="flex h-full min-h-0 items-center justify-center px-8 py-12">
      <div className="max-w-sm">
        <div className="mb-5 flex h-12 w-12 items-center justify-center rounded-lg border border-line bg-surface text-ink-3">
          {icon}
        </div>
        <h2 className="text-md font-medium tracking-tight text-ink">{title}</h2>
        <p className="mt-2 text-sm leading-relaxed text-ink-2 text-pretty">{description}</p>
        {action ? <div className="mt-5 flex items-center gap-2">{action}</div> : null}
      </div>
    </div>
  );
}

interface BusyStateProps {
  label: string;
  detail?: string;
}

/**
 * 读取宿主状态的过渡态。
 * 不复用 EmptyState：空状态是一个结论，而这里还不知道结论是什么。
 * 首屏在拿到 library_stats 之前本来就短暂存在，把它渲染成「还没有打开图库」
 * 会先说一句假话再改口。
 */
export function BusyState({ label, detail }: BusyStateProps) {
  return (
    <div
      role="status"
      aria-live="polite"
      className="flex h-full min-h-0 items-center justify-center px-8 py-12"
    >
      <div className="flex max-w-sm flex-col items-center gap-3 text-center">
        <span aria-hidden="true" className="flex items-center gap-1">
          {[0, 1, 2].map((step) => (
            <span
              key={step}
              className="h-1 w-1 animate-pulse rounded-full bg-ink-3"
              style={{ animationDelay: `${step * 140}ms` }}
            />
          ))}
        </span>
        <p className="text-sm text-ink-2">{label}</p>
        {detail ? (
          <p className="text-xs leading-relaxed text-ink-3 text-pretty">{detail}</p>
        ) : null}
      </div>
    </div>
  );
}

interface ErrorStateProps {
  title: string;
  description: string;
  detail?: string;
  onRetry(): void;
}

/** 错误状态就地展示，附上宿主返回的原始信息，方便排障。 */
export function ErrorState({ title, description, detail, onRetry }: ErrorStateProps) {
  return (
    <div className="flex h-full min-h-0 items-center justify-center px-8 py-12">
      <div className="max-w-md">
        <div className="mb-5 flex h-12 w-12 items-center justify-center rounded-lg border border-line bg-danger-soft text-danger">
          <WarningCircle size={24} weight="regular" />
        </div>
        <h2 className="text-md font-medium tracking-tight text-ink">{title}</h2>
        <p className="mt-2 text-sm leading-relaxed text-ink-2">{description}</p>
        {detail ? (
          <pre className="selectable mt-4 overflow-x-auto rounded-md border border-line bg-sunken p-3 text-xs leading-relaxed whitespace-pre-wrap text-ink-3">
            {detail}
          </pre>
        ) : null}
        <div className="mt-5">
          <Button onClick={onRetry}>重试</Button>
        </div>
      </div>
    </div>
  );
}

interface SkeletonGridProps {
  columns: number;
  rowHeight: number;
  gap: number;
  viewportHeight: number;
}

/**
 * 骨架屏按最终网格的行列与行高铺满，真实数据到达时形状不变，不产生布局跳动。
 * 超过视口的部分不渲染，10 万张库的首屏骨架也只有几十个格子。
 */
export function SkeletonGrid({ columns, rowHeight, gap, viewportHeight }: SkeletonGridProps) {
  const rows = Math.max(1, Math.ceil(viewportHeight / (rowHeight + gap)) + 1);
  const cells = rows * columns;

  return (
    <div
      role="status"
      aria-label="正在加载缩略图"
      className="grid"
      style={{
        gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
        gap: `${gap}px`,
      }}
    >
      {Array.from({ length: cells }, (_, i) => (
        <div key={i} className="skeleton rounded-xs" style={{ height: rowHeight }} />
      ))}
    </div>
  );
}
