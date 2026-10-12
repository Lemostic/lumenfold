import { useCallback, useEffect, useMemo, useRef, useState, useSyncExternalStore } from "react";
import { bridge, describeError, hostAvailable } from "../lib/ipc";
import {
  DEMO_ASSETS,
  DEMO_METADATA,
  DEMO_MODELS,
  DEMO_PEOPLE,
  DEMO_STATS,
  type DemoAsset,
} from "../lib/fixtures";
import {
  TAG_META,
  hasTag,
  type AssetMetadata,
  type AssetQuery,
  type AssetSummary,
  type AssetTagCount,
  type BatchReport,
  type BatchRequest,
  type DiscoveryReport,
  type LibraryStats,
  type ModelEntry,
  type PersonSummary,
  type SearchMode,
  type SearchRequest,
} from "../lib/types";

/**
 * 数据源。Tauri 宿主可用时走命令，不可用时（浏览器里跑 pnpm dev）
 * 由演示图库接管。后者只为了评估布局，不参与发行。
 */
interface DataSource {
  kind: "host" | "demo";
  stats(): Promise<LibraryStats>;
  open(path: string): Promise<LibraryStats>;
  /** 用一组目录替换图库并重扫。增删改都走这一条。 */
  setRoots(roots: string[]): Promise<LibraryStats>;
  /** 自动发现含照片的目录。只发现、不采纳。 */
  discover(base: string, maxDepth: number): Promise<DiscoveryReport>;
  rescan(): Promise<LibraryStats>;
  cancelScan(): Promise<LibraryStats>;
  setThumbCacheLimit(limitBytes: number): Promise<LibraryStats>;
  clearThumbCache(): Promise<LibraryStats>;
  batchOperate(request: BatchRequest): Promise<BatchReport>;
  assets(query: AssetQuery): Promise<DemoAsset[]>;
  /** 每个类别当前有多少张，外加「已经分过类」的总数。 */
  tagCounts(): Promise<AssetTagCount[]>;
  metadata(assetId: string): Promise<AssetMetadata>;
  people(): Promise<PersonSummary[]>;
  search(req: SearchRequest): Promise<Array<{ asset: DemoAsset; score: number }>>;
  models(): Promise<ModelEntry[]>;
  /** `licenseAccepted` 必须为 true 才会真的发出请求，见 ADR-0002 §4。 */
  startDownload(modelId: string, licenseAccepted: boolean): Promise<ModelEntry>;
  cancelDownload(modelId: string): Promise<ModelEntry>;
}

function bySort(a: DemoAsset, b: DemoAsset, sort: AssetQuery["sort"]): number {
  switch (sort) {
    case "taken-asc":
      return (a.takenAt ?? 0) - (b.takenAt ?? 0);
    case "name":
      return a.fileName.localeCompare(b.fileName, "zh-CN");
    case "size":
      return b.bytes - a.bytes;
    case "rating":
      return b.rating - a.rating || (b.takenAt ?? 0) - (a.takenAt ?? 0);
    case "taken-desc":
    default:
      return (b.takenAt ?? 0) - (a.takenAt ?? 0);
  }
}

function demoSearch(req: SearchRequest): Array<{ asset: DemoAsset; score: number }> {
  const needle = req.query.trim().toLowerCase();
  if (needle.length === 0) return [];

  // 演示图库没有向量索引，语义模式如实返回空，而不是编造相似度分数。
  if (req.mode === "semantic") return [];

  const hits: Array<{ asset: DemoAsset; score: number }> = [];
  for (const asset of DEMO_ASSETS) {
    const metadata = DEMO_METADATA[asset.id];
    const inName = asset.fileName.toLowerCase().includes(needle);
    const inKeywords = metadata?.keywords.some((k) => k.toLowerCase().includes(needle)) ?? false;
    const inCaption = metadata?.caption?.toLowerCase().includes(needle) ?? false;
    if (!inName && !inKeywords && !inCaption) continue;
    // 关键词命中比文件名命中更接近用户意图，给一个确定性的排序权重
    const score = inKeywords ? 2 : inCaption ? 1.5 : 1;
    hits.push({ asset, score });
    if (hits.length >= req.limit) break;
  }
  return hits.sort(
    (a, b) => b.score - a.score || (b.asset.takenAt ?? 0) - (a.asset.takenAt ?? 0),
  );
}

