import { useState } from "react";
import { CheckCircle, Circle, DownloadSimple, HardDrives, Moon, SealCheck, Sun, Warning, X } from "@phosphor-icons/react";
import { Button } from "../components/Button";
import { LibraryPanel } from "../components/LibraryPanel";
import { Segmented } from "../components/Segmented";
import { ErrorState } from "../components/States";
import { cancelModelDownload, clearThumbCache, dataSource, setThumbCacheLimit, startModelDownload, useLibraryStats, useModels, type AsyncResource } from "../state/library";
import { formatBytes } from "../lib/format";
import { describeError } from "../lib/ipc";
import { ui, useUi } from "../state/ui";
import type { LibraryStats, ModelEntry } from "../lib/types";

export function SettingsView() {
  const theme = useUi((s) => s.theme);
  const stats = useLibraryStats();

  return (
    <div className="h-full min-h-0 overflow-y-auto scroll-area">
      <div className="mx-auto w-full max-w-[880px] px-5 py-6">
        <section className="mb-8">
          <h2 className="text-md font-medium tracking-tight text-ink">图库</h2>
          <LibraryPanel stats={stats} />
        </section>

        <section className="mb-8">
          <h2 className="text-md font-medium tracking-tight text-ink">外观</h2>
          <div className="mt-3 flex items-center gap-3">
            <Segmented
              label="主题"
              value={theme}
              onChange={(next) => ui.setTheme(next)}
              options={[
                { value: "dark", label: "暗色", icon: <Moon size={13} /> },
                { value: "light", label: "亮色", icon: <Sun size={13} /> },
              ]}
            />
            <p className="text-xs text-ink-3">
              主题锁定在整页，不做局部反转；插件读取同一套 CSS 变量，会跟着一起变。
            </p>
          </div>
        </section>

        <section className="mb-8">
          <h2 className="text-md font-medium tracking-tight text-ink">模型</h2>
          <p className="mt-1.5 max-w-[65ch] text-xs leading-relaxed text-ink-2">
            Lumenfold 不在安装包里带任何模型权重。每个模型都有各自的许可，下载之前会先显示许可，
            下载完成后校验 SHA-256 才会被标记为可用。非商用模型只能由使用者自己下载、自己承担用途。
          </p>
          {dataSource.kind === "demo" ? (
            <p className="mt-2 rounded-sm border border-line bg-sunken p-2 text-xs leading-relaxed text-ink-3">
              当前是浏览器演示模式，没有 Rust 宿主。下面的清单、体积与校验值都是示例数据，
              真实清单由 apps/desktop/src-tauri/src/commands.rs 的模型注册表提供。
            </p>
          ) : null}
          <ModelRegistry />
        </section>

        <section>
          <h2 className="text-md font-medium tracking-tight text-ink">存储</h2>
          <StoragePanel stats={stats} />
        </section>
      </div>
    </div>
  );
}

/** 与宿主 `thumbstore` 一致的档位。10 GB 不够——一个两万张的图库扫一遍就装满了。 */
const CACHE_LIMIT_OPTIONS = [16, 32, 64, 128, 256] as const;

const GIB = 1024 * 1024 * 1024;

/**
 * 缩略图缓存的占用与预算。
 *
 * 这一节以前只有一句「上限默认 10GB，超出按最近最少使用淘汰」，而当时：
 * 缓存只活在内存里、进程一退出就没了；上限是个固定条目数，跟字节数无关；
 * 淘汰策略也不是 LRU。所以那句话里三个词，没一个兑现得了。
 *
 * 现在缓存是磁盘上真实的一份文件，`cacheLimitBytes` 是宿主真正会拿去比大小的
 * 那个数，改完立刻生效；淘汰按写入时间从最旧的开始，所以文案也不再说「最近最少使用」。
 */
