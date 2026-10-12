import {
  memo,
  useCallback,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { Check, FileImage, Flag, Star } from "@phosphor-icons/react";
import { justifyRows, type JustifyMeasure } from "../lib/justified";

import { thumbWidth } from "../state/library";
import { TAG_ANIMATED, hasTag } from "../lib/types";
import { ui, useUi } from "../state/ui";
import type { DemoAsset } from "../lib/fixtures";

const GAP = 3;
/**
 * 预置多少行。小的时候快速滚动会赶上虚拟化的落后，行边置出现一条空白，
 * 而空白再被新图填上——用户看起来就是「卡顿」。多预置两行换来的是极少的额外 DOM。
 */
const OVERSCAN = 5;
/** 读不到尺寸时排版用的占位比例，只影响格子大小，不冒充真实尺寸 */
const PLACEHOLDER_ASPECT = { width: 3, height: 2 };

/** 排版用的测量：尺寸缺失时退回占位比例，渲染时另行显示「无法解码」占位。 */
export function measureGridItem(item: GridItem): JustifyMeasure {
  return item.hasDimensions
    ? { width: item.asset.width, height: item.asset.height }
    : PLACEHOLDER_ASPECT;
}

interface GridItem {
  asset: DemoAsset;
  /** 尺寸可读时为 true；否则渲染成「无法解码」占位而不是伪造的图片 */
  hasDimensions: boolean;
}

interface JustifiedGridProps {
  assets: DemoAsset[];
  width: number;
  rowHeight: number;
  emptyState: ReactNode;
  /** 悬停显示文件名。空间紧张时关掉,省掉每格的渐变层 */
  showFilename: boolean;
  /** 滚到接近末尾时触发,用于分页拉取下一批 */
  onEndReached?(): void;
  /** 双击一张照片(未开启标记模式时)。弹最大化原图 */
  onActivateDoubleClick?(asset: DemoAsset): void;
}

/**
 * 虚拟化 justified 网格。
 *
 * 只渲染视口内的行，行的位置由固定行高纯算术推出，不做 DOM 测量。
 * 缩略图请求全部落在 lumen://thumb 上，由浏览器自己做缓存、解码与并发调度，
 * 宿主不逐张走 IPC，这是十万张滚动不掉帧的真正原因。
 */
export function JustifiedGrid({
  assets,
  width,
  rowHeight,
  emptyState,
  showFilename,
  onEndReached,
  onActivateDoubleClick,
}: JustifiedGridProps) {
  const selected = useUi((s) => s.selected);
  const cursorId = useUi((s) => s.cursorId);
  const markMode = useUi((s) => s.markMode);
  const scrollRef = useRef<HTMLDivElement>(null);

  const items = useMemo<GridItem[]>(
    () =>
      assets.map((asset) => ({
        asset,
        hasDimensions: asset.width > 0 && asset.height > 0,
      })),
    [assets],
  );

  const rows = useMemo(
    () => justifyRows(items, measureGridItem, width, rowHeight, GAP),
    [items, width, rowHeight],
  );
  const orderedIds = useMemo(() => assets.map((a) => a.id), [assets]);
  /**
   * 光标是否还在当前结果集里。
   *
   * 之前这个检查是 `orderedIds.includes(cursorId)`，而这个 effect 依赖的正好是
   * `orderedIds`，而它每次分页都是新数组——于是每滚一页就在数组上线性地扫一遍。
   * 已加载到上万张时这就是每次滚到底都会慢一下的那一次。
   */
  const idSet = useMemo(() => new Set(orderedIds), [orderedIds]);

  const virtualizer = useVirtualizer({
    count: rows.length,
    getScrollElement: () => scrollRef.current,
    estimateSize: () => rowHeight + GAP,
    overscan: OVERSCAN,
  });

  const locate = useCallback(
    (id: string) => {
      for (let rowIndex = 0; rowIndex < rows.length; rowIndex += 1) {
        const colIndex = rows[rowIndex]?.cells.findIndex((c) => c.data.asset.id === id) ?? -1;
        if (colIndex >= 0) return { rowIndex, colIndex };
      }
      return null;
    },
    [rows],
  );

  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (event.key === "a" && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        ui.selectAll(orderedIds);
        return;
      }
      if (event.key === "Escape") {
        ui.clearSelection();
        return;
      }
      if (event.key === "Home" || event.key === "End") {
        event.preventDefault();
        const id = event.key === "Home" ? orderedIds[0] : orderedIds[orderedIds.length - 1];
        if (id !== undefined) ui.moveCursor(id);
        return;
      }
      if (cursorId === null) return;

      if (event.key === " " || event.key === "Enter") {
        event.preventDefault();
        ui.toggleMark(cursorId);
        return;
      }

      const here = locate(cursorId);
      if (!here) return;
      const rowLength = rows[here.rowIndex]?.cells.length ?? 0;
      let offset = 0;
      if (event.key === "ArrowRight") offset = 1;
      else if (event.key === "ArrowLeft") offset = -1;
      else if (event.key === "ArrowDown") offset = rowLength;
      else if (event.key === "ArrowUp") offset = -rowLength;
      else return;

      event.preventDefault();
      const targetCol = here.colIndex + offset;
      // 上下移动把列夹在相邻行内，左右移动越过行边界时落到相邻行
      const neighbourRow =
        rows[here.rowIndex + (event.key === "ArrowDown" ? 1 : event.key === "ArrowUp" ? -1 : 0)];
      const maxCol =
        event.key === "ArrowRight" || event.key === "ArrowLeft"
          ? rowLength - 1
          : Math.max((neighbourRow?.cells.length ?? 1) - 1, 0);
      const clampedCol = Math.min(Math.max(targetCol, 0), maxCol);
      const target = rows[here.rowIndex]?.cells[clampedCol]?.data.asset.id;
      if (target === undefined) return;

      ui.moveCursor(target);
      if (event.shiftKey) ui.selectRange(cursorId, target, orderedIds);
    },
    [cursorId, locate, orderedIds, rows],
  );

  /**
   * 光标必须留在当前结果集里。
   * 排序、人物筛选或翻页之后，光标可能指向一条已经不在列表里的资产；
   * 此时 locate 返回 null，方向键会完全失灵，直到用户先点一下。
   * 所以把光标拉回第一条可见记录，结果为空时清掉。
   */
  useEffect(() => {
    if (orderedIds.length === 0) {
      if (cursorId !== null) ui.moveCursor(null);
      return;
    }
    if (cursorId !== null && idSet.has(cursorId)) return;
    ui.moveCursor(orderedIds[0] ?? null);
  }, [cursorId, idSet, orderedIds]);

  if (assets.length === 0) return <>{emptyState}</>;

  return (
    <div
      ref={scrollRef}
      role="grid"
      aria-label="照片网格"
      aria-rowcount={rows.length}
      tabIndex={0}
      onScroll={onEndReached ? (event) => {
        const el = event.currentTarget;
        if (el.scrollHeight - el.scrollTop - el.clientHeight < el.clientHeight * 2) onEndReached();
      } : undefined}
      onKeyDown={handleKeyDown}
      className="h-full overflow-y-auto scroll-area focus-visible:outline-offset-[-2px]"
    >
      <div className="relative w-full" style={{ height: virtualizer.getTotalSize() }}>
        {virtualizer.getVirtualItems().map((virtualRow) => {
          const row = rows[virtualRow.index];
          if (!row) return null;
          return (
            <div
              key={virtualRow.key}
              role="row"
              aria-rowindex={virtualRow.index + 1}
              /*
               * 宽高都是算出来的，所以可以完全封装：浏览器不需要再去量这一块的布局。
               * 无尺寸封装时，每滚一次就要重做一遍整棵 DOM 的布局。
               *
               * absolute top-0 left-0 是必须的，不是样式洁癖：虚拟滚动把每行的
               * 位置算成一个 translateY 偏移量，前提是这一行已经脱离正常流。
               * 只有 transform 而没有 absolute，行既在流里占着自己的高度、
               * 又被整体下移 start，两份偏移叠加——行与行之间就会裂出一段
               * 和行高同量级的空白（网格模式下「中间有大段空白」的成因）。
               */
              style={{
                transform: `translateY(${virtualRow.start}px)`,
                height: row.height,
                gap: GAP,
                width: row.width,
                contain: "layout paint style",
              }}
              className="absolute top-0 left-0 flex"
            >
              {row.cells.map((cell) => (
                <Tile
                  key={cell.data.asset.id}
                  gridItem={cell.data}
                  cellWidth={cell.width}
                  cellHeight={cell.height}
                  isSelected={selected.has(cell.data.asset.id)}
                  isCursor={cursorId === cell.data.asset.id}
                  showFilename={showFilename}
                  markMode={markMode}
                  onActivate={(event) => {
                    const id = cell.data.asset.id;
                    if (event.shiftKey && cursorId !== null) {
                      ui.selectRange(cursorId, id, orderedIds);
                      return;
                    }
                    // 标记模式下单击就是打标记而不是换掉整个选区,
                    // 网格和照片墙在这里必须一致,否则用户学到的规则会随视图变化。
                    if (markMode || event.ctrlKey || event.metaKey) {
                      ui.toggleMark(id);
                      return;
                    }
                    ui.selectOnly(id);
                  }}
                  onDoubleClick={() => {
                    // 双击只对未开启标记模式的网格有意义——标记模式单击就是打标记,
                    // 双击会重复触发,反而打乱选区。这里只在未开启时弹 Lightbox。
                    if (!markMode && onActivateDoubleClick) {
                      onActivateDoubleClick(cell.data.asset);
                    }
                  }}
                />
              ))}
            </div>
          );
        })}
      </div>
    </div>
  );
}