const demoSource: DataSource = {
  kind: "demo",
  stats: async () => DEMO_STATS,
  open: async () => {
    throw new Error("演示图库是内置的固定数据，不能切换目录");
  },
  // 演示模式没有真实文件系统：如实报错，而不是回一个空列表让用户以为
  // 「这台机器上没有含照片的目录」。
  setRoots: async () => {
    throw new Error("演示图库是内置的固定数据，不能增删目录");
  },
  discover: async () => {
    throw new Error("演示模式没有真实的文件系统可以扫描");
  },
  rescan: async () => DEMO_STATS,
  // 演示图库是固定数据，既没有目录可扫也没有文件可动。如实报错，
  // 伪造一个「取消成功」会让状态栏显示一段根本没发生过的扫描。
  cancelScan: async () => {
    throw new Error("演示图库是内置的固定数据，没有真实的扫描可以取消");
  },
  // 演示数据没有真实的缩略图缓存，但可以「设上限」。不报错反而更糟：
  // 改一个不存在的上限，等于教用户相信这里存在一块可以管理的磁盘。
  setThumbCacheLimit: async (limitBytes) => ({
    ...DEMO_STATS,
    cacheLimitBytes: limitBytes,
  }),
  clearThumbCache: async () => {
    throw new Error("演示图库没有可清理的缩略图缓存");
  },
  batchOperate: async () => {
    throw new Error("演示图库是内置的固定数据，不会真的复制、移动或删除文件");
  },
  assets: async (query) => {
    let pool: DemoAsset[] = DEMO_ASSETS;
    if (query.personId) {
      pool = DEMO_ASSETS.filter((a) => DEMO_METADATA[a.id]?.keywords.includes("人像"));
    }
    return pool.sort((a, b) => bySort(a, b, query.sort)).slice(query.offset, query.offset + query.limit);
  },
  tagCounts: async () =>
    TAG_META.map((entry) => ({
      id: entry.id,
      count: DEMO_ASSETS.filter((asset) => hasTag(asset.tags, entry.bit)).length,
      // 演示数据每一条都带着现算出来的类别位，所以没有「还没分类」这回事。
      classified: DEMO_ASSETS.length,
      total: DEMO_ASSETS.length,
    })),
  metadata: async (assetId) => {
    const found = DEMO_METADATA[assetId];
    if (!found) throw new Error(`演示图库中没有 ${assetId}`);
    return found;
  },
  people: async () => DEMO_PEOPLE,
  search: async (req) => demoSearch(req),
  models: async () => DEMO_MODELS,
  // 演示模式只展示注册表。伪造下载进度比不提供按钮更糟，如实报错。
  startDownload: async () => {
    throw new Error("演示模式只展示模型注册表，不会真的下载权重");
  },
  cancelDownload: async () => {
    throw new Error("演示模式没有正在进行的下载");
  },
};

const hostSource: DataSource = {
  kind: "host",
  stats: () => bridge.libraryStats(),
  open: (path) => bridge.openLibrary(path),
  setRoots: (roots) => bridge.setLibraryRoots(roots),
  discover: (base, maxDepth) => bridge.discoverLibraries(base, maxDepth),
  rescan: () => bridge.rescan(),
  cancelScan: () => bridge.cancelScan(),
  setThumbCacheLimit: (limitBytes) => bridge.setThumbCacheLimit(limitBytes),
  clearThumbCache: () => bridge.clearThumbCache(),
  batchOperate: (request) => bridge.batchOperate(request),
  assets: async (query) => {
    const rows: AssetSummary[] = await bridge.queryAssets(query);
    // 真实缩略图走自定义协议，不经过 IPC，见 docs/ARCHITECTURE.md
    return rows.map((row) => ({ ...row, thumbUrl: thumbnailUrl(row, 384) }));
  },
  tagCounts: () => bridge.assetTagCounts(),
  metadata: (assetId) => bridge.assetMetadata(assetId),
  people: () => bridge.listPeople(),
  search: async (req) => {
    const rows = await bridge.search(req);
    return rows.map((row) => ({ ...row, asset: { ...row.asset, thumbUrl: thumbnailUrl(row.asset, 384) } }));
  },
  models: () => bridge.listModels(),
  startDownload: (modelId, licenseAccepted) => bridge.startModelDownload(modelId, licenseAccepted),
  cancelDownload: (modelId) => bridge.cancelModelDownload(modelId),
};

