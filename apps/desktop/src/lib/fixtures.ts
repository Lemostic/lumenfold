/**
 * 演示图库。仅在浏览器里跑 `pnpm dev`、宿主不可用时接管，
 * 用途是评估布局与交互。真实数据一律来自 Rust 宿主。
 *
 * 缩略图为 picsum 占位摄影，尺寸比例与真实缩略图流水线一致。
 * 数据由固定种子生成，每次刷新得到同一组图库，布局评估才有可比性。
 */
import {
  TAG_CLASSIFIED,
  TAG_PANORAMA,
  type AssetMetadata,
  type AssetSummary,
  type LibraryStats,
  type ModelEntry,
  type PersonSummary,
} from "./types";

/**
 * 演示数据的类别位。
 *
 * 规则和宿主 `lumenfold_index::classify` 一致，只是这里没有文件可读，
 * 所以「动图」判断不了。这份图库全是 JPEG，于是动图和 RAW 在浏览器里就是 0——
 * 那是实话，不是缺数据。
 */
function deriveDemoTags(width: number, height: number): number {
  const long = Math.max(width, height);
  const short = Math.min(width, height);
  const panorama = short > 0 && long * 2 >= short * 5;
  return TAG_CLASSIFIED | (panorama ? TAG_PANORAMA : 0);
}

/** 演示缩略图的本地占位地址。真实运行时由 lumen://thumb/<hash> 提供。 */
export interface DemoAsset extends AssetSummary {
  thumbUrl: string;
}

/** mulberry32：小而确定的 PRNG */
function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const CAMERAS = [
  { make: "FUJIFILM", model: "X-T5", lens: "XF 35mm F1.4 R", iso: [160, 200, 400, 800] },
  { make: "SONY", model: "ILCE-7RM5", lens: "FE 24-70mm F2.8 GM II", iso: [100, 200, 400, 640] },
  { make: "Canon", model: "EOS R5", lens: "RF 85mm F1.2 L USM", iso: [100, 320, 640, 1250] },
  { make: "NIKON", model: "Z8", lens: "NIKKOR Z 14-24mm f/2.8 S", iso: [64, 200, 800, 1600] },
  { make: "Leica", model: "Q3", lens: "Summilux 28mm f/1.7 ASPH.", iso: [100, 400, 800, 2000] },
];

const SEEDS = [
  "lumenfold-coast-dawn",
  "lumenfold-alpine-ridge",
  "lumenfold-desert-dunes",
  "lumenfold-street-night",
  "lumenfold-forest-fog",
  "lumenfold-harbour-boats",
  "lumenfold-market-stalls",
  "lumenfold-reef-shallows",
  "lumenfold-autumn-canopy",
  "lumenfold-concrete-facade",
  "lumenfold-lake-reflect",
  "lumenfold-train-platform",
  "lumenfold-glacier-blue",
  "lumenfold-field-hay",
  "lumenfold-old-town",
  "lumenfold-river-delta",
];

const RATIOS: Array<[number, number]> = [
  [3, 2],
  [2, 3],
  [4, 3],
  [3, 4],
  [16, 9],
  [1, 1],
  [5, 4],
  [4, 5],
];

const SHUTTERS = ["1/4000s", "1/1000s", "1/500s", "1/250s", "1/125s", "1/60s", "1/15s"];
const APERTURES = [1.4, 1.8, 2.8, 4, 5.6, 8, 11];
const FOCAL_LENGTHS = [16, 24, 28, 35, 50, 85, 135];
const KEYWORDS = ["海雾", "晨光", "长曝", "街头", "人像", "云层", "逆光", "水面", "雪线", "夜灯"];

const CAPTION_POOL = [
  "第三次尝试，潮水退到礁石以下。",
  "风很大，曝光只能靠拉高感光度。",
  "同行的人在后面等了四十分钟。",
  "",
  "",
  "这张是整组里唯一没有裁切过的。",
];

const BASE_TIME = Date.UTC(2026, 8, 30, 9, 12, 0) / 1000;

interface DemoLibrary {
  assets: DemoAsset[];
  metadata: Record<string, AssetMetadata>;
}

/** 从定长数组里按种子取一项。返回类型被数组元素类型收窄，不需要断言。 */
function pick<T>(pool: readonly T[], rand: () => number): T {
  const item = pool[Math.floor(rand() * pool.length)];
  if (item === undefined) throw new Error("演示数据池不能为空");
  return item;
}

function pickRating(rand: () => number): DemoAsset["rating"] {
  const stars = rand() < 0.42 ? 1 + Math.floor(rand() * 5) : 0;
  return stars as DemoAsset["rating"];
}

