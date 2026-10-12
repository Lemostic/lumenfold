import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Check, FileImage, Flag, Star } from "@phosphor-icons/react";
import { thumbWidth } from "../state/library";
import { ui, useUi } from "../state/ui";
import type { DemoAsset } from "../lib/fixtures";

/**
 * 紧凑照片墙。
 *
 * 形制取自 ImageGlass:等大的格子、格子下面一条文件名栏、一屏能看很多张。
 * 它和 justified 网格的分工是——网格负责「精挑细看某一张」,照片墙负责
 * 「一屏看个大概,然后挨个打标记」。所以这里不做长短边混排:
 * 所有行等高,虚拟滚动纯靠算术定位,滚动和缩放都不需要测量 DOM。
 *
 * 注意:LayoutId="wall" 现在指向 WallView(单图大图模式)。
 * 本视图保留为 compact grid 形式,可在未来作为密集浏览模式再次启用。
 */

const GAP = 6;
/** 文件名栏高度。固定值,行高才能在渲染前算出来。 */
const LABEL_H = 22;
const OVERSCAN = 5;

interface ContactSheetViewProps {
  assets: DemoAsset[];
  width: number;
  cell: number;
  /** 按年月分组;关掉就是连续的一格格 */
  grouped: boolean;
  emptyState: React.ReactNode;
  onEndReached?(): void;
}

/** 拍摄年月。取不到拍摄时间就归到「日期未知」,而不是混进 1970 年。 */
function monthLabel(asset: DemoAsset): string {
  if (asset.takenAt === null) return "日期未知";
  const date = new Date(asset.takenAt * 1000);
  return `${date.getFullYear()}年${date.getMonth() + 1}月`;
}

export function ContactSheetView({
  assets,
  width,
  cell,
  grouped,
  emptyState,
  onEndReached,
}: ContactSheetViewProps) {
  const selected = useUi((s) => s.selected);
  const cursorId = useUi((s) => s.cursorId);
  const markMode = useUi((s) => s.markMode);
  const scrollRef = useRef<HTMLDivElement>(null);

  const columns = Math.max(1, Math.floor((width + GAP) / (cell + GAP)));
  const lineHeight = cell + LABEL_H + GAP;

  const virtualizer = useVirtualizer({
    count: Math.ceil(assets.length / columns),
    getScrollElement: () => scrollRef.current,
    estimateSize: () => lineHeight,
    overscan: OVERSCAN,
  });

  const orderedIds = useMemo(() => assets.map((a) => a.id), [assets]);

  /**
   * 顶部的日期标签取自当前视口的第一行。
   *
   * 用吸附标签而不是在流里插标题行:标题行会让行高不再统一,
   * estimateSize 就得逐行查表,虚拟滚动的纯算术定位也就没了。
   */
  const stickyLabel = useMemo(() => {
    if (!grouped) return null;
    const first = virtualizer.getVirtualItems()[0]?.index ?? 0;
    const asset = assets[first * columns];
    return asset ? monthLabel(asset) : null;
  }, [grouped, assets, columns, virtualizer]);

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (event.ctrlKey || event.metaKey) {
        if (event.key === "a") {
          event.preventDefault();
          ui.selectAll(orderedIds);
          return;
        }
        if (event.key === "i") {
          event.preventDefault();
          ui.invertSelection(orderedIds);
          return;
        }
      }
      if (event.key === "Escape") {
        event.preventDefault();
        ui.clearSelection();
      }
    },
    [orderedIds],
  );

  /**
   * 排序或翻页之后,光标可能指向一条已经不在列表里的资产。
   * 留着会让后续的方向键操作找不到锚点,所以拉回第一条。
   */
  useEffect(() => {
    if (orderedIds.length === 0) {
      if (cursorId !== null) ui.moveCursor(null);
      return;
    }
    if (cursorId !== null && orderedIds.includes(cursorId)) return;
    ui.moveCursor(orderedIds[0] ?? null);
  }, [cursorId, orderedIds]);

  if (assets.length === 0) return <>{emptyState}</>;

  return (
    <div
      ref={scrollRef}
      role="grid"
      aria-label="照片墙"
      aria-rowcount={Math.ceil(assets.length / columns)}
      aria-colcount={columns}
      tabIndex={0}
      onKeyDown={handleKeyDown}
      onScroll={
        onEndReached
          ? (event) => {
              const el = event.currentTarget;
              if (el.scrollHeight - el.scrollTop - el.clientHeight < el.clientHeight * 2) {
                onEndReached();
              }
            }
          : undefined
      }
      className="h-full overflow-y-auto scroll-area focus-visible:outline-offset-[-2px]"
    >
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const first = virtualRow.index * columns;
          const rowAssets = assets.slice(first, first + columns);

          return (
            <div
              key={virtualRow.key}
              role="row"
              aria-rowindex={virtualRow.index + 1}
              className="absolute top-0 left-0 flex"
              style={{
                transform: `translateY(${virtualRow.start}px)`,
                height: lineHeight,
                gap: GAP,
                contain: "layout paint style",
              }}
            >
              {rowAssets.map((asset) => (
                <WallCell
                  key={asset.id}
                  asset={asset}
                  cell={cell}
                  isSelected={selected.has(asset.id)}
                  isCursor={cursorId === asset.id}
                  markMode={markMode}
                  onActivate={(event) => {
                    const id = asset.id;
                    if (event.shiftKey && cursorId !== null) {
                      ui.selectRange(cursorId, id, orderedIds);
                      return;
                    }
                    if (markMode || event.ctrlKey || event.metaKey) {
                      ui.toggleMark(id);
                      return;
                    }
                    ui.selectOnly(id);
                  }}
                />
              ))}
            </div>
          );
        })}
      </div>

      {stickyLabel !== null ? (
        <div
          aria-hidden="true"
          className="pointer-events-none sticky top-0 z-10 -mt-6 flex h-6 items-center bg-linear-to-b from-bg to-bg/0 px-0.5 text-2xs font-medium tracking-wide text-ink-2"
        >
          {stickyLabel}
        </div>
      ) : null}
    </div>
  );
}