/**
 * 缩略图实际会请求的宽度档位。
 *
 * 请求尺寸必须向上取整到档位，而不是照着格子的 CSS 像素写死：
 * 窗口每缩放一次就换一个新宽度，等于每缩放一次全库缩略图缓存全部作废。
 * 四档已经够密——密度滑块的最小值 120 落到 128，最大值 360 落到 512，
 * 中间两档留给照片墙和检查器预览。
 */
const THUMB_BUCKETS = [128, 256, 384, 512] as const;
/** 需求宽度超过所有档位时的兜底，同时和宿主侧的 MAX_WIDTH 对齐。 */
const LARGEST_THUMB = 512;

/** 把想要的宽度抬到最近的档位。 */
export function thumbWidth(desired: number): number {
  return THUMB_BUCKETS.find((bucket) => bucket >= desired) ?? LARGEST_THUMB;
}

/**
 * 缩略图地址。走自定义协议而不是 IPC，浏览器自己做缓存与并发调度。
 *
 * **必须是 `http://`，不能写 `lumen://`。**
 *
 * WebView2 不认自定义协议，wry 的做法是把它们映射到
 * `http://<协议名>.localhost/`，再用 `WebResourceRequested` 按
 * `http://lumen.*` 这个过滤器拦下来。关键在于：**这个改写只发生在顶层导航
 * （wry 里 `WebviewBuilder::build` 对初始 URL 做的替换），子资源不经过它。**
 *
 * 所以 `<img src="lumen://localhost/thumb/abc">` 这种写法，WebView2 拿到的是一个
 * 不认识的协议，直接判为加载失败——请求连 Rust 都到不了，屏幕上只剩一个破图。
 * 写成 http 源，WebView2 才会按 `http://lumen.*` 发起请求并被 wry 拦下来，
 * 到 handler 手上时又被还原成 `lumen://localhost/thumb/abc`，正好是解析器认识的形状。
 * Tauri 自己的 `convertFileSrc` 也是产出这个形式，原因一样。
 *
 * 对应的 CSP 必须在 `img-src` 里放行 `http://lumen.localhost`（改写后的真实源），
 * 只写 `lumen:` 不够——CSP 是按浏览器实际请求的那个 URL 来匹配的。
 */
function thumbnailUrl(asset: Pick<AssetSummary, "id">, width: number): string {
  return `http://lumen.localhost/thumb/${asset.id}?w=${thumbWidth(width)}`;
}

/** Native-size source URL for the photo wall and Lightbox. Demo assets keep
 * their remote placeholder URL because there is no host protocol in a browser. */
export function originalUrlFor(asset: Pick<DemoAsset, "id" | "thumbUrl">): string {
  if (asset.thumbUrl.startsWith("http://lumen.localhost/thumb/")) {
    return asset.thumbUrl.replace(/http:\/\/lumen\.localhost\/thumb\/[^?]+(?:\?[^?]*)?/, `http://lumen.localhost/full/${asset.id}`);
  }
  return asset.thumbUrl;
}

/** 用一组目录替换图库并重扫。增、删、改顺序都走这一条。 */
export function setLibraryRoots(roots: string[]): Promise<LibraryStats> {
  return dataSource.setRoots(roots);
}

