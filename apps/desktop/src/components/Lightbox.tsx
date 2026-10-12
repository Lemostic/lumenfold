import { useEffect, useRef, useState } from "react";
import {
  CornersOut,
  CornersIn,
  Sparkle,
  X,
  ArrowCounterClockwise,
  ArrowsHorizontal,
  ArrowsVertical,
} from "@phosphor-icons/react";
import { IconButton } from "./Button";
import { originalUrlFor, useAssetMetadata } from "../state/library";
import { formatDateTime, formatMegapixels } from "../lib/format";
import { convertFileSrc } from "@tauri-apps/api/core";
import { bridge, hostAvailable } from "../lib/ipc";
import { EnhancePanel } from "./EnhancePanel";
import type { DemoAsset } from "../lib/fixtures";
import type { EnhancementTask } from "../lib/types";

interface LightboxProps {
  asset: DemoAsset | null;
  /** 同集合里前后资产,用于左右键切换 */
  prev: DemoAsset | null;
  next: DemoAsset | null;
  onClose(): void;
  onPrev(): void;
  onNext(): void;
}

/**
 * 最大化原图查看器。
 *
 * 问题 3:网格模式下未开启标记模式时,双击照片弹出本组件。
 *
 * 编辑相关的旋转 / 镜像(问题 5)在这里加:提供 90° 旋转和水平/垂直镜像按钮,
 * 修改只发生在本地预览,需要用户确认保存覆盖原图或保存副本。
 */
