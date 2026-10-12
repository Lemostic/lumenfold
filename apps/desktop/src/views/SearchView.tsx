import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { MagnifyingGlass, Sparkle, X } from "@phosphor-icons/react";
import { JustifiedGrid } from "./JustifiedGrid";
import { EmptyState, ErrorState, SkeletonGrid } from "../components/States";
import { Lightbox } from "../components/Lightbox";
import { Segmented } from "../components/Segmented";
import { useSearch } from "../state/library";
import { useElementWidth } from "../lib/useElementWidth";
import { ui, useUi } from "../state/ui";
import type { DemoAsset } from "../lib/fixtures";

const PAGE_SIZE = 2000;

/** 结果态：贴顶的紧凑条。 */
const BAR_COMPACT =
  "flex h-14 shrink-0 items-center gap-3 border-b border-line bg-surface px-6";
/** 进入态：居中的大搜索框。 */
const BAR_HERO =
  "chrome-popover mx-auto flex h-14 w-full max-w-[640px] shrink-0 items-center gap-3 rounded-xl border border-line px-4 transition-shadow focus-within:border-line-strong focus-within:shadow-pop";

const MODE_OPTIONS = [
  { value: "text" as const, label: "文本" },
  { value: "semantic" as const, label: "语义" },
];

/**
 * 搜索视图。
 *
 * 进入态是居中的大搜索框（类似搜索引擎首页），有词之后收成顶部窄条、下方铺结果。
 *
 * # 为什么整页只有一个 `<input>`
 *
 * 之前这里是「有词就 return 结果态、没词就 return 进入态」，两个分支各自渲染了
 * 一个输入框。后果在中文输入法下立刻显形：第一次按键就把正在拼字的那个 input
 * 卸载重挂，输入法状态跟着没了，第一段候选被当成已提交文本上屏——
 * 用户看到的就是「输入中文，第一个字母变成英文」。
 *
 * 所以现在三个区块的 DOM 位置**固定**：进入态的介绍文案、搜索条、进入态的其余内容。
 * 切换形态只改 `className`（`hidden` / 布局类），不增删节点，输入框因此从头到尾
 * 是同一个元素。结果区在输入框**之后**，它增删不影响输入框的位置。
 */