/** 自动发现含照片的目录。只发现、不采纳——勾选之后交给 `setLibraryRoots`。 */
export function discoverLibraries(base: string, maxDepth: number): Promise<DiscoveryReport> {
  return dataSource.discover(base, maxDepth);
}

/** 叫停正在进行的扫描。已扫到的照片保留，索引继续可用。 */
export function cancelScan(): Promise<LibraryStats> {
  return dataSource.cancelScan();
}

/** 重新扫描当前图库。取消之后按「继续扫描」走的就是它。 */
export function rescan(): Promise<LibraryStats> {
  return dataSource.rescan();
}

/** 设定缩略图缓存的字节上限。宿主会把低于下限的值抬回去，所以这里不重复判断。 */
export function setThumbCacheLimit(limitBytes: number): Promise<LibraryStats> {
  return dataSource.setThumbCacheLimit(limitBytes);
}

/** 删掉磁盘上的全部缩略图。下次用到哪张再重新生成哪张。 */
export function clearThumbCache(): Promise<LibraryStats> {
  return dataSource.clearThumbCache();
}

export const dataSource: DataSource = hostAvailable ? hostSource : demoSource;

/**
 * 已加载资产的索引。检查器要按 id 取回完整对象，
 * 而宿主只提供 asset_metadata（拍摄参数），没有按 id 取摘要的命令，
 * 所以这里在列表加载时顺手建索引，避免再查一次库。
 */
const assetIndex = new Map<string, DemoAsset>();

function rememberAssets(list: DemoAsset[]) {
  for (const asset of list) assetIndex.set(asset.id, asset);
}

export function assetById(id: string): DemoAsset | null {
  return assetIndex.get(id) ?? null;
}

/* ------------------------------------------------------------------ 加载态 */

/** 一页的条数。与虚拟网格的行高配合，一页大约是十几屏，足够掩盖加载延迟。 */
export const PAGE_SIZE = 600;

export type AsyncState<T> =
  | { status: "loading" }
  | { status: "ready"; value: T }
  /** message 给人看，raw 保留宿主原始错误，视图层据此区分未建索引与真实故障 */
  | { status: "error"; message: string; raw: unknown };

export type AsyncResource<T> = AsyncState<T> & {
  /** 依赖变了才重新拉取，期间回到 loading */
  reload(): void;
  /** 重新拉取但保留上一份数据，期间**不**回到 loading */
  refresh(): void;
};

/**
 * key 是依赖的稳定字符串。展开 deps 会让 React 无法校验依赖数组长度，
 * 这里改成显式传 key，代价是多写一个参数，换来的是依赖关系不会写错。
 */
function useAsync<T>(key: string, load: () => Promise<T>): AsyncResource<T> {
  const [state, setState] = useState<AsyncState<T>>({ status: "loading" });
  const [nonce, setNonce] = useState(0);
  /** 数字比对象更能表达「这一次要不要闪回加载态」，见 refresh 的注释。 */
  const [silent, setSilent] = useState(false);
  const loader = useRef(load);
  loader.current = load;

  useEffect(() => {
    let alive = true;
    // 只有「换了一个新东西要加载」才闪回加载态。扫描期间每 400ms 轮询一次状态，
    // 如果每次都闪，主区域就会周期性地被替换成「正在打开图库」再换回来——
    // 这就是扫描时整个界面抖动的原因。refresh 时留着旧数据，屏幕上什么都不动。
    if (!silent) setState({ status: "loading" });
    loader.current().then(
      (value) => {
        if (alive) setState({ status: "ready", value });
      },
      (error: unknown) => {
        if (alive) setState({ status: "error", message: describeError(error), raw: error });
      },
    );
    return () => {
      alive = false;
    };
  }, [key, nonce, silent]);

  const reload = useCallback(() => {
    setSilent(false);
    setNonce((n) => n + 1);
  }, []);

  const refresh = useCallback(() => {
    setSilent(true);
    setNonce((n) => n + 1);
  }, []);

  return { ...state, reload, refresh };
}