export function Lightbox({ asset, prev, next, onClose, onPrev, onNext }: LightboxProps) {
  const [decodeFailed, setDecodeFailed] = useState(false);
  const [transform, setTransform] = useState<Transform>({ rotate: 0, flipH: false, flipV: false });
  const [confirming, setConfirming] = useState<ConfirmMode | null>(null);
  /**
   * 是否在保存时顺便增强画质。
   *
   * 它和旋转/镜像并列，但代价完全不同：那两个是毫秒级的像素搬运，
   * 这个是几分钟的模型推理。所以它自己的状态、自己的确认文案、自己的进度条。
   */
  const [enhance, setEnhance] = useState(false);
  // 主体区域的可视尺寸，用来决定旋转时该用哪条边作为上限。
  // 1fr 网格行不会把盒子的尺寸告诉后代，CSS 没有「跟着旋转交换 max-width/max-height」
  // 的写法，所以只能在 JS 里观察再写。
  const stageRef = useRef<HTMLDivElement>(null);
  const [stage, setStage] = useState({ w: 0, h: 0 });
  const [zoom, setZoom] = useState(1);
  const [enhanceTask, setEnhanceTask] = useState<EnhancementTask | null>(null);
  const [enhancedSrc, setEnhancedSrc] = useState<string | null>(null);

  const metadata = useAssetMetadata(asset?.id ?? null);

  // 关闭时清掉所有临时变换:这是「查看」而不是「编辑」,不能把旋转状态留在外面。
  useEffect(() => {
    // Navigation changes the asset without closing the viewer. Reset the
    // temporary edit state so a preview never leaks into the next photo.
    setTransform({ rotate: 0, flipH: false, flipV: false });
    setDecodeFailed(false);
    setConfirming(null);
    // 增强的开关同样是「这一张」的临时状态，不能带到下一张。
    setEnhance(false);
    setZoom(1);
    setEnhanceTask(null);
    setEnhancedSrc(null);
  }, [asset?.id]);

  useEffect(() => {
    if (!enhanceTask || !hostAvailable) return;
    const timer = window.setInterval(() => void bridge.enhancementTasks().then((rows) => {
      const next = rows.find((row) => row.id === enhanceTask.id);
      if (!next) return;
      setEnhanceTask(next);
      if (next.status === "succeeded" && next.previewPath) { setEnhancedSrc(convertFileSrc(next.previewPath)); window.clearInterval(timer); }
      if (next.status === "failed" || next.status === "cancelled") window.clearInterval(timer);
    }), 1000);
    return () => window.clearInterval(timer);
  }, [enhanceTask?.id, hostAvailable]);

  // 跟随 1fr 中间行的实际尺寸，每次窗口/字号/缩放变化都重新写入。
  useEffect(() => {
    const el = stageRef.current;
    if (!el) return;
    const update = () => setStage({ w: el.clientWidth, h: el.clientHeight });
    update();
    const ro = new ResizeObserver(update);
    ro.observe(el);
    const stopNativeWheel = (event: WheelEvent) => { if (!event.ctrlKey) event.preventDefault(); };
    el.addEventListener("wheel", stopNativeWheel, { passive: false });
    return () => { ro.disconnect(); el.removeEventListener("wheel", stopNativeWheel); };
  }, []);

  useKeyboardNav(asset !== null, onPrev, onNext, onClose, transform, setTransform);

  if (asset === null) return null;

  const showPlaceholder = asset.width === 0 || asset.height === 0 || decodeFailed;
  const transformStyle = transformToCss(transform);
  const zoomStyle: React.CSSProperties = {
    ...transformStyle,
    transform: `${transformStyle.transform ?? ""} scale(${zoom})`.trim(),
  };
  // 90°/270° 时可视宽高互换：把可视的「高度」塞进 stage 实际给的「宽度」上限里，
  // 否则 transform 之后会被 stage 切掉一截——旋转时也是最常见被裁的场景。
  const swapAxesForRotation = transform.rotate === 90 || transform.rotate === 270;
  const stageMaxStyle: React.CSSProperties = swapAxesForRotation
    ? { maxWidth: stage.h || undefined, maxHeight: stage.w || undefined }
    : {};

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={`放大查看 ${asset.fileName}`}
      className="fixed inset-0 z-[var(--lf-z-dialog)] grid grid-rows-[auto_1fr_auto] bg-overlay backdrop-blur-md"
      onClick={(event) => {
        // 点击背景关闭,点击工具栏/图本身不关闭。
        if (event.target === event.currentTarget) onClose();
      }}
    >
      <header className="drag-region flex h-11 shrink-0 items-center gap-3 border-b border-line bg-surface/85 px-3 backdrop-blur-md">
        <h2 className="min-w-0 truncate text-sm font-medium text-ink">{asset.fileName}</h2>
        <span className="tabular shrink-0 text-2xs text-ink-3">
          {asset.width} × {asset.height} · {formatMegapixels(asset.width, asset.height)} · {formatDateTime(asset.takenAt)}
        </span>
        <div className="flex-1" />
        <IconButton
          label="向左旋转 90°"
          onClick={() => setTransform((t) => ({ ...t, rotate: ((t.rotate - 90) % 360 + 360) % 360 }))}
        >
          <ArrowCounterClockwise size={16} />
        </IconButton>
        <IconButton
          label="水平镜像"
          aria-pressed={transform.flipH}
          onClick={() => setTransform((t) => ({ ...t, flipH: !t.flipH }))}
        >
          <ArrowsHorizontal size={16} />
        </IconButton>
        <IconButton
          label="垂直镜像"
          aria-pressed={transform.flipV}
          onClick={() => setTransform((t) => ({ ...t, flipV: !t.flipV }))}
        >
          <ArrowsVertical size={16} />
        </IconButton>
        <IconButton
          label={enhanceTask?.status === "running" ? "照片增强处理中" : enhancedSrc ? "已显示增强结果" : "立即增强照片"}
          aria-pressed={Boolean(enhanceTask)}
          disabled={Boolean(enhanceTask) || !hostAvailable}
           onClick={() => { if (!hostAvailable) return; void bridge.startEnhancement(asset.id).then(setEnhanceTask).catch(() => undefined); }}
        >
          <Sparkle size={16} />
        </IconButton>
        {hasTransform(transform) ? (
          <IconButton
            label="重置变换"
            onClick={() => setTransform({ rotate: 0, flipH: false, flipV: false })}
          >
            <CornersIn size={16} />
          </IconButton>
        ) : null}
        <IconButton label="关闭" onClick={onClose}>
          <X size={16} />
        </IconButton>
      </header>

      <div
        ref={stageRef}
        onWheel={(event) => {
          if (showPlaceholder || event.ctrlKey) return;
          setZoom((current) => Math.min(8, Math.max(1, Number((current * (event.deltaY < 0 ? 1.15 : 1 / 1.15)).toFixed(2)))));
        }}
        className="relative flex min-h-0 items-center justify-center overflow-hidden bg-sunken"
      >
        {showPlaceholder ? (
          <p className="text-sm text-ink-3">原图暂不可用</p>
        ) : (
          // 没有 `overflow-hidden`：旋转 90° 之后可视宽高会交换，需要的不是裁掉，
          // 而是让 box 跟着旋转走一段变成可容纳的方向。`max-w-full/max-h-full` 是常规
          // 情况下的上限；旋转 90°/270° 时用上面观察到的 stage 宽高反着喂上限。
          // `object-contain` 始终按原图的宽高比例压进盒子里，所以图本身不会变形。
          <img
            key={asset.id}
            src={enhancedSrc ?? originalUrlFor(asset)}
            alt={asset.fileName}
            draggable={false}
            onError={() => setDecodeFailed(true)}
            className="block max-w-full max-h-full object-contain"
            style={{ ...zoomStyle, ...stageMaxStyle }}
          />
        )}

        {zoom !== 1 ? (
          <button type="button" onClick={() => setZoom(1)} aria-label="恢复适应窗口大小" className="absolute right-4 bottom-4 rounded-md bg-popover px-2 py-1 text-xs tabular-nums text-ink shadow-pop">
            {Math.round(zoom * 100)}% · 重置
          </button>
        ) : null}
        {prev ? (
          <button
            type="button"
            aria-label="上一张"
            onClick={onPrev}
            className="absolute top-1/2 left-3 -translate-y-1/2 flex h-9 w-9 items-center justify-center rounded-full bg-popover text-ink shadow-pop transition-opacity hover:text-accent"
          >
            ‹
          </button>
        ) : null}
        {next ? (
          <button
            type="button"
            aria-label="下一张"
            onClick={onNext}
            className="absolute top-1/2 right-3 -translate-y-1/2 flex h-9 w-9 items-center justify-center rounded-full bg-popover text-ink shadow-pop transition-opacity hover:text-accent"
          >
            ›
          </button>
        ) : null}
      </div>

      <footer className="flex h-11 shrink-0 items-center justify-between gap-3 border-t border-line bg-surface/85 px-3 backdrop-blur-md">
        <div className="flex items-center gap-3 text-2xs text-ink-3">
          {metadata.status === "ready" && metadata.value ? (
            <span className="tabular">
              {[metadata.value.cameraMake, metadata.value.cameraModel].filter(Boolean).join(" ") || "未知机身"}
              {metadata.value.lens ? ` · ${metadata.value.lens}` : ""}
              {metadata.value.aperture ? ` · f/${metadata.value.aperture}` : ""}
            </span>
          ) : (
            <span>拍摄参数载入中…</span>
          )}
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            disabled={!hasTransform(transform) && !enhance}
            onClick={() => setConfirming("copy")}
            className="inline-flex h-7 items-center gap-1.5 rounded-md border border-line bg-surface-2 px-2.5 text-xs text-ink-2 transition-colors hover:bg-surface-3 hover:text-ink disabled:cursor-not-allowed disabled:opacity-40"
          >
            保存副本
          </button>
          <button
            type="button"
            disabled={!hasTransform(transform) && !enhance}
            onClick={() => setConfirming("overwrite")}
            className="inline-flex h-7 items-center gap-1.5 rounded-md bg-accent px-2.5 text-xs font-medium text-accent-ink transition-colors hover:bg-accent-hover disabled:cursor-not-allowed disabled:opacity-40"
          >
            <CornersOut size={13} />
            保存覆盖
          </button>
        </div>
      </footer>

      {confirming !== null ? (
        <SaveDialog
          mode={confirming}
          asset={asset}
          transform={transform}
          enhance={enhance}
          onCancel={() => setConfirming(null)}
          onSaved={() => {
            setConfirming(null);
            setTransform({ rotate: 0, flipH: false, flipV: false });
            setEnhance(false);
          }}
        />
      ) : null}
    </div>
  );
}