/**
 * 格子的 props 全是基本类型（除了持久的 `gridItem` 对象），所以 memo 能真的阶梯消掉滚动的重绘。
 *
 * 之前没有包装，虚拟化每改一次行就把视口内所有格子重建一遍——
 * 而每格里都有一个 `<img>` 丏几个条件输出的子栈，几十张就是几千个元素。
 */
interface TileProps {
  gridItem: GridItem;
  cellWidth: number;
  cellHeight: number;
  isSelected: boolean;
  isCursor: boolean;
  showFilename: boolean;
  markMode: boolean;
  onActivate(event: React.MouseEvent<HTMLButtonElement>): void;
  onDoubleClick?(): void;
}

const Tile = memo(function Tile({
  gridItem,
  cellWidth,
  cellHeight,
  isSelected,
  isCursor,
  showFilename,
  markMode,
  onActivate,
  onDoubleClick,
}: TileProps) {
  const { asset, hasDimensions } = gridItem;
  // RAW 与 HEIC 在 libvips 管线就位前无法解码，宿主会返回 404。
  // 这两种情况都渲染成占位，而不是让浏览器显示破图。
  const [decodeFailed, setDecodeFailed] = useState(false);
  const showPlaceholder = !hasDimensions || decodeFailed;

  return (
    <button
      type="button"
      role="gridcell"
      aria-selected={isSelected}
      aria-label={`${asset.fileName}${asset.rating > 0 ? `,${asset.rating} 星` : ""}`}
      tabIndex={-1}
      onClick={onActivate}
      onDoubleClick={onDoubleClick}
      className={[
        "group relative block shrink-0 overflow-hidden rounded-xs bg-sunken",
        "transition-shadow duration-(--lf-duration-fast) ease-(--lf-ease)",
        isSelected
          ? "shadow-[0_0_0_2px_var(--lf-accent)]"
          : "shadow-[0_0_0_1px_var(--lf-border)]",
        isCursor ? "outline-2 outline-offset-1 outline-(--lf-focus)" : "outline-none",
      ].join(" ")}
      // 同理：格子的宽高是传进来的数字，不需要重新测量。
      style={{ width: cellWidth, height: cellHeight, contain: "layout paint style" }}
    >
      {showPlaceholder ? (
        <span className="flex h-full w-full flex-col items-center justify-center gap-1.5 text-ink-3">
          <FileImage size={18} />
          <span className="max-w-[90%] truncate px-2 text-2xs">{asset.fileName}</span>
        </span>
      ) : (
        <img
          // 按格子的实际宽度分档请求，而不是固定一个尺寸：
          // 格子宽度是算出来的，写死尺寸会让一半的格子下载用不上的分辨率。
          src={thumbSrcFor(asset.thumbUrl, Math.max(cellWidth, cellHeight))}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onError={() => setDecodeFailed(true)}
          className="thumb-enter h-full w-full object-cover"
        />
      )}

      {/* 标记模式才显示勾选角标；浏览模式的选中状态只由边框表达。 */}
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

      {/*
        动图角标。缩略图是静止的第一帧，不给这个提示的话用户会以为
        这张图本来就是不动的。位置挑右上角：左上角归旗标/星级，
        右下角归标记模式的勾选，底部整条归悬停时的文件名。
      */}
      {hasTag(asset.tags, TAG_ANIMATED) ? (
        <span
          aria-hidden="true"
          className="absolute top-1.5 right-1.5 rounded-xs bg-black/65 px-1 py-px text-[10px] leading-tight font-medium text-white"
        >
          动图
        </span>
      ) : null}

      {markMode && !isSelected ? (
        // 和照片墙一样：未标记的格子也要让用户知道单击是加标记。
        <span
          aria-hidden="true"
          className="absolute inset-0 grid place-items-center bg-black/0 opacity-0 transition-opacity group-hover:bg-black/25 group-hover:opacity-100"
        >
          <span className="flex h-6 w-6 items-center justify-center rounded-full border-2 border-white/90">
            <Check size={12} weight="bold" className="text-white" />
          </span>
        </span>
      ) : null}

      {showFilename && !showPlaceholder ? (
        <span className="pointer-events-none absolute inset-x-0 bottom-0 truncate bg-linear-to-t from-black/80 to-transparent px-2 pt-5 pb-1.5 text-2xs text-white opacity-0 transition-opacity duration-(--lf-duration-fast) group-hover:opacity-100">
          {asset.fileName}
        </span>
      ) : null}
    </button>
  );
});

/**
 * 把缩略图地址里的宽度换成分档后的值。
 *
 * 真实图库走 `lumen://localhost/thumb/<id>?w=`，替换即可；
 * 演示图库是 picsum，本来就没有宽度参数，保持原样。
 */
function withWidth(url: string, pixels: number): string {
  const width = thumbWidth(Math.ceil(pixels * devicePixelRatio));
  return /[?&]w=\d+/.test(url)
    ? url.replace(/([?&]w=)\d+/, `$1${width}`)
    : url;
}

/**
 * 缩略图对象的内容。目前的格子每次渲染都调用 `withWidth` 重新算一次
 * 宽度档位并重新拼字符串——虽然结果一致、React 会忽略它，但这是每格一次的
 * 正则执行。这里把结果缓存起来，并且在宽度档位没变时直接返回。
 */
const thumbSrcCache = new Map<string, { pixels: number; src: string }>();

function thumbSrcFor(url: string, pixels: number): string {
  const cached = thumbSrcCache.get(url);
  if (cached && cached.pixels === pixels) return cached.src;
  const src = withWidth(url, pixels);
  thumbSrcCache.set(url, { pixels, src });
  return src;
}