/* ---------------------------------------------------------------- 图库状态 */

/**
 * 全应用共用的一份图库状态。
 *
 * 这里以前是 `useAsync("stats", …)`，于是每个调用点各持一份互不相干的快照：
 * AppShell 那份每 400ms 轮询一次，LibraryView 那份只在挂载时取一次。扫描期间
 * 挂载的 LibraryView（正常路径就是这样：选完目录，扫描开始，视图才挂上来）
 * 会把 `scanning: true` 永久冻在那里，于是 LibraryView 里那句「扫描结束后再取
 * 一次完整结果」永远等不到 false——网格停在打开那一刻的一页上，而侧栏已经在
 * 报新数字了。同一个事实有两份状态，迟早会互相矛盾。
 *
 * 现在只有一份：谁刷新，所有人一起更新。
 */
let statsState: AsyncState<LibraryStats> = { status: "loading" };
/** 请求序号。回来得晚的响应如果已经不是最新的，就直接丢掉。 */
let statsSerial = 0;
const statsListeners = new Set<() => void>();

function publishStats(next: AsyncState<LibraryStats>) {
  statsState = next;
  statsListeners.forEach((listener) => listener());
}

function subscribeStats(listener: () => void) {
  statsListeners.add(listener);
  return () => {
    statsListeners.delete(listener);
  };
}

/** 快照必须是同一个对象引用，否则 useSyncExternalStore 每次渲染都会认为变了。 */
function readStats(): AsyncState<LibraryStats> {
  return statsState;
}

/**
 * 已经处在加载态时不必再发一次通知——`reload()` 在加载中重复调用很常见，
 * 每次都给所有人一次多余的渲染没有意义。
 *
 * `silent` 为真时保留上一份数据：扫描期间每 400ms 轮询一次，闪回加载态会把
 * 主区域反复替换成「正在打开图库」再换回来，这正是扫描时界面抖动的来源。
 */
function requestStats(silent: boolean) {
  const serial = (statsSerial += 1);
  if (!silent && statsState.status !== "loading") publishStats({ status: "loading" });
  dataSource.stats().then(
    (value) => {
      if (serial === statsSerial) publishStats({ status: "ready", value });
    },
    (error: unknown) => {
      if (serial === statsSerial) {
        publishStats({ status: "error", message: describeError(error), raw: error });
      }
    },
  );
}

export function useLibraryStats(): AsyncResource<LibraryStats> {
  const state = useSyncExternalStore(subscribeStats, readStats, readStats);

  // 第一份订阅负责把数据取回来，之后由使用方（轮询、命令回调）驱动刷新。
  useEffect(() => {
    if (statsSerial === 0) requestStats(false);
  }, []);

  const reload = useCallback(() => requestStats(false), []);
  const refresh = useCallback(() => requestStats(true), []);

  return { ...state, reload, refresh };
}

/**
 * 只订阅「扫描中」这一个布尔量。
 *
 * `useSyncExternalStore` 用 `Object.is` 比较快照，所以返回**原始值**时，
 * 扫描期间每 400ms 一次的状态发布不会让订阅方重渲染——只有这个布尔量真的
 * 翻转时才渲染一次（两次）。这正是它存在的理由：网格视图只用得到这一个字段，
 * 但订阅整份 stats 就意味着一屏几百张照片的网格每 400ms 跟着计时器白重渲一遍。
 *
 * 线程早就分开了（扫描在 `spawn_blocking` 里），把渲染线程拖住的是这种无谓的
 * 重渲染：渲染线程不闲，后台的进度就显示不出来，看起来就是「扫描把界面卡住了」。
 */
export function useLibraryScanning(): boolean {
  const scanning = useSyncExternalStore(subscribeStats, readScanning, readScanning);

  // 和 `useLibraryStats` 一样，第一份订阅负责把数据取回来。
  // 两个 hook 都调也没关系：`requestStats` 里用 `statsSerial === 0` 兜住了。
  useEffect(() => {
    if (statsSerial === 0) requestStats(false);
  }, []);

  return scanning;
}

