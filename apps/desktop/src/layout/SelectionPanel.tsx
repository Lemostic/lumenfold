import { X } from "@phosphor-icons/react";
import { formatBytes, formatCount } from "../lib/format";
import { assetById } from "../state/library";
import { ui } from "../state/ui";
import type { DemoAsset } from "../lib/fixtures";

/** 预览图完全占满宽度时的高度，与检查器一致。 */
const PREVIEW_ASPECT = "aspect-[3/2]";

/**
 * 已选栏。
 *
 * 只在标记模式下、且确实有东西被标记时出现。有它的原因是标记模式的操作
 * 是「从头到尾一点一点勾」，用户的注意力在网格上，而不是在计数上。
 * 列表负责给他一个可回溯的记录，也能在那里取消。
 *
 * 排序是「最新选的在最上面」而不是「按文件名」或「按拍摄时间」：
 * 列表是一个做事顺序的录存，顺序就是信息。
 */
export function SelectionPanel({ order }: { order: readonly string[] }) {
  // 从 store 的序列直接取一个 id 是不够的：列表里需要文件名与缩略图。
  const items = order.map((id) => assetById(id)).filter((asset): asset is DemoAsset => asset !== null);
  const latest = items[0] ?? null;

  return (
    <aside
      aria-label="已选栏"
      className={[
        "flex w-70 shrink-0 flex-col border-l border-line bg-surface",
        "max-[1179px]:absolute max-[1179px]:inset-y-0 max-[1179px]:right-0",
        "max-[1179px]:z-[var(--lf-z-overlay)] max-[1179px]:shadow-pop",
      ].join(" ")}
    >
      <div className="flex h-11 shrink-0 items-center justify-between border-b border-line px-3">
        <h2 className="tabular text-sm font-medium text-ink">已选 {formatCount(items.length)} 项</h2>
        <button
          type="button"
          onClick={() => ui.clearSelection()}
          className="rounded-sm px-1.5 py-1 text-2xs text-ink-3 transition-colors hover:bg-surface-2 hover:text-ink"
        >
          全部取消
        </button>
      </div>

      {latest ? (
        <section className="shrink-0 border-b border-line p-3">
          <img
            src={latest.thumbUrl}
            alt=""
            className={`${PREVIEW_ASPECT} w-full rounded-md border border-line bg-sunken object-cover`}
            loading="lazy"
            decoding="async"
          />
          <p className="mt-2 truncate text-xs font-medium text-ink" title={latest.path}>
            {latest.fileName}
          </p>
          <p className="tabular mt-0.5 text-2xs text-ink-3">
            最近标记 · {latest.width} × {latest.height} · {formatBytes(latest.bytes)}
          </p>
        </section>
      ) : null}

      {/* 列表自己滚动：预览固定在上面，下面的长尾可以尽量多几行。 */}
      <ul className="min-h-0 flex-1 overflow-y-auto scroll-area">
        {items.map((asset) => (
          <li key={asset.id} className="group flex items-center gap-2 border-b border-line px-2 py-1.5">
            <img
              src={asset.thumbUrl}
              alt=""
              className="h-9 w-12 shrink-0 rounded-xs border border-line bg-sunken object-cover"
              loading="lazy"
              decoding="async"
            />
            <span className="min-w-0 flex-1 truncate text-2xs text-ink-2" title={asset.fileName}>
              {asset.fileName}
            </span>
            <button
              type="button"
              onClick={() => ui.deselect(asset.id)}
              aria-label={`取消标记 ${asset.fileName}`}
              className="shrink-0 rounded-sm p-1 text-ink-3 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-surface-2 hover:text-danger focus-visible:opacity-100"
            >
              <X size={12} />
            </button>
          </li>
        ))}
      </ul>
    </aside>
  );
}