interface Transform {
  rotate: number;
  flipH: boolean;
  flipV: boolean;
}

function hasTransform(t: Transform): boolean {
  return t.rotate !== 0 || t.flipH || t.flipV;
}

function transformToCss(t: Transform): React.CSSProperties {
  const transforms: string[] = [];
  if (t.rotate !== 0) transforms.push(`rotate(${t.rotate}deg)`);
  if (t.flipH || t.flipV) {
    transforms.push(`scale(${t.flipH ? -1 : 1}, ${t.flipV ? -1 : 1})`);
  }
  if (transforms.length === 0) return {};
  return { transform: transforms.join(" ") };
}

type ConfirmMode = "overwrite" | "copy";

interface SaveDialogProps {
  mode: ConfirmMode;
  asset: DemoAsset;
  transform: Transform;
  /** 保存时是否顺便跑一次画质增强 */
  enhance: boolean;
  onCancel(): void;
  onSaved(): void;
}

/**
 * 保存确认对话框(问题 5)。
 *
 * 旋转/镜像的「应用」是不可逆操作:
 * - 覆盖原图:用户接受丢失原方向/翻转
 * - 保存副本:原图保留,新文件以 _rotated / _mirrored 后缀落到同目录
 *
 * 必须二次确认,不能让旋转按钮就那么直接写入磁盘。
 *
 * 画质增强叠在同一套确认里,但它多了一件事:代价。旋转是毫秒级的,增强是分钟级的,
 * 所以开着增强时这里会额外说明预计耗时、显示进度、并提供取消。
 */