/** 快照是布尔值，`Object.is` 因此只在真正翻转时才判定为变化。 */
function readScanning(): boolean {
  return statsState.status === "ready" && statsState.value.scanning;
}

/**
 * 类别计数。
 *
 * 依赖里带上扫描状态：类别是在扫描时逐条补上的（老索引里的记录 tags 是 0，
 * 要等增量扫描把头重读一遍），所以扫描跑完计数才会变。
 *
 * 用那一个布尔量而不是每 400ms 轮询：计数只在类别栏里用得到，
 * 让它跟着状态刷新的节拍一起算没有意义，而每一次算都要在宿主侧扫一遍全库。
 */
export function useTagCounts(): AsyncResource<AssetTagCount[]> {
  const scanning = useLibraryScanning();
  return useAsync(`tag-counts:${scanning ? "scanning" : "idle"}`, () =>
    dataSource.tagCounts(),
  );
}

export function useAssets(query: AssetQuery): AsyncResource<DemoAsset[]> {
  return useAsync(`assets:${query.sort}:${query.offset}:${query.limit}:${query.personId ?? ""}`, () =>
    dataSource.assets(query).then((list) => {
      rememberAssets(list);
      return list;
    }),
  );
}

export interface PagedAssets {
  assets: DemoAsset[];
  status: "loading" | "ready" | "error";
  message: string;
  raw: unknown;
  /** 还有下一页。宿主返回不足一页即视为到底。 */
  more: boolean;
  loadingMore: boolean;
  loadMore(): void;
  reload(): void;
}

/**
 * 分页累积加载。
 * 十万张的库不能一次性过 IPC，也不该被静默截断到某一页：
 * 滚到接近末尾时再拉下一页，宿主返回不足一页即视为到底。
 */
