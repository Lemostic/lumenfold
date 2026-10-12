/**
 * 与 Rust 宿主之间的数据契约。
 * 字段名与 serde camelCase 一一对应，改动必须同时改 apps/desktop/src-tauri/src/types.rs。
 */

export interface AppInfo {
  version: string;
  apiVersion: number;
}

export interface LibraryStats {
  assetCount: number;
  indexed: boolean;
  /**
   * 作为图库的目录，按用户排定的顺序。
   *
   * 以前这里是 `root: string | null`——图库只有一个根。现在是一组：
   * 图库更像知识库，可以同时挂 `iCloud Photos`、`DCIM` 和一块素材盘。
   * 空数组表示还没配过图库。
   */
  roots: string[];
  /**
   * 内存 LRU 加上磁盘缓存目录，两者之和——应用确实占着这么多字节。
   */
  cacheBytes: number;
  /**
   * 缩略图缓存的字节上限，由用户在设置页决定。
   *
   * 这个字段以前根本不存在，设置页却写着「上限默认 10GB」：一句没有兑现、
   * 也没有任何代码去守的承诺。现在它是真的——超出之后由磁盘缓存按写入时间
   * 从最旧的开始删。
   */
  cacheLimitBytes: number;
  /** 扫描进行中，计数会持续增长 */
  scanning: boolean;
  /** 最近一次扫描是被用户叫停的，而不是自己跑完的 */
  cancelled: boolean;
  /** 最近一次扫描里直接从持久化索引复用、没有重新打开照片的张数 */
  scanReused: number;
  /** 最近一次扫描里不得不打开照片重新读取元数据的张数 */
  scanProbed: number;
}

/** 批量操作对文件做什么。与 Rust 侧 BatchAction 一一对应。 */
export type BatchAction = "copy" | "move" | "delete";

/** 批量操作需要目标文件夹；删除不需要。 */
export function needsDestination(action: BatchAction): boolean {
  return action !== "delete";
}

export interface BatchRequest {
  assetIds: string[];
  action: BatchAction;
  /** 目标文件夹。删除时忽略，复制与剪切必填。 */
  destination: string | null;
}

export interface BatchItemResult {
  assetId: string;
  fileName: string;
  ok: boolean;
  /** 失败原因，成功时为空串 */
  message: string;
  /** 复制/剪切后的新位置，删除时为 null */
  path: string | null;
}

export interface BatchReport {
  items: BatchItemResult[];
  succeeded: number;
  failed: number;
  destination: string | null;
}

export type Orientation = "portrait" | "landscape" | "square" | "unknown";

/**
 * 类别标记位，与 `lumenfold_index` 的 `TAG_*` 一一对应。
 * 改动必须同时改 apps/desktop/src-tauri/crates/lumenfold-index/src/lib.rs。
 */
export const TAG_ANIMATED = 1 << 0;
export const TAG_SCREENSHOT = 1 << 1;
export const TAG_RAW = 1 << 2;
export const TAG_PANORAMA = 1 << 3;
/** 「这条记录已经分过类」。老索引里的记录没有这一位，分类靠增量扫描补齐。 */
export const TAG_CLASSIFIED = 1 << 31;

/**
 * 位测试。
 *
 * `>>> 0` 是必须的，不是防御性写法：JS 的按位运算按 32 位**有符号**做，
 * `1 << 31` 求值是 -2147483648。不还原成无符号的话，`tags & TAG_CLASSIFIED`
 * 会得到一个负数，读代码的人会以为哪里溢出了。
 */
export function hasTag(tags: number, tag: number): boolean {
  return ((tags & tag) >>> 0) !== 0;
}

export interface AssetSummary {
  id: string;
  path: string;
  fileName: string;
  /** 读不到尺寸的格式（RAW / HEIC 未接解码器）会是 0 */
  width: number;
  height: number;
  bytes: number;
  /** Unix 秒，未知为 null */
  takenAt: number | null;
  rating: 0 | 1 | 2 | 3 | 4 | 5;
  flagged: boolean;
  orientation: Orientation;
  /**
   * 类别标记位，见上面的 `TAG_*`。
   *
   * 最高位是「分过类」。老索引里的记录没有它——那时界面该说「分类还在补齐」，
   * 而不是把「还没数」显示成「没有」。
   */
  tags: number;
}

/** 一个类别当前有多少张照片。宿主只回 id 和数量，中文名在 `TAG_META` 里。 */
export interface AssetTagCount {
  id: string;
  count: number;
  /** 已经分过类的记录总数；小于 `total` 就说明分类还在补齐 */
  classified: number;
  /** 已索引的照片总数，不分是否分过类 */
  total: number;
}

/**
 * 类别栏的界面元数据。数组顺序就是栏里的排列顺序。
 *
 * 「全部」不在表里：它是「不加筛选」而不是一个真的类别，宿主那边也没有
 * 对应的标记位，硬塞进这张表会让人以为它是个可以查的类别。
 */
export const TAG_META: readonly {
  id: string;
  label: string;
  hint: string;
  bit: number;
}[] = [
  {
    id: "animated",
    label: "动图",
    hint: "容器里真的有多帧：GIF、APNG、动画 WebP、HEIF 图像序列。逐帧播放尚未支持，这里筛的是「它本来就是动图」。",
    bit: TAG_ANIMATED,
  },
  {
    id: "screenshot",
    label: "截图",
    hint: "启发式判断，会漏也会误：文件名里有截图标记，或者是「PNG + 没有拍摄时间 + 尺寸正好是某个常见屏幕分辨率」。",
    bit: TAG_SCREENSHOT,
  },
  {
    id: "raw",
    label: "RAW",
    hint: "相机原始格式。本版本扫描的 RAW 只有 cr2 / nef / dng，且暂时读不出尺寸。",
    bit: TAG_RAW,
  },
  {
    id: "panorama",
    label: "全景",
    hint: "长边至少是短边的 2.5 倍。竖幅全景同样算。</br>",
    bit: TAG_PANORAMA,
  },
];