function SaveDialog({ mode, asset, transform, enhance, onCancel, onSaved }: SaveDialogProps) {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  const description =
    mode === "overwrite"
      ? enhance
        ? "原图将被新方向/镜像覆盖,并且就地做一次画质增强。这个动作不可撤销——如果你想保留原图,请选保存副本。"
        : "原图将被新方向/镜像覆盖。这个动作不可撤销——如果你想保留原图,请选保存副本。"
      : enhance
        ? "原图保留,新文件按方向与镜像后的结果写到同目录,文件名带上 _rotated / _mirrored / _enhanced 后缀。增强的全部耗时都花在这个副本上,原图一个字节都不会动。"
        : "原图保留,新文件按方向与镜像后的结果写到同目录,文件名带上 _rotated / _mirrored 后缀。";

  return (
    <div
      className="fixed inset-0 z-[var(--lf-z-toast)] grid place-items-center bg-overlay p-6 backdrop-blur-sm"
      role="dialog"
      aria-modal="true"
      aria-label="确认保存"
      onClick={(event) => {
        if (event.target === event.currentTarget && !busy) onCancel();
      }}
    >
      <div className="chrome-modal w-full max-w-[460px] rounded-lg p-4">
        <h3 className="text-md font-medium tracking-tight text-ink">
          {mode === "overwrite" ? "保存覆盖到原图" : "保存为副本"}
        </h3>
        <p className="mt-2 text-sm leading-relaxed text-ink-2 text-pretty">{description}</p>

        <ul className="mt-3 flex flex-col gap-1.5 rounded-md border border-line bg-sunken p-3 text-xs text-ink-2">
          <li className="flex items-baseline justify-between gap-3">
            <span className="text-ink-3">文件</span>
            <span className="selectable truncate">{asset.fileName}</span>
          </li>
          <li className="flex items-baseline justify-between gap-3">
            <span className="text-ink-3">变换</span>
            <span className="tabular text-ink-2">{describeTransform(transform)}</span>
          </li>
          {enhance ? (
            <li className="flex items-baseline justify-between gap-3">
              <span className="text-ink-3">增强</span>
              <span className="text-ink-2">开</span>
            </li>
          ) : null}
        </ul>

        {enhance ? (
          <EnhancePanel width={asset.width} height={asset.height} busy={busy} />
        ) : null}

        {failure ? (
          <p className="mt-3 rounded-md border border-danger-line bg-danger-soft p-2 text-xs text-danger">
            {failure}
          </p>
        ) : null}

        <div className="mt-4 flex justify-end gap-2">
          <button
            type="button"
            onClick={onCancel}
            disabled={busy}
            className="inline-flex h-8 items-center rounded-md px-3 text-sm text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink disabled:opacity-50"
          >
            取消
          </button>
          <button
            type="button"
            onClick={async () => {
              setBusy(true);
              setFailure(null);
              try {
                await saveTransform(asset, transform, mode, enhance);
                onSaved();
              } catch (error: unknown) {
                setFailure(String(error));
              } finally {
                setBusy(false);
              }
            }}
            disabled={busy}
            className={[
              "inline-flex h-8 items-center rounded-md px-3 text-sm font-medium transition-colors disabled:opacity-50",
              mode === "overwrite"
                ? "bg-accent text-accent-ink hover:bg-accent-hover"
                : "bg-surface-2 text-ink hover:bg-surface-3",
            ].join(" ")}
          >
            {busy ? "正在保存…" : mode === "overwrite" ? "确认覆盖" : "确认保存副本"}
          </button>
        </div>
      </div>
    </div>
  );
}