export function SearchView() {
  const query = useUi((s) => s.searchQuery);
  const mode = useUi((s) => s.searchMode);
  const rowHeight = useUi((s) => s.rowHeight);
  const inputRef = useRef<HTMLInputElement>(null);

  /**
   * 真正送去检索的那个词。
   *
   * 和 `query` 分开是为了输入法：拼字过程中 `query` 会一路变成 `n` / `ni` / `nih`，
   * 那是拼音字母，不是用户要搜的东西。每个中间串都发一次请求，既白跑几趟，
   * 又会让结果区在用户还没选完候选词之前先闪一次「没有匹配」。
   */
  const [term, setTerm] = useState(query);
  /** 给渲染用的拼字状态；`composing` 那个 ref 给事件处理用，避免闭包读到旧值。 */
  const [isComposing, setIsComposing] = useState(false);
  const composing = useRef(false);

  const results = useSearch(term, mode);
  const { ref, width } = useElementWidth<HTMLDivElement>();

  const assets: DemoAsset[] =
    results.status === "ready" ? results.value.map((r) => r.asset) : [];

  const hasQuery = query.trim().length > 0;
  const hasTerm = term.trim().length > 0;

  /**
   * 清除按钮、斜杠聚焦、从别的视图切回来——这些是**外部**改动，要同步到待检索词。
   *
   * 拼字过程中不同步：输入法自己在改 `query`，那一步不该被当成一次真的检索。
   */
  useEffect(() => {
    if (!composing.current) setTerm(query);
  }, [query]);

  const handleChange = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    const value = event.currentTarget.value;
    // 值始终跟着输入框走：受控 input 在拼字期间落后于 DOM 会把候选字吞掉。
    ui.setSearchQuery(value);
    if (!composing.current) setTerm(value);
  }, []);

  const handleCompositionStart = useCallback(() => {
    composing.current = true;
    setIsComposing(true);
  }, []);

  const handleCompositionEnd = useCallback(
    (event: React.CompositionEvent<HTMLInputElement>) => {
      composing.current = false;
      setIsComposing(false);
      // 候选词落定，这才是用户要搜的东西。
      setTerm(event.currentTarget.value);
    },
    [],
  );

  const lightboxId = useUi((s) => (s.selected.size === 1 ? [...s.selected][0] : null));
  const lightboxAsset = useMemo(() => {
    if (lightboxId === null) return null;
    return assets.find((a) => a.id === lightboxId) ?? null;
  }, [lightboxId, assets]);
  const lightboxNeighbors = useMemo(() => {
    if (lightboxAsset === null) return { prev: null, next: null };
    const index = assets.findIndex((a) => a.id === lightboxAsset.id);
    if (index < 0) return { prev: null, next: null };
    return {
      prev: index > 0 ? assets[index - 1] ?? null : null,
      next: index < assets.length - 1 ? assets[index + 1] ?? null : null,
    };
  }, [lightboxAsset, assets]);

  const onActivateDoubleClick = useCallback((asset: DemoAsset) => {
    ui.selectOnly(asset.id);
    ui.moveCursor(asset.id);
  }, []);

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* 区块 1：进入态的介绍文案。空词时占上半屏，有词时只是隐藏——不卸载。 */}
      <div
        className={
          hasQuery
            ? "hidden"
            : "flex min-h-0 flex-1 flex-col items-center justify-end px-6"
        }
      >
        <div className="flex flex-col items-center gap-2">
          <div className="flex items-center gap-2 text-accent">
            <svg viewBox="0 0 16 16" width="20" height="20" aria-hidden="true">
              <path d="M1 13.5 13.5 1" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
              <path
                d="M8.5 13.5 13.5 8.5"
                stroke="currentColor"
                strokeWidth="2"
                strokeLinecap="round"
                opacity="0.55"
              />
            </svg>
            <span className="text-xl font-medium tracking-tight text-ink">Lumenfold</span>
          </div>
          <h1 className="mt-2 text-2xl font-semibold tracking-tight text-ink">搜索你的照片</h1>
          <p className="text-sm text-ink-2 text-pretty">按文件名与路径查找。语义检索需要先建好向量索引。</p>
        </div>
      </div>

      {/* 区块 2：唯一的输入框，两种形态共用同一个 DOM 节点。 */}
      <div className={hasQuery ? BAR_COMPACT : BAR_HERO}>
        <MagnifyingGlass size={18} className="shrink-0 text-ink-3" />
        <input
          ref={inputRef}
          type="search"
          value={query}
          onChange={handleChange}
          onCompositionStart={handleCompositionStart}
          onCompositionEnd={handleCompositionEnd}
          placeholder={hasQuery ? "搜索文件名" : "比如：IMG_4821 / 海边"}
          aria-label="搜索照片"
          autoFocus
          className="min-w-0 flex-1 bg-transparent text-md text-ink placeholder:text-ink-3 focus-visible:outline-none"
        />
        {hasQuery ? (
          <Segmented
            label="检索方式"
            value={mode}
            onChange={(next) => ui.setSearchMode(next)}
            options={MODE_OPTIONS}
          />
        ) : null}
        {query ? (
          <button
            type="button"
            aria-label="清除搜索"
            onClick={() => ui.setSearchQuery("")}
            className="inline-flex h-7 w-7 shrink-0 items-center justify-center rounded-md text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink"
          >
            <X size={16} />
          </button>
        ) : null}
      </div>

      {/* 区块 3：进入态的其余内容（模式切换与两张说明卡）。 */}
      <div
        className={
          hasQuery
            ? "hidden"
            : "flex min-h-0 flex-1 flex-col items-center px-6 pt-4"
        }
      >
        <Segmented
          label="检索方式"
          value={mode}
          onChange={(next) => ui.setSearchMode(next)}
          options={MODE_OPTIONS}
        />
        <ul className="mt-8 grid w-full max-w-[640px] grid-cols-2 gap-3 text-xs text-ink-2">
          <li className="rounded-md border border-line bg-sunken p-3">
            <p className="font-medium text-ink">文本检索</p>
            <p className="mt-1 leading-relaxed text-ink-3 text-pretty">
              查文件名与所在路径，不联网。关键词与说明文字要等元数据库建好之后才加入。
            </p>
          </li>
          <li className="rounded-md border border-line bg-sunken p-3">
            <p className="font-medium text-ink">语义检索</p>
            <p className="mt-1 leading-relaxed text-ink-3 text-pretty">
              描述画面内容，由本地 CLIP 编码后在向量索引里找相似照片。
            </p>
          </li>
        </ul>
      </div>

      {/* 区块 4：结果。在输入框之后，增删不影响输入框。 */}
      {hasQuery ? (
        <div ref={ref} className="min-h-0 flex-1 overflow-hidden p-3">
          {/*
            语义模式先判：它需要向量索引，宿主的拒绝里带着技术措辞（"no library index yet"），
            那不是给用户看的。这里换成一句说清楚「缺什么、什么时候有」的话。
          */}
          {mode === "semantic" ? (
            <SemanticPending term={term} />
          ) : results.status === "loading" ? (
            <SkeletonGrid
              columns={Math.max(1, Math.floor(width / (rowHeight * 1.4)))}
              rowHeight={rowHeight}
              gap={3}
              viewportHeight={560}
            />
          ) : results.status === "error" ? (
            <ErrorState
              title="检索失败"
              description="宿主在执行检索时返回了错误。"
              detail={results.message}
              onRetry={results.reload}
            />
          ) : isComposing || !hasTerm ? (
            // 输入法还在拼字：`term` 里还是上一次的词，此刻说「没有匹配」是在撒谎。
            <EmptyState
              icon={<MagnifyingGlass size={22} />}
              title="输入关键词开始检索"
              description="正在用输入法拼字，选完候选词就会开始查。"
            />
          ) : assets.length === 0 ? (
            <EmptyState
              icon={<MagnifyingGlass size={22} />}
              title={`没有匹配「${term}」的照片`}
              description="文本检索比的是文件名与所在路径，不看画面内容。想按画面找，切到语义模式。"
            />
          ) : (
            <div className="flex h-full min-h-0 flex-col">
              <p className="tabular shrink-0 pb-2 text-xs text-ink-3">
                命中 <span className="text-ink-2">{assets.length.toLocaleString("zh-CN")}</span> 项
              </p>
              <div className="min-h-0 flex-1">
                <JustifiedGrid
                  assets={assets.slice(0, PAGE_SIZE)}
                  width={width}
                  rowHeight={rowHeight}
                  showFilename
                  emptyState={null}
                  onActivateDoubleClick={onActivateDoubleClick}
                />
              </div>
            </div>
          )}
        </div>
      ) : null}

      <Lightbox
        asset={lightboxAsset}
        prev={lightboxNeighbors.prev}
        next={lightboxNeighbors.next}
        onClose={() => ui.clearSelection()}
        onPrev={() => {
          if (lightboxAsset === null) return;
          const index = assets.findIndex((a) => a.id === lightboxAsset.id);
          const prevAsset = index >= 0 ? assets[index - 1] : undefined;
          if (prevAsset) {
            ui.selectOnly(prevAsset.id);
            ui.moveCursor(prevAsset.id);
          }
        }}
        onNext={() => {
          if (lightboxAsset === null) return;
          const index = assets.findIndex((a) => a.id === lightboxAsset.id);
          const nextAsset = index >= 0 ? assets[index + 1] : undefined;
          if (nextAsset) {
            ui.selectOnly(nextAsset.id);
            ui.moveCursor(nextAsset.id);
          }
        }}
      />
    </div>
  );
}

/**
 * 语义检索未就绪时的如实说明。
 *
 * 这里不编造相似度分数，也不把宿主的英文技术错误直接摊给用户：
 * 缺的是一份 CLIP 向量索引，说清楚这一件事就够了。
 */
function SemanticPending({ term }: { term: string }) {
  return (
    <EmptyState
      icon={<Sparkle size={22} />}
      title="语义检索还没有可检索的向量索引"
      description={
        term.trim().length === 0
          ? "语义检索要先用 CLIP 把整库照片编码成向量。没有向量索引时给出相似度分数只能是编造，所以这里如实说明状态。"
          : `「${term}」需要先用 CLIP 编码整库照片。没有向量索引时给出相似度分数只能是编造，所以这里如实说明状态而不是显示假结果。`
      }
    />
  );
}