/** Pixel transforms used by the Lightbox editor. */
export interface ImageTransform {
  rotate: number;
  flipH: boolean;
  flipV: boolean;
}

export type EditMode = "overwrite" | "copy";

/**
 * 画质增强任务的进度快照，与 Rust 侧 `enhance::EnhanceProgress` 一一对应。
 *
 * 一次 12MP 的增强要 8 到 9 分钟，所以它必须能把「跑到哪了」说出来，
 * 而不是只有一个转圈。
 */
export interface EnhancementTask {
  id: string;
  assetId: string;
  fileName: string;
  sourcePath: string;
  outputPath: string | null;
  startedAt: number;
  finishedAt: number | null;
  durationMs: number | null;
  status: "running" | "succeeded" | "failed" | "cancelled" | "interrupted";
  inputBytes: number;
  outputBytes: number | null;
  error: string | null;
  previewPath: string | null;
}


export interface EnhanceProgress {
  running: boolean;
  done: number;
  total: number;
  cancelled: boolean;
  provider: string;
}

export interface AssetMetadata {
  assetId: string;
  cameraMake: string | null;
  cameraModel: string | null;
  lens: string | null;
  iso: number | null;
  /** 原文，例如 "1/250s" */
  shutter: string | null;
  /** f 值，例如 1.8 */
  aperture: number | null;
  focalLengthMm: number | null;
  keywords: string[];
  caption: string | null;
  gpsLat: number | null;
  gpsLon: number | null;
}

export type SortKey = "taken-desc" | "taken-asc" | "name" | "size" | "rating";

export interface AssetQuery {
  limit: number;
  offset: number;
  sort: SortKey;
  /** 分桶：时间线日键、相册 id、人物 id */
  bucket: string | null;
  personId: string | null;
  /**
   * 类别筛选，取 `TAG_META` 的 id（`animated` / `screenshot` / `raw` / `panorama`）。
   * `null` 表示不筛。
   *
   * 未知 id 不是「不过滤」：宿主会让它返回零张，因为「按一个不存在的类别筛选」
   * 的真实答案就是零张，而不是把整库端出来。
   */
  tag: string | null;
}

export interface PersonSummary {
  id: string;
  name: string | null;
  coverAssetId: string | null;
  faceCount: number;
  assetCount: number;
}

export type SearchMode = "text" | "semantic";

export interface SearchRequest {
  query: string;
  mode: SearchMode;
  limit: number;
}

export interface SearchResult {
  asset: AssetSummary;
  score: number;
}

export type ModelStatus = "absent" | "downloading" | "ready" | "error";

/** 自动发现出来的一个候选目录。与 Rust 侧 `discover::LibraryCandidate` 一一对应。 */
export interface LibraryCandidate {
  path: string;
  /** 目录名；盘根没有名字时后端回退到整条路径 */
  name: string;
  /** 这个目录**自己**直接含多少张照片，不算子目录 */
  photos: number;
  /** 连同子目录一共多少张 */
  photosDeep: number;
  alreadyAdded: boolean;
}

export interface DiscoveryReport {
  base: string;
  /** 按「一共多少张」从多到少排好的候选 */
  candidates: LibraryCandidate[];
  /** 撞到遍历上限了，结果不完整 */
  truncated: boolean;
  visited: number;
}

/**
 * 图库目录的展示文案。
 *
 * 一个目录就直接显示它；多个就显示第一个加个数——把五条路径全塞进状态栏
 * 只会把真正该看的东西挤没，完整列表在设置页里。
 */
export function libraryLabel(roots: readonly string[]): string | null {
  if (roots.length === 0) return null;
  if (roots.length === 1) return roots[0] ?? null;
  return `${roots[0]} 等 ${roots.length} 个目录`;
}

/**
 * 模型能力。必须是封闭枚举：人物视图要判断「缺的是识别模型还是检测模型」，
 * 靠匹配 task 的自由文本会把 CLIP 的图像编码器误认成人脸识别模型。
 * Rust 侧对应 ModelCapability，序列化值一致。
 */
export type ModelCapability =
  | "text-embedding"
  | "image-embedding"
  | "face-detect"
  | "face-embed"
  /** 画质增强。唯一一个属于「编辑」而不是「索引」的能力。 */
  | "super-resolution"
  | "other";

export interface ModelEntry {
  id: string;
  name: string;
  /** 展示用的中文任务描述 */
  task: string;
  /** 供程序判断的能力标签，不要用它做展示 */
  capability: ModelCapability;
  license: string;
  commercialUse: boolean;
  sizeBytes: number;
  status: ModelStatus;
  downloadedBytes: number;
  sourceUrl: string;
  sha256: string;
}

/** 宿主返回的错误结构，LumenError 的序列化形式 */
export interface HostError {
  kind: "not_indexed" | "unsupported" | "io" | "no_library";
  message: string;
}

export function errorKind(error: unknown): HostError["kind"] | null {
  if (typeof error === "object" && error !== null && "kind" in error) {
    const kind = error.kind;
    if (
      kind === "not_indexed" ||
      kind === "unsupported" ||
      kind === "io" ||
      kind === "no_library"
    ) {
      return kind;
    }
  }
  return null;
}