interface WallCellProps {
  asset: DemoAsset;
  cell: number;
  isSelected: boolean;
  isCursor: boolean;
  markMode: boolean;
  onActivate(event: React.MouseEvent<HTMLButtonElement>): void;
}

const thumbSrcCache = new Map<string, { cell: number; src: string }>();

function thumbSrcFor(url: string, cell: number): string {
  const cached = thumbSrcCache.get(url);
  if (cached && cached.cell === cell) return cached.src;
  const src = withWidth(url, cell);
  thumbSrcCache.set(url, { cell, src });
  return src;
}

const WallCell = memo(function WallCell({
  asset,
  cell,
  isSelected,
  isCursor,
  markMode,
  onActivate,
}: WallCellProps) {
  const hasDimensions = asset.width > 0 && asset.height > 0;
  const [decodeFailed, setDecodeFailed] = useState(false);
  const showPlaceholder = !hasDimensions || decodeFailed;

  return (
    <button
      type="button"
      role="gridcell"
      aria-selected={isSelected}
      aria-label={`${asset.fileName}${asset.rating > 0 ? `,${asset.rating} 星` : ""}${isSelected ? ",已标记" : ""}`}
      tabIndex={-1}
      onClick={onActivate}
      className={[
        "group relative block shrink-0 overflow-hidden rounded-xs bg-sunken text-left",
        "transition-shadow duration-(--lf-duration-fast) ease-(--lf-ease)",
        isSelected
          ? "shadow-[0_0_0_2px_var(--lf-accent)]"
          : "shadow-[0_0_0_1px_var(--lf-border)]",
        isCursor ? "outline-2 outline-offset-1 outline-(--lf-focus)" : "outline-none",
      ].join(" ")}
      style={{ width: cell }}
    >
      <span className="relative block" style={{ height: cell }}>
        {showPlaceholder ? (
          <span className="flex h-full w-full items-center justify-center text-ink-3">
            <FileImage size={18} />
          </span>
        ) : (
          <img
            src={thumbSrcFor(asset.thumbUrl, cell)}
            alt=""
            loading="lazy"
            decoding="async"
            draggable={false}
            onError={() => setDecodeFailed(true)}
            className="thumb-enter h-full w-full object-cover"
          />
        )}

        {markMode && isSelected ? (
          <span
            aria-hidden="true"
            className="absolute top-1 right-1 flex h-4 w-4 items-center justify-center rounded-full bg-accent text-accent-ink"
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

        {markMode && !isSelected ? (
          <span
            aria-hidden="true"
            className="absolute inset-0 grid place-items-center bg-black/0 opacity-0 transition-opacity group-hover:bg-black/25 group-hover:opacity-100"
          >
            <span className="flex h-6 w-6 items-center justify-center rounded-full border-2 border-white/90">
              <Check size={12} weight="bold" className="text-white" />
            </span>
          </span>
        ) : null}
      </span>

      <span
        className="flex items-center truncate bg-surface px-1.5 text-2xs text-ink-3"
        style={{ height: LABEL_H }}
      >
        {asset.fileName}
      </span>
    </button>
  );
});

function withWidth(url: string, cell: number): string {
  const width = thumbWidth(Math.ceil(cell * devicePixelRatio));
  return /[?&]w=\d+/.test(url)
    ? url.replace(/([?&]w=)\d+/, `$1${width}`)
    : url;
}
