import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useVirtualizer } from "@tanstack/react-virtual";
import { ArrowLeft, ArrowRight, Check, FileImage, Flag, Star } from "@phosphor-icons/react";
import { originalUrlFor, thumbWidth } from "../state/library";
import { TAG_ANIMATED, hasTag } from "../lib/types";
import { ui, useUi } from "../state/ui";
import type { DemoAsset } from "../lib/fixtures";

/**
 * 照片墙(单图大图模式)。
 *
 * 问题 2:之前的「照片墙」是网格状小图,用来一屏看个大概。
 * 现在改成「中间一张最大化原图 + 下方一条横向缩略图流」:
 * - 主区域是当前照片的原图,保持比例、垂直水平居中
 * - 下面 80px 高的横向流显示当前照片前后各若干张,左右方向键切换
 * - 预加载:进入页面或切换时,并行预加载当前照片前后各 PRELOAD_RADIUS 张原图,
 *   浏览器原生 new Image() 不阻塞主线程,切换时几乎是即时的
 *
 * 设计决策:不再保留原来的 contact sheet 网格模式——它和网格(grid)模式重合度太高,
 * 而单图大图模式是当前需要的核心体验。
 */

const PRELOAD_RADIUS = 10;
const STREAM_HEIGHT = 144;
const STREAM_GAP = 8;
const STREAM_THUMB_HEIGHT = 120;
const STREAM_THUMB_MIN_WIDTH = 72;
const STREAM_THUMB_MAX_WIDTH = 224;
const STREAM_THUMB_FALLBACK_WIDTH = 160;
const STREAM_THUMB_OVERSCAN = 4;

/**
 * 流条里一张缩略图的宽度。
 *
 * 这里原本是固定 120×120 的方框配 `object-contain`。3:2 的照片塞进方框后
 * 上下各留一条 20px 的背景色——用户看到的「每个图片下方有黑边」就是它,
 * 而且照片本身只占了方框中间那一条。
 *
 * 改成宽度跟着照片比例走:方框和照片同形,`object-cover` 既不用裁也不用留边。
 * 上下限是给全景和极端竖幅兜底的,免得一张 1:6 的全景把整条流撑爆。
 */
function streamThumbWidth(asset: DemoAsset | undefined): number {
  if (!asset || asset.width <= 0 || asset.height <= 0) return STREAM_THUMB_FALLBACK_WIDTH;
  const scaled = Math.round((STREAM_THUMB_HEIGHT * asset.width) / asset.height);
  return Math.min(STREAM_THUMB_MAX_WIDTH, Math.max(STREAM_THUMB_MIN_WIDTH, scaled));
}

/**
 * 原图地址。
 *
 * 真实图库走 `lumen://localhost/full/<id>` 经 wry 拦截返回解码后的原图;
 * 演示图库走 picsum,作为最大档位预览。demo 只用来评估布局。
 */
function fullUrlFor(asset: DemoAsset): string {
  return originalUrlFor(asset);
}

/**
 * 异步预加载一张图。浏览器原生 new Image() 在后台解码,不阻塞主线程。
 * 完成或失败都 resolve,Promise 自身不会被外部 await,只是用来调度。
 */
function preloadImage(url: string): void {
  const img = new Image();
  img.decoding = "async";
  img.loading = "eager";
  img.src = url;
  // 不监听 onload / onerror:失败由浏览器丢弃,UI 端另算;成功则是浏览器已就绪。
  // 用 img.decode() 可以更精确,但会强制阻塞到当前帧,反而不利于「下一张切立刻可见」。
}

/**
 * 给定当前下标和全集,生成要预加载的下标序列(前 PRELOAD_RADIUS 到后 PRELOAD_RADIUS)。
 * 出去越界的、当前的。
 */
function preloadRange(center: number, total: number): number[] {
  const indices: number[] = [];
  for (let offset = -PRELOAD_RADIUS; offset <= PRELOAD_RADIUS; offset += 1) {
    if (offset === 0) continue;
    const next = center + offset;
    if (next < 0 || next >= total) continue;
    indices.push(next);
  }
  return indices;
}

interface WallViewProps {
  assets: DemoAsset[];
  width: number;
  height: number;
  emptyState: React.ReactNode;
  onEndReached?(): void;
  /**
   * 双击主区域触发。LibraryView 用它来弹 Lightbox——光标和选中由调用方决定，
   * 这里只负责「用户说了想看大图」这一信号。
   */
  onActivate?(asset: DemoAsset): void;
}

