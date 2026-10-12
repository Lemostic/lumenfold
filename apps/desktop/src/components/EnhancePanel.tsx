import { useEffect, useState } from "react";
import { bridge, hostAvailable } from "../lib/ipc";
import type { EnhanceProgress } from "../lib/types";

/**
 * 保存前对画质增强的说明，以及保存中的进度。
 *
 * # 为什么值得单独一个组件
 *
 * 旋转和镜像是毫秒级的像素搬运，点完就完了。画质增强是一次模型推理，
 * 12MP 的照片要 8 到 9 分钟（CPU 回退时 20 分钟以上）。一个八分钟的操作
 * 如果只有一个「正在保存…」，用户没有任何办法判断它是在跑、卡住了、还是死了。
 * 所以这里给出三件事：**大概要多久**、**现在到哪了**、**怎么停下来**。
 *
 * # 这里直接走 bridge 而不是 window.__lumenfoldEdit
 *
 * Lightbox 的保存路径走注入的 handler 是为了让演示模式能如实报错。
 * 进度和取消是纯宿主能力——演示模式根本没有模型可跑——所以直接问 bridge，
 * 用 `hostAvailable` 挡住演示模式，而不是再往那个接口上挂两个方法。
 */
export function EnhancePanel({
  width,
  height,
  busy,
}: {
  /** 源图画素宽高；0 表示索引里读不到尺寸 */
  width: number;
  height: number;
  /** 保存请求已经在飞了 */
  busy: boolean;
}) {
  const [progress, setProgress] = useState<EnhanceProgress | null>(null);

  useEffect(() => {
    if (!busy || !hostAvailable) return;
    let alive = true;
    const tick = () => {
      bridge.enhanceProgress().then(
        (next) => {
          if (alive) setProgress(next);
        },
        () => {
          // 进度拿不到不是错误：任务本身还在跑。安静地保持上一次显示。
        },
      );
    };
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [busy]);

  if (!hostAvailable) {
    return (
      <p className="mt-3 rounded-md border border-line bg-sunken p-2 text-xs text-ink-3">
        演示模式没有可运行的增强模型。
      </p>
    );
  }

  const pixels = width * height;
  if (pixels === 0) {
    return (
      <p className="mt-3 rounded-md border border-line bg-sunken p-2 text-xs text-ink-3">
        这张照片在索引里读不到像素尺寸，暂时无法估计耗时。
      </p>
    );
  }

  return (
    <div className="mt-3 rounded-md border border-line bg-sunken p-3 text-xs">
      <p className="font-medium text-ink">画质增强</p>

      {!busy ? (
        <>
          <p className="mt-1 leading-relaxed text-ink-2 text-pretty">
            用超分模型放大两倍再缩回原尺寸：只恢复已有的细节，不改变构图、比例和内容。
          </p>
          <p className="tabular mt-1.5 text-ink-3">
            这张 {formatMegapixels(width, height)} 预计需要 {estimateRange(pixels)}
          </p>
          <p className="mt-1 text-ink-3">
            推理期间可以取消，取消后照片不会被改动。
          </p>
        </>
      ) : (
        <>
          <ProgressBar progress={progress} />
          <p className="tabular mt-1.5 text-ink-3">
            {progress?.provider === "unloaded" || progress == null
              ? "正在加载模型…"
              : `运行在 ${providerLabel(progress.provider)}`}
            {progress && progress.total > 0
              ? ` · ${progress.done.toLocaleString("zh-CN")} / ${progress.total.toLocaleString("zh-CN")} 块`
              : ""}
          </p>
          <button
            type="button"
            onClick={() => {
              // 不 await：取消是「请求停」，正在跑的那块瓦片会跑完再干净返回。
              // 界面靠上面的进度条自己变，不靠这个按钮的返回值。
              void bridge.cancelEnhance();
            }}
            className="mt-2 inline-flex h-7 items-center rounded-md border border-line bg-surface-2 px-2.5 text-xs text-ink-2 transition-colors hover:bg-surface-3 hover:text-ink"
          >
            取消增强
          </button>
        </>
      )}
    </div>
  );
}

function ProgressBar({ progress }: { progress: EnhanceProgress | null }) {
  const total = progress?.total ?? 0;
  const done = progress?.done ?? 0;
  // 还在建模（total 为 0）时给一条不确定的满格底色，而不是假装进度是 0%：
  // 9-11 秒的建模期里一个不动的 0% 看起来像是卡住了。
  const percent = total > 0 ? Math.min(100, Math.round((done / total) * 100)) : null;

  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={percent ?? undefined}
      aria-label="画质增强进度"
      className="mt-2 h-1.5 w-full overflow-hidden rounded-full bg-surface-3"
    >
      <div
        className={[
          "h-full rounded-full bg-accent transition-[width] duration-(--lf-duration)",
          percent === null ? "w-full animate-pulse" : "",
        ].join(" ")}
        style={percent === null ? undefined : { width: `${percent}%` }}
      />
    </div>
  );
}

function providerLabel(provider: string): string {
  if (provider === "directml") return "DirectML（GPU）";
  if (provider === "cpu") return "CPU（没有可用的 GPU 加速）";
  return provider;
}

/**
 * 已实测的每输入像素耗时（毫秒）。
 *
 * 这两个数是这台机器上量出来的（Swin2SR Lightweight ×2，瓦片 256），
 * 不是猜的：GPU 0.045、CPU 0.108。别的地方换硬件就会偏，
 * 所以文案里写的是「预计」而不是一个确定值。
 */
const MS_PER_PIXEL_DIRECTML = 0.045;
const MS_PER_PIXEL_CPU = 0.108;

function estimateRange(pixels: number): string {
  const fast = formatDuration(pixels * MS_PER_PIXEL_DIRECTML);
  const slow = formatDuration(pixels * MS_PER_PIXEL_CPU);
  return `${fast}（有 GPU 加速）到 ${slow}（回退到 CPU）`;
}

function formatDuration(ms: number): string {
  const seconds = Math.round(ms / 1000);
  if (seconds < 90) return `${seconds} 秒`;
  return `约 ${Math.round(seconds / 60)} 分钟`;
}

/** 只用于估算文案；真正的格式化在 lib/format 里。 */
function formatMegapixels(width: number, height: number): string {
  return `${(width * height / 1_000_000).toFixed(1)} MP`;
}