function StoragePanel({ stats }: { stats: AsyncResource<LibraryStats> }) {
  const [pending, setPending] = useState(false);
  const [message, setMessage] = useState<string | null>(null);

  if (stats.status !== "ready") {
    return (
      <div className="mt-3 flex items-start gap-3 rounded-md border border-line bg-surface p-3">
        <HardDrives size={18} className="mt-0.5 shrink-0 text-ink-3" />
        <p className="text-xs text-ink-3">读取存储信息中</p>
      </div>
    );
  }

  const { cacheBytes, cacheLimitBytes } = stats.value;
  // 用户刚点完还没生效时，props 里还是旧值。用 pending 挡住重复点击，
  // 而不是拿本地状态去假装上限已经改了——那样下次刷新就会露馅。
  const nearLimit = cacheLimitBytes > 0 && cacheBytes >= cacheLimitBytes * 0.9;

  async function run(action: () => Promise<LibraryStats>, note: string) {
    setPending(true);
    setMessage(null);
    try {
      await action();
      // 命令本身已经把刷过的统计带回来了，但 `stats` 是一条轮询的异步资源，
      // 不重新问一次，界面上就还是改之前的那个数字——那正好是用户最不能忍的一种界面。
      // 用 refresh 而不是 reload：这一下不该让整块面板闪回「读取中」。
      stats.refresh();
      setMessage(note);
    } catch (error) {
      setMessage(describeError(error));
    } finally {
      setPending(false);
    }
  }

  return (
    <div className="mt-3 rounded-md border border-line bg-surface p-3">
      <div className="flex items-start gap-3">
        <HardDrives size={18} className="mt-0.5 shrink-0 text-ink-3" />
        <div className="min-w-0 flex-1">
          <p className="text-xs leading-relaxed text-ink-2">
            缩略图缓存{" "}
            <span className="tabular text-ink">{formatBytes(cacheBytes)}</span>
            {cacheLimitBytes > 0 ? (
              <>
                {" "}
                / 上限 <span className="tabular text-ink">{formatBytes(cacheLimitBytes)}</span>
              </>
            ) : null}
            ，超出后从最旧写入的开始删。
          </p>
          <p className="mt-1 text-xs leading-relaxed text-ink-3">
            缓存在本机磁盘上，关掉 Lumenfold 也在，下次打开直接复用。原图始终留在你指定的目录里，
            缓存可以随时整块重建，不会影响原图。
          </p>

          <div className="mt-3">
            <Segmented
              label="缓存上限"
              value={String(gibFromBytes(cacheLimitBytes))}
              disabled={pending}
              onChange={(next) => void run(() => setThumbCacheLimit(Number(next) * GIB), "上限已更新")}
              options={CACHE_LIMIT_OPTIONS.map((gib) => ({
                value: String(gib),
                label: `${gib} GiB`,
              }))}
            />
          </div>

          <div className="mt-3 flex items-center gap-3">
            <Button
              size="sm"
              disabled={pending || cacheBytes === 0}
              onClick={() => void run(clearThumbCache, "缓存已清空，下次打开时重新生成")}
            >
              清空缩略图缓存
            </Button>
            {nearLimit ? (
              <span className="inline-flex items-center gap-1 text-2xs text-warn">
                <Warning size={12} />
                已经接近上限，旧的缩略图正在被删掉
              </span>
            ) : null}
            {message ? <span className="text-2xs text-ink-3">{message}</span> : null}
          </div>

          <p className="mt-2 text-2xs text-ink-3">
            {cacheBytes === 0 ? "缓存现在是空的。" : null}
            清空只删缩略图，索引和原图都不受影响。
          </p>
        </div>
      </div>
    </div>
  );
}

/** 上限只会是 CACHE_LIMIT_OPTIONS 里的整数 GiB；认不出来的值显示成默认值。 */
function gibFromBytes(bytes: number): number {
  const gib = Math.round(bytes / GIB);
  return (CACHE_LIMIT_OPTIONS as readonly number[]).includes(gib) ? gib : 64;
}

const STATUS_TEXT: Record<ModelEntry["status"], string> = {
  absent: "未下载",
  downloading: "下载中",
  ready: "已就绪",
  error: "下载失败",
};

