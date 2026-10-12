import { memo, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { Images } from "@phosphor-icons/react";
import { JustifiedGrid } from "./JustifiedGrid";
import { WallView } from "./WallView";
import { EmptyState, ErrorState, SkeletonGrid } from "../components/States";
import { Button } from "../components/Button";
import { Lightbox } from "../components/Lightbox";
import { useElementWidth } from "../lib/useElementWidth";
import { useLibraryScanning, usePagedAssets } from "../state/library";
import { TAG_META, errorKind } from "../lib/types";
import { ui, useUi } from "../state/ui";
import type { DemoAsset } from "../lib/fixtures";

/** 宿主错误 → 面向用户的标题与说明。同一套文案在时间线与人物视图复用。 */
export function errorTitle(raw: unknown): string {
  switch (errorKind(raw)) {
    case "no_library":
      return "还没有打开图库";
    case "not_indexed":
      return "这个功能还没有索引支撑";
    case "unsupported":
      return "当前版本不支持这项查询";
    default:
      return "读取图库失败";
  }
}

export function errorDescription(raw: unknown): string {
  switch (errorKind(raw)) {
    case "no_library":
      return "选一个照片目录之后这个视图才会有内容。";
    case "not_indexed":
      return "人物聚类与语义检索依赖索引和模型,两者就位后这里会自动填充。";
    case "unsupported":
      return "宿主明确拒绝了这个请求,而不是没找到数据。下面是它给出的原因。";
    default:
      return "宿主返回了错误,下面是原始信息。";
  }
}

function LibraryViewImpl() {
  const sort = useUi((s) => s.sort);
  const personId = useUi((s) => s.personId);
  const rowHeight = useUi((s) => s.rowHeight);
  const layout = useUi((s) => s.layout);
  const tagFilter = useUi((s) => s.tagFilter);

  const assets = usePagedAssets({ sort, bucket: null, personId, tag: tagFilter });
  const { ref, width, height } = useElementWidth<HTMLDivElement>();

  // 问题 3:网格模式双击弹 Lightbox。Lightbox 是 LibraryView 级的浮层,
  // 与下面哪个视图都没有位置耦合,这样切换 grid/wall 时 Lightbox 不会被打断。
  const [lightboxId, setLightboxId] = useState<string | null>(null);

  /**
   * 扫描进行中时宿主每 400ms 就多一批记录,但排序可能因此整体错位:
   * 用户往下滚,新照片被排到前面,正在看的那一屏就被顶走了。
   * 所以扫描期间冻结这一页,扫描结束再一次性取最新的完整结果。
   *
   * 这里订阅的是 `useLibraryScanning()`（布尔量）而不是整份 stats：
   * 这个组件只用得到 `scanning` 这一个字段,订阅整份状态会让每 400ms 的
   * 进度发布都把它连同整屏网格一起重渲染一遍。
   */
  const scanning = useLibraryScanning();
  const seenScan = useRef(false);
  useEffect(() => {
    if (!seenScan.current) {
      seenScan.current = true;
      return;
    }
    if (!scanning) assets.reload();
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [scanning]);

  useEffect(() => {
    ui.setVisibleIds(assets.assets.map((asset) => asset.id));
  }, [assets.assets]);

  const lightboxAsset = useMemo(() => {
    if (lightboxId === null) return null;
    return assets.assets.find((a) => a.id === lightboxId) ?? null;
  }, [lightboxId, assets.assets]);

  const lightboxNeighbors = useMemo(() => {
    if (lightboxAsset === null) return { prev: null, next: null };
    const index = assets.assets.findIndex((a) => a.id === lightboxAsset.id);
    if (index < 0) return { prev: null, next: null };
    return {
      prev: index > 0 ? assets.assets[index - 1] ?? null : null,
      next: index < assets.assets.length - 1 ? assets.assets[index + 1] ?? null : null,
    };
  }, [lightboxAsset, assets.assets]);

  const onActivateDoubleClick = useCallback((asset: DemoAsset) => {
    setLightboxId(asset.id);
    ui.moveCursor(asset.id);
  }, []);

  const closeLightbox = useCallback(() => setLightboxId(null), []);

  const gotoNeighbor = useCallback(
    (offset: -1 | 1) => {
      if (lightboxAsset === null) return;
      const index = assets.assets.findIndex((a) => a.id === lightboxAsset.id);
      if (index < 0) return;
      const next = assets.assets[index + offset];
      if (!next) return;
      setLightboxId(next.id);
      ui.moveCursor(next.id);
    },
    [lightboxAsset, assets.assets],
  );

  const empty = useMemo(
    () =>
      tagFilter === null ? (
        <EmptyState
          icon={<Images size={22} />}
          title="这个图库里还没有照片"
          description="Lumenfold 只读取你指定的目录,不会搬动或复制原图。把照片放进图库根目录后,扫描任务会把它们登记进来。"
        />
      ) : (
        // 按类别筛空和「图库是空的」是两件事，共用一个空状态会让用户
        // 以为自己的照片没了。
        <EmptyState
          icon={<Images size={22} />}
          title={`没有「${TAG_META.find((entry) => entry.id === tagFilter)?.label ?? tagFilter}」这一类照片`}
          description="类别是扫描时按文件本身判出来的。如果刚升级过，分类可能还在后台补齐——上面类别栏里的计数会说明还有多少张没分过类。"
        />
      ),
    [tagFilter],
  );

  return (
    <div className="flex h-full min-h-0 flex-col">
      {personId !== null ? (
        <div className="flex h-9 shrink-0 items-center gap-2 border-b border-line bg-surface px-3">
          <span className="text-xs text-ink-2">已按人物过滤</span>
          <Button size="sm" variant="ghost" onClick={() => ui.openPerson(null)}>
            返回全部人物
          </Button>
        </div>
      ) : null}

      <div ref={ref} className="min-h-0 flex-1 overflow-hidden">
        {width === 0 ? null : assets.status === "loading" ? (
          <div className="p-3">
            <SkeletonGrid
              columns={Math.max(1, Math.floor(width / (rowHeight * 1.4)))}
              rowHeight={rowHeight}
              gap={3}
              viewportHeight={560}
            />
          </div>
        ) : assets.status === "error" ? (
          <ErrorState
            title={errorTitle(assets.raw)}
            description={errorDescription(assets.raw)}
            detail={assets.message}
            onRetry={assets.reload}
          />
        ) : layout === "wall" ? (
          <WallView
            assets={assets.assets}
            width={width}
            height={height}
            onEndReached={assets.more ? assets.loadMore : undefined}
            emptyState={empty}
            onActivate={(asset) => {
              setLightboxId(asset.id);
              ui.moveCursor(asset.id);
            }}
          />
        ) : (
          <JustifiedGrid
            assets={assets.assets}
            width={width}
            rowHeight={rowHeight}
            showFilename={rowHeight >= 160}
            onEndReached={assets.more ? assets.loadMore : undefined}
            emptyState={empty}
            onActivateDoubleClick={onActivateDoubleClick}
          />
        )}
      </div>

      {/*
        问题 3:Lightbox 挂在 LibraryView 根部,与具体视图解耦。
        - 网格双击弹
        - WallView 的左右键切换等价于此处 Lightbox 的 prev/next 行为,
          这里把 WallView 的 prev/next 留给它自己的代码路径
        - 关闭时回到原视图,不重置 scroll
      */}
      <Lightbox
        asset={lightboxAsset}
        prev={lightboxNeighbors.prev}
        next={lightboxNeighbors.next}
        onClose={closeLightbox}
        onPrev={() => gotoNeighbor(-1)}
        onNext={() => gotoNeighbor(1)}
      />
    </div>
  );
}

/**
 * 用 `memo` 包一层。
 *
 * `AppShell` 在扫描期间每 400ms 重渲染一次（进度数字要动），而它把这几个视图
 * 当子节点直接渲染。不包的话，父组件的每一次重渲染都会连带把这个组件重跑一遍：
 * 一屏几百个格子跟着一个计时器空转，渲染线程被占着，后台进度反而更难显示出来。
 *
 * 这个组件没有任何 props，所以 `memo` 的浅比较永远相等——它只会因为**自己
 * 订阅的东西**（`useUi`、`usePagedAssets`、`useLibraryScanning`）变化而重渲染。
 */
export const LibraryView = memo(LibraryViewImpl);