export function usePagedAssets(
  base: Omit<AssetQuery, "offset" | "limit">,
  pageSize = PAGE_SIZE,
): PagedAssets {
  const baseKey = `${base.sort}:${base.bucket ?? ""}:${base.personId ?? ""}:${base.tag ?? ""}:${pageSize}`;
  const [assets, setAssets] = useState<DemoAsset[]>([]);
  const [status, setStatus] = useState<PagedAssets["status"]>("loading");
  const [message, setMessage] = useState("");
  const [raw, setRaw] = useState<unknown>(null);
  const [more, setMore] = useState(true);
  const [loadingMore, setLoadingMore] = useState(false);

  /**
   * 把 base 拆成原词再重新组装。
   *
   * 调用方每次都是 `usePagedAssets({ sort, bucket: null, personId })，对象字面里永远是新的。
   * `fetchPage` 把 base 放在依赖里，所以它每次渲染都是新函数——
   * 网格里的 `onEndReached={assets.loadMore}` 亏就是新闭包，让下游的 memo 全部失效。
   */
  const { sort, bucket, personId, tag } = base;
  const stableBase = useMemo(
    () => ({ sort, bucket, personId, tag }),
    [sort, bucket, personId, tag],
  );

  const fetchPage = useCallback(
    (offset: number, append: boolean) => {
      const run = async () => {
        if (append) setLoadingMore(true);
        try {
          const page = await dataSource.assets({ ...stableBase, limit: pageSize, offset });
          rememberAssets(page);
          setAssets((prev) => (append ? [...prev, ...page] : page));
          setMore(page.length === pageSize);
          setStatus("ready");
          setRaw(null);
        } catch (error: unknown) {
          setMessage(describeError(error));
          setRaw(error);
          setStatus((prev) => (prev === "loading" ? "error" : prev));
        } finally {
          setLoadingMore(false);
        }
      };
      void run();
    },
    [stableBase, pageSize],
  );

  useEffect(() => {
    setAssets([]);
    setMore(true);
    setStatus("loading");
    fetchPage(0, false);
    // baseKey 已经把 base 里参与查询的字段拼平了
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [baseKey]);

  return {
    assets,
    status,
    message,
    raw,
    more,
    loadingMore,
    loadMore: () => {
      if (more && !loadingMore && status === "ready") fetchPage(assets.length, true);
    },
    reload: () => {
      setAssets([]);
      setMore(true);
      setStatus("loading");
      fetchPage(0, false);
    },
  };
}

export function usePeople(): AsyncResource<PersonSummary[]> {
  return useAsync("people", () => dataSource.people());
}

/**
 * The model catalog plus live download progress.
 *
 * Downloads report progress by publishing byte counts into the host state, and
 * the host has no push channel for them — so, exactly like `library_stats`
 * during a scan, the frontend polls. Polling only runs while something is
 * actually downloading; an idle catalog is fetched once and then left alone.
 */
/**
 * 模型目录的变更广播。
 *
 * 为什么需要这个：设置页和人物页都在用同一份清单，而下载是从
 * 哪一页发起的不知道。没有广播就只能靠传回一个 callback，而那要每个
 * 调用点都自己记住。一个模块级的“目录变了”信号让两个页面都自己跟。
 */
let modelsGeneration = 0;
const modelListeners = new Set<() => void>();

function subscribeModels(listener: () => void) {
  modelListeners.add(listener);
  return () => modelListeners.delete(listener);
}

function announceModelChange() {
  modelsGeneration += 1;
  modelListeners.forEach((listener) => listener());
}

export function useModels(): AsyncResource<ModelEntry[]> {
  const resource = useAsync("models", () => dataSource.models());
  const generation = useSyncExternalStore(subscribeModels, () => modelsGeneration, () => modelsGeneration);
  const downloading = resource.status === "ready" && resource.value.some((m) => m.status === "downloading");

  // 有人开始/取消了下载：立即拉一次目录，否则下面这个 effect 永远不会启动。
  //
  // 这是之前“点了下载但看不到进度”的根因：轮询的前提是目录里已经出现
  // `downloading`，而旧的目录一直说“absent”。两者互为因果，每次都在等对方先动。
  const started = generation > 0;
  useEffect(() => {
    if (!started) return;
    resource.refresh();
  }, [generation, started, resource.refresh]);

  useEffect(() => {
    if (!downloading) return;
    const timer = window.setInterval(() => resource.refresh(), 500);
    return () => window.clearInterval(timer);
    // refresh 本身是稳定的（useCallback），依赖它不会让定时器每帧重建。
  }, [downloading, resource.refresh]);

  return resource;
}

/**
 * 开始下载一个模型。`licenseAccepted` 由界面在用户看过许可全文之后传进来。
 *
 * 这一层不做任何兜底：宿主拒绝就是拒绝，把错误原样抛给设置页展示，
 * 好过在这里吞掉然后让用户以为已经开始了。
 */
export function startModelDownload(modelId: string, licenseAccepted: boolean): Promise<ModelEntry> {
  return dataSource.startDownload(modelId, licenseAccepted).finally(announceModelChange);
}

export function cancelModelDownload(modelId: string): Promise<ModelEntry> {
  return dataSource.cancelDownload(modelId).finally(announceModelChange);
}

/**
 * 检索。空词直接给空结果，不发给宿主。
 *
 * 进入搜索页时 query 是空的，而 `useAsync` 一挂载就会请求一次。发过去的话
 * 宿主要么白扫一遍全库，要么（没打开图库时）回一个 `no_library` 错误，
 * 于是刚进页面就闪一个红色的失败态。空词本来就没有答案，这里直接回答它。
 */
export function useSearch(query: string, mode: SearchMode) {
  return useAsync(`search:${query}:${mode}`, () =>
    query.trim().length === 0
      ? Promise.resolve([])
      : dataSource.search({ query, mode, limit: 400 }),
  );
}

export function useAssetMetadata(assetId: string | null): AsyncResource<AssetMetadata | null> {
  return useAsync(`metadata:${assetId ?? ""}`, async () =>
    assetId ? dataSource.metadata(assetId) : null,
  );
}