function ModelRegistry() {
  const models = useModels();

  if (models.status === "error") {
    return (
      <div className="mt-4">
        <ErrorState
          title="读不到模型注册表"
          description="宿主没有返回模型清单。若已经建过索引，重试通常能恢复。"
          detail={models.message}
          onRetry={models.reload}
        />
      </div>
    );
  }

  if (models.status === "loading") {
    return (
      <div className="mt-4 flex flex-col gap-2">
        {[0, 1, 2].map((i) => (
          <div key={i} className="skeleton h-[92px] rounded-md" />
        ))}
      </div>
    );
  }

  if (models.value.length === 0) {
    return <p className="mt-3 text-xs text-ink-3">模型注册表是空的。</p>;
  }

  return (
    <ul className="mt-4 flex flex-col gap-2">
      {models.value.map((model) => (
        <li key={model.id}>
          <ModelCard model={model} />
        </li>
      ))}
    </ul>
  );
}

function ModelCard({ model }: { model: ModelEntry }) {
  const [pending, setPending] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  // 许可确认是点开一个对话框，不是一个 alert：ADR-0002 要求许可全文与商用标识
  // 在第一个字节之前呈现给用户，一个「确定」按钮做不到这件事。
  const [confirming, setConfirming] = useState(false);

  const ratio = model.sizeBytes === 0 ? 0 : (model.downloadedBytes / model.sizeBytes) * 100;
  const downloading = model.status === "downloading";
  const hasPartial = !downloading && model.status === "absent" && model.downloadedBytes > 0;

  async function run(action: () => Promise<unknown>) {
    setPending(true);
    setFailure(null);
    try {
      await action();
    } catch (error: unknown) {
      setFailure(describeError(error));
    } finally {
      setPending(false);
    }
  }

  return (
    <article className="rounded-md border border-line bg-surface p-3">
      <div className="flex flex-wrap items-start justify-between gap-3">
        <div className="min-w-0">
          <h3 className="flex items-center gap-2 text-sm font-medium text-ink">
            {model.status === "ready" ? (
              <CheckCircle size={16} weight="fill" className="shrink-0 text-success" />
            ) : (
              <Circle size={16} className="shrink-0 text-ink-3" />
            )}
            {model.name}
          </h3>
          <p className="mt-1 text-xs text-ink-3">{model.task}</p>
        </div>

        <div className="flex items-center gap-2">
          {/* 体积在第一次下载之前是未知的：清单没有固定，从 Content-Length 学。 */}
          <span className="tabular text-xs text-ink-3">
            {model.sizeBytes === 0 ? "大小未知" : formatBytes(model.sizeBytes)}
          </span>
          {model.status === "ready" ? (
            <span className="text-xs text-success">已就绪</span>
          ) : downloading ? (
            <Button size="sm" onClick={() => void run(() => cancelModelDownload(model.id))} disabled={pending}>
              取消
            </Button>
          ) : (
            // 所有模型都给下载入口：非商用权重也可下，但必须先弹出许可确认。
            <Button
              size="sm"
              variant={model.commercialUse ? "primary" : "danger"}
              icon={<DownloadSimple size={14} />}
              disabled={pending}
              onClick={() => setConfirming(true)}
            >
              {hasPartial ? "继续下载" : "下载"}
            </Button>
          )}
        </div>
      </div>

      {!model.commercialUse ? (
        <p className="mt-3 flex items-start gap-2 rounded-sm border border-accent-line bg-accent-soft p-2 text-xs leading-relaxed text-ink-2">
          <Warning size={14} className="mt-0.5 shrink-0 text-accent" />
          <span>
            该权重为非商用许可（{model.license}）。Lumenfold 的 AGPL 授权不会让模型权重变成可商用，
            用途由使用者自己承担。该权重会下到你自己的机器上，但它不能用来商业目的。
          </span>
        </p>
      ) : null}

      {failure ? (
        <p className="mt-2 rounded-sm border border-line bg-sunken p-2 text-xs leading-relaxed text-danger">
          {failure}
        </p>
      ) : null}

      {downloading ? (
        <div className="mt-3">
          <div className="h-1 w-full overflow-hidden rounded-full bg-surface-3">
            {/* 总长未知时按不确定进度画，否则会跳到 0% 然后卡住不动。 */}
            <div
              className="h-full bg-accent transition-[width] duration-(--lf-duration)"
              style={{ width: model.sizeBytes === 0 ? "100%" : `${ratio}%`, opacity: model.sizeBytes === 0 ? 0.4 : 1 }}
            />
          </div>
          <p className="tabular mt-1.5 text-2xs text-ink-3">
            {model.sizeBytes === 0
              ? `已下载 ${formatBytes(model.downloadedBytes)}`
              : `${formatBytes(model.downloadedBytes)} / ${formatBytes(model.sizeBytes)}`}
          </p>
        </div>
      ) : (
        <dl className="tabular mt-3 grid grid-cols-[4rem_1fr] gap-x-3 gap-y-1 text-2xs text-ink-3">
          <dt>许可</dt>
          <dd className="text-ink-2">{model.license}</dd>
          <dt>校验</dt>
          <dd className="selectable break-all text-ink-2">
            {model.sha256 === "" ? "清单未固定，下载器在下载时计算并记录" : model.sha256}
          </dd>
          <dt>来源</dt>
          <dd className="selectable break-all text-ink-2">
            {model.sourceUrl === "" ? "无下载入口" : model.sourceUrl}
            {model.commercialUse ? "" : "（非商用）"}
          </dd>
          <dt>状态</dt>
          <dd className="text-ink-2">
            {STATUS_TEXT[model.status]}
            {hasPartial ? `（已下载 ${formatBytes(model.downloadedBytes)}，可断点续传）` : ""}
          </dd>
        </dl>
      )}

      {confirming ? (
        <LicenseGate
          model={model}
          onCancel={() => setConfirming(false)}
          onAccept={() => {
            setConfirming(false);
            void run(() => startModelDownload(model.id, true));
          }}
        />
      ) : null}
    </article>
  );
}

