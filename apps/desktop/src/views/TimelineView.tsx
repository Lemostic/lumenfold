import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Check, ClockCounterClockwise, Flag, Star } from "@phosphor-icons/react";
import { EmptyState, ErrorState, SkeletonGrid } from "../components/States";
import { Lightbox } from "../components/Lightbox";
import { usePagedAssets } from "../state/library";
import { useElementWidth } from "../lib/useElementWidth";
import { dayKey, formatDayHeading, formatMonthHeading, monthKey } from "../lib/format";
import type { DemoAsset } from "../lib/fixtures";
import { errorDescription, errorTitle } from "./LibraryView";
import { justifyRows, type JustifyRow } from "../lib/justified";
import { ui, useUi } from "../state/ui";

const GAP = 3;
const HEADER_HEIGHT = 30;
const PAD_X = 12;

const NO_ASSETS: DemoAsset[] = [];

/** 时间线里的资产尺寸来自扫描器，缺失时退回 3:2 占位比例。 */
const measureAsset = (asset: DemoAsset): { width: number; height: number } =>
  asset.width > 0 && asset.height > 0 ? asset : { width: 3, height: 2 };

/**
 * 时间线。分组头也是虚拟列表里的一种「块」，高度固定，
 * 于是整条时间线依然是纯算术定位，不需要逐块测量，也不需要滚动编排。
 */
type Block =
  | { kind: "month"; id: string; label: string; height: number }
  | { kind: "day"; id: string; label: string; height: number }
  | { kind: "row"; id: string; height: number; cells: JustifyRow<DemoAsset>["cells"] };

function buildBlocks(
  assets: DemoAsset[],
  width: number,
  rowHeight: number,
): Block[] {
  const contentWidth = Math.max(width - PAD_X * 2, 120);
  const blocks: Block[] = [];
  let currentMonth = "";
  let currentDay = "";
  let bucket: DemoAsset[] = [];

  const pushRows = () => {
    if (bucket.length === 0) return;
    for (const row of justifyRows(bucket, measureAsset, contentWidth, rowHeight, GAP)) {
      blocks.push({ kind: "row", id: `row-${blocks.length}`, height: row.height, cells: row.cells });
    }
    bucket = [];
  };

  for (const asset of assets) {
    const month = monthKey(asset.takenAt);
    const day = dayKey(asset.takenAt);

    if (month !== currentMonth) {
      pushRows();
      currentMonth = month;
      blocks.push({
        kind: "month",
        id: `month-${month}`,
        label: asset.takenAt === null ? "未知时间" : formatMonthHeading(asset.takenAt),
        height: HEADER_HEIGHT,
      });
      currentDay = "";
    }

    if (day !== currentDay) {
      pushRows();
      currentDay = day;
      blocks.push({
        kind: "day",
        id: `day-${day}`,
        label: formatDayHeading(asset.takenAt ?? 0),
        height: HEADER_HEIGHT,
      });
    }

    bucket.push(asset);
  }

  pushRows();
  return blocks;
}

