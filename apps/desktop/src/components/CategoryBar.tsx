import { memo } from "react";
import { TAG_META } from "../lib/types";
import { useTagCounts } from "../state/library";
import { ui, useUi } from "../state/ui";

/**
 * 图库的类别栏。
 *
 * 放在图库顶部、网格与照片墙共用：类别是同一条时间线的一个切面，
 * 换布局不该让用户丢掉刚选的类别。选的类别存在 ui store 里，也是同一个理由。
 *
 * 计数直接来自宿主，不在这里数已加载的那几页：分页只加载了 600 条，
 * 拿它当分母会把「动图 37」说成全库的数字，而那是错的。
 */
export const CategoryBar = memo(function CategoryBar() {
  const active = useUi((s) => s.tagFilter);
  const counts = useTagCounts();

  const rows = counts.status === "ready" ? counts.value : null;
  const countFor = (id: string): number | null => {
    if (rows === null) return null;
    return rows.find((row) => row.id === id)?.count ?? 0;
  };
  const total = rows?.[0]?.total ?? null;
  const classified = rows?.[0]?.classified ?? null;

  /**
   * 分类是扫描时逐条补上的：老索引里的记录没有 CLASSIFIED 位，要等增量扫描
   * 把容器头重读一遍。这个差就是「还没数」的那部分——不把它说出来，
   * 用户会以为自己图库里真的没有动图。
   */
  const pending =
    total !== null && classified !== null && classified < total ? total - classified : 0;

  return (
    <div className="flex h-11 shrink-0 items-center gap-1.5 overflow-x-auto border-b border-line bg-surface px-3">
      <Chip label="全部" count={total} active={active === null} onClick={() => ui.setTagFilter(null)} />
      {TAG_META.map((entry) => (
        <Chip
          key={entry.id}
          label={entry.label}
          title={entry.hint}
          count={countFor(entry.id)}
          active={active === entry.id}
          // 再点一次当前类别就是取消筛选，省掉一次「先点全部」的往返。
          onClick={() => ui.setTagFilter(active === entry.id ? null : entry.id)}
        />
      ))}

      {pending > 0 ? (
        <span className="tabular ml-1 shrink-0 text-2xs text-ink-3">
          分类还在补齐：还有 {pending.toLocaleString("zh-CN")} 张没分过类
        </span>
      ) : null}
    </div>
  );
});

interface ChipProps {
  label: string;
  /** `null` 表示「还没问到」，和 0 不是一回事 */
  count: number | null;
  active: boolean;
  title?: string;
  onClick(): void;
}

function Chip({ label, count, active, title, onClick }: ChipProps) {
  return (
    <button
      type="button"
      // 原生 title 而不是 Tooltip 组件：这段说明是补充性的，而类别栏里
      // 每个 chip 都套一层 hover 气泡容器会让这一行多出十几个 DOM 包层。
      title={title}
      aria-pressed={active}
      onClick={onClick}
      className={[
        "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-md px-2.5 text-xs",
        "transition-colors duration-(--lf-duration-fast) ease-(--lf-ease)",
        active ? "bg-accent-soft text-accent" : "text-ink-2 hover:bg-surface-2 hover:text-ink",
      ].join(" ")}
    >
      <span>{label}</span>
      {/* 加载中给「—」而不是 0：0 是一个事实，「还没问到」不是。 */}
      <span className="tabular text-2xs text-ink-3">
        {count === null ? "—" : count.toLocaleString("zh-CN")}
      </span>
    </button>
  );
}
