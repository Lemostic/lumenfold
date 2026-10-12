import { invoke, isTauri } from "@tauri-apps/api/core";
import type {
  AppInfo,
  AssetMetadata,
  AssetQuery,
  AssetSummary,
  AssetTagCount,
  BatchReport,
  BatchRequest,
  DiscoveryReport,
  EnhanceProgress,
  EnhancementTask,
  LibraryStats,
  ModelEntry,
  PersonSummary,
  SearchRequest,
  SearchResult,
  EditMode,
  ImageTransform,
} from "./types";

/**
 * Tauri 命令的唯一入口。
 * 所有跨进程调用都走这里，视图层不允许直接 import invoke，
 * 这样把 Tauri 换成别的壳时只需替换本文件。
 */
export const bridge = {
  appInfo: () => invoke<AppInfo>("app_info"),
  libraryStats: () => invoke<LibraryStats>("library_stats"),
  /**
   * 打开一个目录作为图库。它是「换个位置」那条路：把图库**替换成**这一个目录。
   * 要挂着多个目录用 `setLibraryRoots`。
   */
  openLibrary: (path: string) => invoke<LibraryStats>("open_library", { path }),
  /**
   * 用一组目录替换图库并重扫。增、删、改顺序都走这一条：图库目录是一个整体，
   * 分开做「加一个」「删一个」会让并发调用有机会插在中间，落下一个中间态。
   */
  setLibraryRoots: (roots: string[]) =>
    invoke<LibraryStats>("set_library_roots", { roots }),
  /**
   * 自动发现含照片的目录。只发现、不采纳——勾选之后由调用方交给 `setLibraryRoots`。
   *
   * `maxDepth` 是硬上限：`iCloud Photos` 这类同步目录走一遍要几分钟。
   */
  discoverLibraries: (base: string, maxDepth: number) =>
    invoke<DiscoveryReport>("discover_libraries", { base, maxDepth }),
  rescan: () => invoke<LibraryStats>("rescan"),
  cancelScan: () => invoke<LibraryStats>("cancel_scan"),
  /**
   * 缩略图缓存的字节预算。传的是**字节数**而不是「10GB」这种带单位的字符串：
   * 宿主要拿它跟 `cache_bytes` 直接比大小，带单位的文案只能在前端再转一次，
   * 转错了就又是一句没有兑现的承诺。返回同一份统计，好让调用方就地刷新。
   */
  setThumbCacheLimit: (limitBytes: number) =>
    invoke<LibraryStats>("set_thumb_cache_limit", { limitBytes }),
  /**
   * 删掉磁盘上所有缩略图。
   *
   * 单独一个动作，而不是靠「把上限调到 1GB」来顺带清空：后者只淘汰到 90%，
   * 用户要的也从来不是「更小的缓存」，而是「把缓存清掉」。
   */
  clearThumbCache: () => invoke<LibraryStats>("clear_thumb_cache"),
  batchOperate: (request: BatchRequest) =>
    invoke<BatchReport>("batch_operate", { request }),
  clipboardFiles: (assetIds: string[], cut: boolean) =>
    invoke<number>("clipboard_files", { assetIds, cut }),
  queryAssets: (query: AssetQuery) =>
    invoke<AssetSummary[]>("query_assets", { query }),
  /**
   * 类别计数。单独一个命令而不是塞进 `libraryStats`：它在扫描期间一直在变，
   * 而只有打开类别栏时才需要，跟着状态轮询一起算纯属浪费。
   */
  assetTagCounts: () => invoke<AssetTagCount[]>("asset_tag_counts"),
  assetMetadata: (assetId: string) =>
    invoke<AssetMetadata>("asset_metadata", { assetId }),
  listPeople: () => invoke<PersonSummary[]>("list_people"),
  search: (req: SearchRequest) => invoke<SearchResult[]>("search", { req }),
  listModels: () => invoke<ModelEntry[]>("list_models"),
  /**
   * `licenseAccepted` 是 ADR-0002 §4「许可先于首字节」的那一半：
   * 宿主在收到 true 之前不会发出任何 HTTP 请求，并把这次接受记进 licenses.json。
   */
  startModelDownload: (modelId: string, licenseAccepted: boolean) =>
    invoke<ModelEntry>("start_model_download", { modelId, licenseAccepted }),
  cancelModelDownload: (modelId: string) =>
    invoke<ModelEntry>("cancel_model_download", { modelId }),
  /**
   * `enhance` 为真时宿主会在几何变换之后跑一次超分再缩回原尺寸。
   *
   * 那一步要几分钟，所以配套给了 `enhanceProgress` / `cancelEnhance`：
   * 一个八分钟的操作不能只有「转圈」。
   */
  editImage: (assetId: string, transform: ImageTransform, mode: EditMode, enhance = false) =>
    invoke<void>("edit_image", { request: { assetId, transform, mode, enhance } }),
  enhanceProgress: () => invoke<EnhanceProgress>("enhance_progress"),
  startEnhancement: (assetId: string) => invoke<EnhancementTask>("start_enhancement", { assetId }),
  enhancementTasks: () => invoke<EnhancementTask[]>("enhancement_tasks"),

  cancelEnhance: () => invoke<void>("cancel_enhance"),
};

export const hostAvailable = isTauri();

/** 把未知错误统一成可展示的一句中文。IPC 返回值跨进程，用 in 收窄而不是断言。 */
export function describeError(error: unknown): string {
  if (typeof error === "string") return error;
  if (typeof error === "object" && error !== null && "message" in error) {
    return String(error.message);
  }
  return "宿主返回了无法识别的错误";
}