/**
 * 许可确认闸门（ADR-0002 §4「许可先于首字节」）。
 *
 * 这一步不是走形式：宿主在收到 `licenseAccepted: true` 之前不会发出任何
 * HTTP 请求，并且会把这次接受连同时间戳写进模型目录下的 licenses.json。
 */
function LicenseGate({
  model,
  onCancel,
  onAccept,
}: {
  model: ModelEntry;
  onCancel(): void;
  onAccept(): void;
}) {
  return (
    <div
      className="fixed inset-0 z-50 grid place-items-center bg-black/45 p-6"
      role="dialog"
      aria-modal="true"
      aria-label={`下载 ${model.name} 的许可确认`}
      onClick={(event) => {
        if (event.target === event.currentTarget) onCancel();
      }}
    >
      <div className="w-full max-w-[460px] rounded-lg border border-line bg-surface p-4 shadow-xl">
        <div className="flex items-start justify-between gap-3">
          <h4 className="flex items-center gap-2 text-sm font-medium text-ink">
            <SealCheck size={16} className="shrink-0 text-accent" />
            下载前确认许可
          </h4>
          <button
            type="button"
            onClick={onCancel}
            aria-label="关闭"
            className="shrink-0 rounded-sm p-1 text-ink-3 transition-colors hover:bg-surface-2 hover:text-ink"
          >
            <X size={14} />
          </button>
        </div>

        <dl className="tabular mt-3 grid grid-cols-[4rem_1fr] gap-x-3 gap-y-1.5 text-xs">
          <dt className="text-ink-3">模型</dt>
          <dd className="text-ink-2">{model.name}</dd>
          <dt className="text-ink-3">用途</dt>
          <dd className="text-ink-2">{model.task}</dd>
          <dt className="text-ink-3">许可</dt>
          <dd className="text-ink-2">{model.license}</dd>
          <dt className="text-ink-3">商用</dt>
          <dd className="text-ink-2">{model.commercialUse ? "允许" : "不允许"}</dd>
        </dl>

        <p className="mt-3 rounded-sm border border-line bg-sunken p-2 text-2xs leading-relaxed text-ink-2">
          权重由你自己从下面这个地址下载到本机，Lumenfold 不分发任何模型文件。
          确认后宿主才会发出请求，并把这次接受与时间戳记进模型目录下的 licenses.json。
          下载中断后可以续传；下载完成后校验 SHA-256，只有校验通过才会标记为可用。
        </p>
        <p className="mt-2 break-all text-2xs text-ink-3">{model.sourceUrl}</p>

        <div className="mt-4 flex justify-end gap-2">
          <Button size="sm" onClick={onCancel}>
            取消
          </Button>
          <Button size="sm" variant="primary" onClick={onAccept}>
            接受并开始下载
          </Button>
        </div>
      </div>
    </div>
  );
}