export function WallView({ assets, width, height, emptyState, onEndReached, onActivate }: WallViewProps) {
  const selected = useUi((s) => s.selected);
  const cursorId = useUi((s) => s.cursorId);
  const markMode = useUi((s) => s.markMode);
  const scrollRef = useRef<HTMLDivElement>(null);
  const streamScrollRef = useRef<HTMLDivElement>(null);

  useEffect(() => {
    scrollRef.current?.focus();
  }, []);

  // 主体下标:优先取光标,光标不在集合里就退回第一条。
  const centerIndex = useMemo(() => {
    if (assets.length === 0) return 0;
    if (cursorId !== null) {
      const index = assets.findIndex((a) => a.id === cursorId);
      if (index >= 0) return index;
    }
    return 0;
  }, [assets, cursorId]);

  const center = assets[centerIndex];

  // 预加载当前主体前后的原图。依赖 centerIndex 变化,而不是 center 对象,
  // 切换同一张图的不同元数据(比如评分)时不该重新请求同样的 URL。
  useEffect(() => {
    if (assets.length === 0) return;
    const indices = preloadRange(centerIndex, assets.length);
    for (const index of indices) {
      const target = assets[index];
      if (target) preloadImage(fullUrlFor(target));
    }
  }, [assets, centerIndex]);

  // 切换到当前主体后,横向流里把当前项滚到视野正中。
  useEffect(() => {
    const el = streamScrollRef.current;
    if (!el) return;
    const target = el.querySelector<HTMLElement>(`[data-stream-index="${centerIndex}"]`);
    if (!target) return;
    const targetCenter = target.offsetLeft + target.offsetWidth / 2;
    const elCenter = el.clientWidth / 2;
    el.scrollTo({ left: targetCenter - elCenter, behavior: "smooth" });
  }, [centerIndex]);

  const move = useCallback(
    (delta: number) => {
      if (assets.length === 0) return;
      const next = Math.min(Math.max(centerIndex + delta, 0), assets.length - 1);
      if (next === centerIndex) return;
      const target = assets[next];
      if (target) ui.moveCursor(target.id);
    },
    [assets, centerIndex],
  );

  /**
   * 全局快捷键:左右方向键切照片,必须避开输入框。
   *
   * 把键盘事件挂到 div 上而不是 window,可以避开全局搜索快捷键的冲突;
   * 但 div 必须在视图里获得焦点。这里用 tabIndex=0 自动聚焦。
   */
  const handleKeyDown = useCallback(
    (event: React.KeyboardEvent<HTMLDivElement>) => {
      if (event.ctrlKey || event.metaKey) {
        if (event.key === "a") {
          event.preventDefault();
          ui.selectAll(assets.map((a) => a.id));
          return;
        }
        if (event.key === "i") {
          event.preventDefault();
          ui.invertSelection(assets.map((a) => a.id));
          return;
        }
      }
      if (event.key === "ArrowRight") {
        event.preventDefault();
        move(1);
        return;
      }
      if (event.key === "ArrowLeft") {
        event.preventDefault();
        move(-1);
        return;
      }
      if (event.key === "Escape") {
        event.preventDefault();
        ui.clearSelection();
      }
    },
    [assets, move],
  );

  if (assets.length === 0) return <>{emptyState}</>;

  // 主区域高度:总高度减去横向流条。
  const mainHeight = Math.max(0, height - STREAM_HEIGHT);

  return (
    <div
      ref={scrollRef}
      role="grid"
      aria-label="照片墙"
      aria-rowcount={assets.length}
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
      <div className="flex w-full flex-col" style={{ minHeight: height }}>
        <MainStage
          asset={center ?? null}
          width={width}
          height={mainHeight}
          onActivate={center && onActivate ? () => onActivate(center) : undefined}
        />

        <Stream
          assets={assets}
          centerIndex={centerIndex}
          selected={selected}
          markMode={markMode}
          onSelect={(id, additive) => {
            if (additive || markMode) {
              ui.toggleMark(id);
            } else {
              ui.selectOnly(id);
            }
            ui.moveCursor(id);
          }}
          onJump={(index) => {
            const target = assets[index];
            if (target) ui.moveCursor(target.id);
          }}
          scrollRef={streamScrollRef}
        />
      </div>
    </div>
  );
}

interface MainStageProps {
  asset: DemoAsset | null;
  width: number;
  height: number;
  onActivate?(): void;
}