function buildDemoLibrary(count: number): DemoLibrary {
  const rand = mulberry32(20261007);
  const assets: DemoAsset[] = [];
  const metadata: Record<string, AssetMetadata> = {};
  let clock = BASE_TIME;

  for (let i = 0; i < count; i += 1) {
    const [rw, rh] = pick(RATIOS, rand);
    const width = rw >= rh ? 2400 : Math.round((2400 * rw) / rh);
    const height = rw >= rh ? Math.round((2400 * rh) / rw) : 2400;
    const camera = pick(CAMERAS, rand);

    // 拍摄时间成簇分布：一天里会连着拍十几张，但簇与簇之间会隔上几天
    clock -= 1800 + Math.floor(rand() * 5400);

    const id = `fx${i.toString(36).padStart(4, "0")}`;
    const seed = `${pick(SEEDS, rand)}-${i}`;
    const keywords = Array.from({ length: 1 + Math.floor(rand() * 3) }, () => pick(KEYWORDS, rand));

    assets.push({
      id,
      path: `D:/photos/${new Date(clock * 1000).getFullYear()}/${id}.jpg`,
      fileName: `IMG_${4821 + i}.jpg`,
      width,
      height,
      bytes: Math.floor(2_400_000 + rand() * 9_600_000),
      takenAt: clock,
      rating: pickRating(rand),
      flagged: rand() < 0.06,
      orientation: width === height ? "square" : width > height ? "landscape" : "portrait",
      tags: deriveDemoTags(width, height),
      thumbUrl: `https://picsum.photos/seed/${seed}/${width}/${height}`,
    });

    const hasGps = rand() < 0.55;
    metadata[id] = {
      assetId: id,
      cameraMake: camera.make,
      cameraModel: camera.model,
      lens: camera.lens,
      iso: pick(camera.iso, rand),
      shutter: pick(SHUTTERS, rand),
      aperture: pick(APERTURES, rand),
      focalLengthMm: pick(FOCAL_LENGTHS, rand),
      keywords,
      caption: pick(CAPTION_POOL, rand) || null,
      gpsLat: hasGps ? 21.4 + rand() * 18 : null,
      gpsLon: hasGps ? 118.2 + rand() * 9 : null,
    };
  }

  return { assets, metadata };
}

const demo = buildDemoLibrary(1800);

export const DEMO_ASSETS: DemoAsset[] = demo.assets;
export const DEMO_METADATA: Record<string, AssetMetadata> = demo.metadata;

export const DEMO_STATS: LibraryStats = {
  assetCount: demo.assets.length,
  indexed: true,
  // 演示模式也给两个目录，好让「多图库」这条路径在浏览器里能被看到，
  // 而不是只在真实宿主里才第一次显形。
  roots: ["D:/photos", "D:/photos-archive"],
  cacheBytes: 4_362_341_632,
  // 与 Rust 侧 `thumbstore::DEFAULT_LIMIT_BYTES` 保持一致。演示数据自己编一个
  // 更好看的数字，只会让「10GB」那个旧文案看起来像是真的。
  cacheLimitBytes: 64 * 1024 * 1024 * 1024,
  cancelled: false,
  scanning: false,
  // 演示数据没有真的扫过目录，所以这两个数只能是 0。状态栏据此不显示
  // 「复用 N / 重读 M」——演示模式把这件事说成真的，比不说更糟。
  scanReused: 0,
  scanProbed: 0,
};

/**
 * 人物簇的封面直接取演示图库里真实存在的缩略图，不再另造一批图片请求。
 * 真实运行时会取自人脸框裁切缓存。
 */
const COVER_SOURCE_INDEX = [41, 113, 207, 288, 355, 410, 466, 521, 577];

export const DEMO_PEOPLE: PersonSummary[] = [
  { id: "p1", name: "林淑贞", faceCount: 312, assetCount: 1408 },
  { id: "p2", name: null, faceCount: 87, assetCount: 392 },
  { id: "p3", name: "周牧", faceCount: 264, assetCount: 1103 },
  { id: "p4", name: null, faceCount: 21, assetCount: 96 },
  { id: "p5", name: "Aiko Nakamura", faceCount: 148, assetCount: 640 },
  { id: "p6", name: null, faceCount: 9, assetCount: 41 },
  { id: "p7", name: "陈可", faceCount: 197, assetCount: 802 },
  { id: "p8", name: null, faceCount: 34, assetCount: 158 },
  { id: "p9", name: "Dario Ferraro", faceCount: 112, assetCount: 497 },
].map((person, index) => ({
  ...person,
  coverAssetId: demo.assets[COVER_SOURCE_INDEX[index] ?? 0]?.id ?? null,
}));

export const DEMO_MODELS: ModelEntry[] = [
  {
    id: "clip-vit-b32",
    name: "CLIP ViT-B/32 图像编码器",
    task: "语义检索与自动标签",
    capability: "image-embedding",
    license: "MIT",
    commercialUse: true,
    sizeBytes: 354_529_280,
    status: "ready",
    downloadedBytes: 354_529_280,
    sourceUrl: "https://huggingface.co/Xenova/clip-vit-base-patch32",
    sha256: "9f1c1d0a4e8b6b1f0f2f0a4c3d5e6b7a8c9d0e1f2a3b4c5d6e7f8091a2b3c4d5",
  },
  {
    id: "yunet-2023mar",
    name: "YuNet 人脸检测器",
    task: "人脸检测",
    capability: "face-detect",
    license: "MIT",
    commercialUse: true,
    sizeBytes: 232_586,
    status: "ready",
    downloadedBytes: 232_586,
    sourceUrl: "https://github.com/opencv/opencv_zoo/tree/main/models/face_detection_yunet",
    sha256: "3b7a1c9e2d4f5061728394a5b6c7d8e9f00112233445566778899aabbccddeeff",
  },
  {
    id: "insightface-buffalo-l",
    name: "InsightFace buffalo_l 检测与识别",
    task: "人脸检测与嵌入",
    capability: "face-embed",
    license: "InsightFace 非商用",
    commercialUse: false,
    sizeBytes: 326_176_768,
    status: "absent",
    downloadedBytes: 0,
    sourceUrl: "https://github.com/deepinsight/insightface",
    sha256: "官方分发未提供校验和，导入时本地计算并记录",
  },
];