export function TimelineView() {
  const sort = useUi((s) => s.sort);
  const rowHeight = useUi((s) => s.rowHeight);
  const selected = useUi((s) => s.selected);
  const markMode = useUi((s) => s.markMode);
  const cursorId = useUi((s) => s.cursorId);

  const assets = usePagedAssets({ sort, bucket: null, personId: null, tag: null });
  const { ref, width } = useElementWidth<HTMLDivElement>();
  const scrollRef = useRef<HTMLDivElement>(null);

  const list = assets.status === "ready" ? assets.assets : NO_ASSETS;

  // Lightbox 自己管 state：时间线没有外部 parent 提供光标和选区，
  // 这里选中的「当前要看的大图」就是双击的那张，按 prev/next 在这一页里翻。
  const [lightboxId, setLightboxId] = useState<string | null>(null);
  const lightboxAsset = useMemo(
    () => (lightboxId === null ? null : list.find((a) => a.id === lightboxId) ?? null),
    [lightboxId, list],
  );
  const lightboxNeighbors = useMemo(() => {
    if (lightboxAsset === null) return { prev: null, next: null };
    const index = list.findIndex((a) => a.id === lightboxAsset.id);
    if (index < 0) return { prev: null, next: null };
    return {
      prev: index > 0 ? list[index - 1] ?? null : null,
      next: index < list.length - 1 ? list[index + 1] ?? null : null,
    };
  }, [lightboxAsset, list]);
  const openLightbox = useCallback((asset: DemoAsset) => {
    ui.selectOnly(asset.id);
    ui.moveCursor(asset.id);
    setLightboxId(asset.id);
  }, []);
  const closeLightbox = useCallback(() => setLightboxId(null), []);

  // 资产列表换页/重新载入时，关掉自动清掉开着的 Lightbox：
  // 引用 assetId 已经不在内存里的话，Lightbox 会空，没必要让它一直挂。
  useEffect(() => {
    if (lightboxId === null) return;
    if (!list.some((a) => a.id === lightboxId)) setLightboxId(null);
  }, [list, lightboxId]);

  const blocks = useMemo(
    () => (width > 0 && list.length > 0 ? buildBlocks(list, width, rowHeight) : []),
    [list, width, rowHeight],
  );

  const offsets = useMemo(() => {
    const table: number[] = [];
    let cursor = 0;
    for (const block of blocks) {
      table.push(cursor);
      cursor += block.height + (block.kind === "row" ? GAP : 0);
    }
    return table;
  }, [blocks]);

  const virtualizer = useVirtualizer({
    count: blocks.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: (index) => (blocks[index]?.height ?? rowHeight) + GAP,
    overscan: 3,
  });

  const scrollOffset = virtualizer.scrollOffset ?? 0;

  /** 粘性日头：滚动位置之上最近的一个 day 块。行高固定，直接算即可。 */
  const pinnedDay = useMemo(() => {
    let label: string | null = null;
    let firstDayOffset = Infinity;
    for (let i = 0; i < blocks.length; i += 1) {
      const offset = offsets[i];
      const block = blocks[i];
      if (offset === undefined || !block) continue;
      if (block.kind === "day") firstDayOffset = Math.min(firstDayOffset, offset);
      if (offset > scrollOffset + 1) break;
      if (block.kind === "day") label = block.label;
    }
    return { label, visible: label !== null && scrollOffset > firstDayOffset };
  }, [blocks, offsets, scrollOffset]);

  if (assets.status === "error") {
    return (
      <ErrorState
        title={errorTitle(assets.raw)}
        description={errorDescription(assets.raw)}
        detail={assets.message}
        onRetry={assets.reload}
      />
    );
  }

  const orderedIds = list.map((a) => a.id);

  return (
    <div className="relative h-full min-h-0">
      <div ref={ref} className="h-full min-h-0 overflow-hidden">
        {assets.status === "loading" ? (
          <div className="p-3">
            <SkeletonGrid
              columns={Math.max(1, Math.floor(width / (rowHeight * 1.4)))}
              rowHeight={rowHeight}
              gap={GAP}
              viewportHeight={560}
            />
          </div>
        ) : blocks.length === 0 ? (
          <EmptyState
            icon={<ClockCounterClockwise size={22} />}
            title="还没有可按时间排列的照片"
            description="时间线按拍摄时间分组。文件读不到拍摄时间时会归到「未知时间」，不会丢失，只是不参与排序。"
          />
        ) : (
          <div
            ref={scrollRef}
            role="grid"
            aria-label="时间线"
            className="h-full overflow-y-auto scroll-area"
            onScroll={assets.more ? (event) => {
              const el = event.currentTarget;
              if (el.scrollHeight - el.scrollTop - el.clientHeight < el.clientHeight * 2) assets.loadMore();
            } : undefined}
          >
            <div className="relative" style={{ height: virtualizer.getTotalSize() }}>
              {virtualizer.getVirtualItems().map((item) => {
                const block = blocks[item.index];
                const offset = offsets[item.index];
                if (!block || offset === undefined) return null;

                if (block.kind === "month") {
                  return (
                    <div
                      key={block.id}
                      style={{ transform: `translateY(${offset}px)`, height: block.height }}
                      className="absolute top-0 left-0 flex w-full items-end px-3"
                    >
                      <h2 className="pb-1 text-xs font-medium tracking-wide text-ink-3">
                        {block.label}
                      </h2>
                    </div>
                  );
                }

                if (block.kind === "day") {
                  return (
                    <div
                      key={block.id}
                      style={{ transform: `translateY(${offset}px)`, height: block.height }}
                      className="absolute top-0 left-0 flex w-full items-end px-3"
                    >
                      <h3 className="tabular pb-1 text-xs text-ink-2">{block.label}</h3>
                    </div>
                  );
                }

                return (
                  <div
                    key={block.id}
                    role="row"
                    style={{
                      transform: `translateY(${offset}px)`,
                      height: block.height,
                      gap: GAP,
                    }}
                    className="absolute top-0 left-0 flex px-3"
                  >
                    {block.cells.map((cell) => {
                      const asset = cell.data;
                      const isSelected = selected.has(asset.id);
                      return (
                        <button
                          key={asset.id}
                          type="button"
                          role="gridcell"
                          aria-selected={isSelected}
                          // 双击在弹 Lightbox 之前要做的事：单击还要继续走 selectOnly 那条路，
                          // 标记模式下双击和单击都只动标记集合（避免一次双击刷两次光标）。
                          // 这里 onClick 的逻辑是「shift 连选 / ctrl+meta 打标记 / 否则单选换光标」，
                          // 把双击叠在这之后：非模式 → 弹大图，模式 → noop。
                          onDoubleClick={() => {
                            if (!markMode) openLightbox(asset);
                          }}
                          aria-label={asset.fileName}
                          onClick={(event) => {
                            // shift 连选、ctrl 挨个打标记、裸单击换选区：
                            // 三个视图必须同一套规则，否则用户每换一个视图就要重新学一遍。
                            if (event.shiftKey && cursorId !== null) {
                              ui.selectRange(cursorId, asset.id, orderedIds);
                              return;
                            }
                            if (markMode || event.ctrlKey || event.metaKey) {
                              ui.toggleMark(asset.id);
                              return;
                            }
                            ui.selectOnly(asset.id);
                          }}
                          className={[
                            "group relative block shrink-0 overflow-hidden rounded-xs bg-sunken",
                            "transition-shadow duration-(--lf-duration-fast) ease-(--lf-ease)",
                            isSelected
                              ? "shadow-[0_0_0_2px_var(--lf-accent)]"
                              : "shadow-[0_0_0_1px_var(--lf-border)]",
                          ].join(" ")}
                          style={{ width: cell.width, height: cell.height }}
                        >
                          <img
                            src={asset.thumbUrl}
                            alt=""
                            loading="lazy"
                            decoding="async"
                            draggable={false}
                            className="thumb-enter h-full w-full object-cover"
                          />
                          {markMode && isSelected ? (
                            <span
                              aria-hidden="true"
                              className="absolute right-1.5 bottom-1.5 flex h-4 w-4 items-center justify-center rounded-full bg-accent text-accent-ink"
                            >
                              <Check size={10} weight="bold" />
                            </span>
                          ) : null}
                          {!isSelected && asset.flagged ? (
                            <span aria-hidden="true" className="absolute top-1.5 left-1.5 text-accent">
                              <Flag size={12} weight="fill" />
                            </span>
                          ) : null}
                          {!isSelected && asset.rating > 0 ? (
                            <span aria-hidden="true" className="absolute top-1.5 left-1.5 text-accent">
                              <Star size={12} weight="fill" />
                            </span>
                          ) : null}
                        </button>
                      );
                    })}
                  </div>
                );
              })}
            </div>
          </div>
        )}
      </div>

      {pinnedDay.visible ? (
        <div
          aria-hidden="true"
          className="pointer-events-none absolute inset-x-0 top-0 z-[var(--lf-z-sticky)] h-9 border-b border-line bg-surface/92 backdrop-blur-sm"
        >
          <div className="flex h-full items-center px-3">
            <span className="tabular text-xs text-ink-2">{pinnedDay.label}</span>
          </div>
        </div>
      ) : null}

      {lightboxAsset ? (
        <Lightbox
          asset={lightboxAsset}
          prev={lightboxNeighbors.prev}
          next={lightboxNeighbors.next}
          onClose={closeLightbox}
          onPrev={() => {
            const target = lightboxNeighbors.prev;
            if (target) openLightbox(target);
          }}
          onNext={() => {
            const target = lightboxNeighbors.next;
            if (target) openLightbox(target);
          }}
        />
      ) : null}
    </div>
  );
}