function describeTransform(t: Transform): string {
  const parts: string[] = [];
  if (t.rotate !== 0) parts.push(`旋转 ${t.rotate}°`);
  if (t.flipH) parts.push("水平镜像");
  if (t.flipV) parts.push("垂直镜像");
  return parts.length === 0 ? "无变换" : parts.join(" · ");
}

/**
 * 调用宿主命令保存变换。
 *
 * 真实宿主在 src-tauri 里实现具体逻辑,这里走 ui.store 注入的 handler,
 * 演示模式抛错,被 SaveDialog 捕获并显示。
 *
 * 设计上让 host/IPC 决定如何落盘:旋转和镜像会改 EXIF Orientation 还是
 * 像素本身,这是产品的策略问题,不放在前端。
 */
async function saveTransform(
  asset: DemoAsset,
  transform: Transform,
  mode: ConfirmMode,
  enhance: boolean,
): Promise<void> {
  // 演示模式:通过 window 全局注入的 host handler 找不到时,主动抛错。
  const handler = (window as unknown as { __lumenfoldEdit?: EditHandler }).__lumenfoldEdit;
  if (typeof handler !== "function") {
    throw new Error("当前运行模式没有图片编辑能力,无法保存");
  }
  await handler({ assetId: asset.id, transform, mode, enhance });
}

interface EditRequest {
  assetId: string;
  transform: Transform;
  mode: ConfirmMode;
  /** 在几何变换之后再做一次画质增强；老调用方不传就是 false */
  enhance?: boolean;
}

type EditHandler = (req: EditRequest) => Promise<void>;

/**
 * 注入 host 编辑能力。在 useOpenLibrary 或 main.tsx 里调用一次。
 * 真实模式下由 Tauri 的 Rust 命令桥接,演示模式下抛错。
 */
export function bindEditHandler(handler: EditHandler | null): void {
  (window as unknown as { __lumenfoldEdit?: EditHandler }).__lumenfoldEdit =
    handler ?? undefined;
}

function useKeyboardNav(
  active: boolean,
  onPrev: () => void,
  onNext: () => void,
  onClose: () => void,
  transform: Transform,
  setTransform: React.Dispatch<React.SetStateAction<Transform>>,
) {
  const prev = useRef(onPrev);
  const next = useRef(onNext);
  const close = useRef(onClose);
  const setT = useRef(setTransform);
  prev.current = onPrev;
  next.current = onNext;
  close.current = onClose;
  setT.current = setTransform;

  useEffect(() => {
    if (!active) return;
    const handler = (event: KeyboardEvent) => {
      // 目标在输入框里时,把 Lightbox 的快捷键让出来。
      const target = event.target;
      if (target instanceof HTMLElement) {
        const tag = target.tagName;
        if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return;
      }
      if (event.key === "ArrowLeft") {
        event.preventDefault();
        prev.current();
      } else if (event.key === "ArrowRight") {
        event.preventDefault();
        next.current();
      } else if (event.key === "Escape") {
        event.preventDefault();
        close.current();
      } else if (event.key.toLowerCase() === "r") {
        event.preventDefault();
        setT.current((t) => ({ ...t, rotate: ((t.rotate + 90) % 360 + 360) % 360 }));
      } else if (event.key.toLowerCase() === "m") {
        event.preventDefault();
        setT.current((t) => ({ ...t, flipH: !t.flipH }));
      } else if (event.key.toLowerCase() === "v") {
        event.preventDefault();
        setT.current((t) => ({ ...t, flipV: !t.flipV }));
      }
    };
    window.addEventListener("keydown", handler);
    return () => window.removeEventListener("keydown", handler);
  }, [active, transform]);
}