function MainStage({ asset, width, height, onActivate }: MainStageProps) {
  const stageRef = useRef<HTMLDivElement>(null);
  const [decodeFailed, setDecodeFailed] = useState(false);
  const [zoom, setZoom] = useState(1);
  useEffect(() => setZoom(1), [asset?.id]);
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const stopNativeWheel = (event: WheelEvent) => { if (!event.ctrlKey) event.preventDefault(); };
    el.addEventListener("wheel", stopNativeWheel, { passive: false });
    return () => el.removeEventListener("wheel", stopNativeWheel);
  }, []);
  const showPlaceholder = asset === null || asset.width === 0 || asset.height === 0 || decodeFailed;

  return (
    <div
      ref={stageRef}
      className="relative flex shrink-0 items-center justify-center overflow-hidden bg-sunken"
      style={{ width, height }}
      onWheel={(event) => {
        if (showPlaceholder || event.ctrlKey) return;
        event.preventDefault();
        setZoom((current) => Math.min(8, Math.max(1, Number((current * (event.deltaY < 0 ? 1.15 : 1 / 1.15)).toFixed(2)))));
      }}
      aria-label={asset ? `当前照片 ${asset.fileName}` : "没有可显示的照片"}
      // 双击主区域 = 想看大图。点击 vs 双击不会打架：单击是别的层级的事，
      // MainStage 自己没绑 onClick，所有命中都会被上层（看照片、批选）接走。
      onDoubleClick={() => onActivate?.()}
    >
      {showPlaceholder || asset === null ? (
        <div className="flex flex-col items-center gap-2 text-ink-3">
          <FileImage size={36} weight="regular" />
          <span className="text-xs">{asset === null ? "没有可显示的照片" : asset.fileName}</span>
        </div>
      ) : (
        <img
          key={asset.id}
          src={fullUrlFor(asset)}
          alt={asset.fileName}
          loading="eager"
          decoding="async"
          draggable={false}
          onError={() => setDecodeFailed(true)}
          className="thumb-enter max-h-full max-w-full object-contain"
          style={{ maxWidth: width, maxHeight: height, transform: `scale(${zoom})` }}
        />
      )}

      {zoom !== 1 ? (
        <button type="button" onClick={() => setZoom(1)} aria-label="恢复适应窗口大小" className="absolute top-3 left-3 rounded-md bg-popover px-2 py-1 text-xs tabular-nums text-ink shadow-pop">
          {Math.round(zoom * 100)}% · 重置
        </button>
      ) : null}
      {asset ? (
        <div className="pointer-events-none absolute inset-x-0 bottom-0 flex items-end justify-between gap-3 bg-gradient-to-t from-black/40 to-transparent px-4 pt-8 pb-3 text-white">
          <div className="min-w-0">
            <p className="truncate text-sm font-medium" title={asset.fileName}>
              {asset.fileName}
            </p>
            <p className="tabular mt-0.5 text-2xs opacity-80">
              {asset.width} × {asset.height}
            </p>
          </div>
          <p className="tabular shrink-0 text-2xs opacity-80">
            {/* 在视图里直接展示「第几张 / 共几张」,让用户随时知道下一张要去哪 */}
          </p>
        </div>
      ) : null}

      {/*
        动图角标。主区域显示的是解码后的第一帧，没有这个提示的话
        用户会以为这张照片本来就是静止的。
      */}
      {asset && hasTag(asset.tags, TAG_ANIMATED) ? (
        <span
          aria-hidden="true"
          className="absolute top-3 right-3 rounded-xs bg-black/65 px-1.5 py-0.5 text-2xs font-medium text-white"
        >
          动图
        </span>
      ) : null}
    </div>
  );
}

interface StreamProps {
  assets: DemoAsset[];
  centerIndex: number;
  selected: ReadonlySet<string>;
  markMode: boolean;
  onSelect(id: string, additive: boolean): void;
  onJump(index: number): void;
  scrollRef: React.RefObject<HTMLDivElement>;
}

function Stream({ assets, centerIndex, selected, markMode, onSelect, onJump, scrollRef }: StreamProps) {
  const virtualizer = useVirtualizer({
    count: assets.length,
    getScrollElement: () => scrollRef.current,
    // 宽度是 asset 的纯函数,所以估算值就是精确值,不需要 measureElement 回测 DOM。
    estimateSize: (index) => streamThumbWidth(assets[index]) + STREAM_GAP,
    // 尺寸按 key 缓存(@tanstack 的 itemSizeCache 就是拿 getItemKey 当键),
    // 同一个位置换了照片必须重新取尺寸,否则滚动偏移会按上一批照片算。
    getItemKey: (index) => assets[index]?.id ?? index,
    horizontal: true,
    overscan: STREAM_THUMB_OVERSCAN,
  });

  const items = virtualizer.getVirtualItems();

  return (
    <div className="relative shrink-0 border-t border-line bg-surface" style={{ height: STREAM_HEIGHT }}>
      <button
        type="button"
        aria-label="上一张"
        title="上一张(←)"
        onClick={() => onJump(Math.max(0, centerIndex - 1))}
        className="absolute top-1/2 left-2 z-10 -translate-y-1/2 flex h-7 w-7 items-center justify-center rounded-full bg-popover text-ink shadow-pop transition-colors hover:text-accent"
      >
        <ArrowLeft size={14} />
      </button>
      <button
        type="button"
        aria-label="下一张"
        title="下一张(→)"
        onClick={() => onJump(Math.min(assets.length - 1, centerIndex + 1))}
        className="absolute top-1/2 right-2 z-10 -translate-y-1/2 flex h-7 w-7 items-center justify-center rounded-full bg-popover text-ink shadow-pop transition-colors hover:text-accent"
      >
        <ArrowRight size={14} />
      </button>

      <div
        ref={scrollRef}
        className="scroll-area h-full overflow-x-auto overflow-y-hidden"
      >
        <div className="relative h-full" style={{ width: virtualizer.getTotalSize() }}>
          {items.map((virtualItem) => {
            const asset = assets[virtualItem.index];
            if (!asset) return null;
            return (
              <StreamThumb
                key={asset.id}
                asset={asset}
                index={virtualItem.index}
                offset={virtualItem.start}
                width={streamThumbWidth(asset)}
                isCenter={virtualItem.index === centerIndex}
                isSelected={selected.has(asset.id)}
                markMode={markMode}
                onClick={(additive) => onSelect(asset.id, additive)}
              />
            );
          })}
        </div>
      </div>
    </div>
  );
}

