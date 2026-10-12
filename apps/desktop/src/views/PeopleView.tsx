import { UserCircle, UsersThree, Warning } from "@phosphor-icons/react";
import { EmptyState, ErrorState } from "../components/States";
import { Button } from "../components/Button";
import { assetById, useAssets, useModels, usePeople } from "../state/library";
import { formatCount } from "../lib/format";
import { ui } from "../state/ui";
import { errorDescription, errorTitle } from "./LibraryView";
import type { ModelEntry } from "../lib/types";

/** 人物簇的封面尺寸，够看清脸又不会让一屏只剩三张 */
const COVER = 168;

/**
 * 人脸聚类需要识别与嵌入权重，检测器（YuNet）单独不够。
 * 按结构化的 capability 判断，不去匹配 task 的展示文案：
 * 否则 CLIP 的图像编码器会被误认成人脸识别模型。
 */
function faceModel(models: ModelEntry[]): ModelEntry | undefined {
  return models.find((m) => m.capability === "face-embed");
}

/** 检测器是下游：拿到脸之后还要嵌入才能归类。 */
function faceDetector(models: ModelEntry[]): ModelEntry | undefined {
  return models.find((m) => m.capability === "face-detect");
}

export function PeopleView() {
  const people = usePeople();
  // 封面取自图库里的真实缩略图，因此这里要先确保资产列表已加载进索引缓存
  useAssets({ limit: 600, offset: 0, sort: "taken-desc", bucket: null, personId: null, tag: null });
  const models = useModels();
  const recognition = models.status === "ready" ? faceModel(models.value) : undefined;
  const detector = models.status === "ready" ? faceDetector(models.value) : undefined;

  if (people.status === "error") {
    return (
      <ErrorState
        title={errorTitle(people.raw)}
        description={errorDescription(people.raw)}
        detail={people.message}
        onRetry={people.reload}
      />
    );
  }

  const clusters = people.status === "ready" ? people.value : [];

  return (
    <div className="h-full min-h-0 overflow-y-auto scroll-area">
      <div className="mx-auto w-full max-w-[1400px] px-4 py-4">
        <ModelNotice model={recognition} detector={detector} modelsReady={models.status === "ready"} />

        {people.status === "loading" ? (
          <div className="grid gap-3" style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${COVER}px, 1fr))` }}>
            {Array.from({ length: 12 }, (_, i) => (
              <div key={i} className="skeleton rounded-md" style={{ height: COVER + 44 }} />
            ))}
          </div>
        ) : clusters.length === 0 ? (
          <EmptyState
            icon={<UsersThree size={22} />}
            title="还没有识别出任何人物"
            description="开启人脸识别后，宿主会为每张照片检测人脸、抽取嵌入并做聚类。命名与合并都可以手工纠正，聚类结果随时可以拆分。"
          />
        ) : (
          <ul className="grid gap-3" style={{ gridTemplateColumns: `repeat(auto-fill, minmax(${COVER}px, 1fr))` }}>
            {clusters.map((person) => {
              const cover = person.coverAssetId ? assetById(person.coverAssetId) : null;
              return (
                <li key={person.id}>
                  <button
                    type="button"
                    onClick={() => ui.openPerson(person.id)}
                    className="group flex w-full flex-col items-start rounded-md border border-line bg-surface p-2 text-left transition-colors duration-(--lf-duration-fast) hover:border-line-strong hover:bg-surface-2 active:translate-y-px"
                  >
                    <span className="block w-full overflow-hidden rounded-sm bg-sunken" style={{ height: COVER }}>
                      {cover ? (
                        <img
                          src={cover.thumbUrl}
                          alt=""
                          loading="lazy"
                          decoding="async"
                          className="h-full w-full object-cover"
                        />
                      ) : (
                        <span className="flex h-full w-full items-center justify-center text-ink-3">
                          <UserCircle size={26} />
                        </span>
                      )}
                    </span>
                    <span className="mt-2 w-full truncate text-sm font-medium text-ink">
                      {person.name ?? "未命名人物"}
                    </span>
                    <span className="tabular mt-0.5 text-2xs text-ink-3">
                      {formatCount(person.faceCount)} 张人脸
                      <span className="mx-1 text-ink-3">·</span>
                      {formatCount(person.assetCount)} 张照片
                    </span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
    </div>
  );
}

/**
 * 模型提示条。这条提示对应 ADR-0002：
 * 识别权重不进安装包，用户自己下载、自己接受许可。
 * 非商用模型必须在这里显式提示，不能只在文档里写一句。
 */
function ModelNotice({
  model,
  detector,
  modelsReady,
}: {
  model: ModelEntry | undefined;
  detector: ModelEntry | undefined;
  modelsReady: boolean;
}) {
  if (!modelsReady) return null;

  return (
    <div className="mb-4 flex items-start gap-3 rounded-md border border-line bg-surface p-3">
      <Warning size={18} className="mt-0.5 shrink-0 text-accent" />
      <div className="min-w-0 flex-1">
        <p className="text-sm text-ink">识别权重未就绪</p>
        <p className="mt-1 text-xs leading-relaxed text-ink-2">
          检测出人脸不等于能把人归到一起，聚类还需要识别与嵌入环节的权重。
          {model
            ? `${model.name}（${model.license}${model.commercialUse ? "" : "，仅限非商用"}）。`
            : "注册表里还没有登记识别权重。"}
          {detector
            ? detector.status === "ready"
              ? `${detector.name}（检测器）已就绪。`
              : `${detector.name}（检测器）也还没下完。`
            : "检测器同样需要一个独立权重。"}
        </p>
        <p className="mt-1 text-xs leading-relaxed text-ink-3">
          Lumenfold 不分发任何模型权重：许可光净的在设置里下载，
          许可不光净的只登记元数据、不提供下载入口，由使用者自行下载后放到模型目录。
        </p>
      </div>
      <Button size="sm" variant="secondary" onClick={() => ui.setView("settings")}>
        去设置
      </Button>
    </div>
  );
}