interface StreamThumbProps {
  asset: DemoAsset;
  index: number;
  offset: number;
  width: number;
  isCenter: boolean;
  isSelected: boolean;
  markMode: boolean;
  onClick(additive: boolean): void;
}

const StreamThumb = memo(function StreamThumb({
  asset,
  index,
  offset,
  width,
  isCenter,
  isSelected,
  markMode,
  onClick,
}: StreamThumbProps) {
  const [decodeFailed, setDecodeFailed] = useState(false);
  const showPlaceholder = (asset.width === 0 || asset.height === 0) || decodeFailed;

  return (
    <button
      type="button"
      role="gridcell"
      aria-selected={isSelected}
      aria-current={isCenter ? "true" : undefined}
      aria-label={`${asset.fileName}${isCenter ? ",当前" : ""}`}
      data-stream-index={index}
      onClick={(event) => {
        event.stopPropagation();
        onClick(event.shiftKey || event.ctrlKey || event.metaKey);
      }}
      className={[
        // 垂直居中只走下面那一个 `transform`。这里**不能**再写 Tailwind 的
        // `-translate-y-1/2`:v4 把它编译成 `translate` 属性,和行内的
        // `transform` 是两个属性、会叠加,于是整块被推高了一个自身高度,
        // 上半张被流条的 `overflow-y-hidden` 裁掉——「上方被截断了」就是它。
        "group absolute top-1/2 shrink-0 overflow-hidden rounded-xs bg-sunken",
        "transition-shadow duration-(--lf-duration-fast) ease-(--lf-ease)",
        isCenter
          ? "shadow-[0_0_0_2px_var(--lf-accent),0_2px_8px_var(--lf-accent-glow)]"
          : isSelected
            ? "shadow-[0_0_0_2px_var(--lf-accent)]"
            : "shadow-[0_0_0_1px_var(--lf-border)] hover:shadow-[0_0_0_1px_var(--lf-border-strong)]",
      ].join(" ")}
      style={{
        width,
        height: STREAM_THUMB_HEIGHT,
        transform: `translate(${offset}px, -50%)`,
        contain: "layout paint style",
      }}
    >
      {showPlaceholder ? (
        <span className="flex h-full w-full items-center justify-center text-ink-3">
          <FileImage size={14} />
        </span>
      ) : (
        <img
          src={withWidth(asset.thumbUrl, width)}
          alt=""
          loading="lazy"
          decoding="async"
          draggable={false}
          onError={() => setDecodeFailed(true)}
          // 框和照片同形 (`streamThumbWidth`),所以 cover 与 contain 结果一致;
          // 选 cover 只是为了在取整差一个像素时裁掉半分,而不是露出一条黑边。
          className="h-full w-full object-cover"
        />
      )}

      {markMode && isSelected ? (
        <span
          aria-hidden="true"
          className="absolute top-0.5 right-0.5 flex h-3.5 w-3.5 items-center justify-center rounded-full bg-accent text-accent-ink"
        >
          <Check size={9} weight="bold" />
        </span>
      ) : null}

      {!isSelected && asset.flagged ? (
        <span aria-hidden="true" className="absolute top-0.5 left-0.5 text-accent">
          <Flag size={10} weight="fill" />
        </span>
      ) : null}

      {!isSelected && asset.rating > 0 ? (
        <span aria-hidden="true" className="absolute top-0.5 left-0.5 text-accent">
          <Star size={10} weight="fill" />
        </span>
      ) : null}
    </button>
  );
});

/** 缩略图 URL 换成档位宽度,避免每次滚动重写。 */
function withWidth(url: string, pixels: number): string {
  const width = thumbWidth(Math.ceil(pixels * devicePixelRatio));
  return /[?&]w=\d+/.test(url) ? url.replace(/([?&]w=)\d+/, `$1${width}`) : url;
}
