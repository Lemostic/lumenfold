import { useCallback, useState } from "react";
import {
  ArrowsClockwise,
  FolderSimple,
  MagnifyingGlass,
  Plus,
  Trash,
} from "@phosphor-icons/react";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { Button } from "./Button";
import {
  discoverLibraries,
  rescan,
  setLibraryRoots,
  type AsyncResource,
} from "../state/library";
import { describeError, hostAvailable } from "../lib/ipc";
import type { DiscoveryReport, LibraryStats } from "../lib/types";

/**
 * 图库目录管理。
 *
 * # 为什么是一组目录而不是一个
 *
 * 照片本来就不在一个地方：手机同步一份、相机导出到另一块盘、截图又落在系统图片目录。
 * 以前的图库是「一个根目录」，换位置就是把它换掉——等于强迫用户在几份照片之间
 * 二选一。现在图库像知识库：可以挂多个来源，每个来源各有自己的增量快照
 * （见 `library_index`，键就是目录本身），所以加一个目录不会让另外几个重扫一遍。
 *
 * # 自动发现只负责发现
 *
 * 「扫描所有有图片的目录」这件事本身没有正确答案——`C:\Users\me` 下面可能有照片，
 * 但把它整个挂成图库多半不是用户想要的。所以这里走一遍、数一数、把候选连同
 * **两个张数**交出去，采纳哪些由用户勾。发现过程不碰图库。
 */
export function LibraryPanel({ stats }: { stats: AsyncResource<LibraryStats> }) {
  const roots = stats.status === "ready" ? stats.value.roots : [];
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [report, setReport] = useState<DiscoveryReport | null>(null);
  const [picked, setPicked] = useState<ReadonlySet<string>>(new Set());
  const [depth, setDepth] = useState(3);

  const apply = useCallback(
    async (next: string[]) => {
      setBusy(true);
      setFailure(null);
      try {
        await setLibraryRoots(next);
        stats.refresh();
      } catch (error: unknown) {
        setFailure(describeError(error));
      } finally {
        setBusy(false);
      }
    },
    [stats],
  );

  const addFolder = useCallback(async () => {
    if (!hostAvailable) {
      setFailure("浏览器演示模式没有真实文件系统，改不了图库目录。");
      return;
    }
    const chosen = await openDialog({
      directory: true,
      multiple: false,
      title: "选择要加入图库的目录",
    });
    if (typeof chosen !== "string") return;
    if (roots.includes(chosen)) {
      setFailure("这个目录已经在图库里了。");
      return;
    }
    await apply([...roots, chosen]);
  }, [roots, apply]);

  const runDiscovery = useCallback(async () => {
    if (!hostAvailable) {
      setFailure("浏览器演示模式没有真实文件系统，发现不了目录。");
      return;
    }
    const base = await openDialog({
      directory: true,
      multiple: false,
      title: "从哪个目录开始找（通常会选「图片」或整个盘的用户目录）",
    });
    if (typeof base !== "string") return;
    setBusy(true);
    setFailure(null);
    try {
      const found = await discoverLibraries(base, depth);
      setReport(found);
      setPicked(new Set());
    } catch (error: unknown) {
      setFailure(describeError(error));
    } finally {
      setBusy(false);
    }
  }, [depth]);

  const addPicked = useCallback(async () => {
    const next = [...roots];
    for (const path of picked) {
      if (!next.includes(path)) next.push(path);
    }
    await apply(next);
    setPicked(new Set());
    setReport(null);
  }, [roots, picked, apply]);

  const toggle = useCallback((path: string) => {
    setPicked((previous) => {
      const next = new Set(previous);
      if (next.has(path)) {
        next.delete(path);
      } else {
        next.add(path);
      }
      return next;
    });
  }, []);

  return (
    <div className="mt-3">
      <p className="max-w-[65ch] text-xs leading-relaxed text-ink-2">
        Lumenfold 只读取你指定的目录，不会搬动或复制原图。可以同时挂多个目录：
        每个目录各有一份增量索引，加一个不会让另外几个重扫一遍。
      </p>

      {roots.length === 0 ? (
        <p className="mt-3 rounded-sm border border-line bg-sunken p-2 text-xs text-ink-3">
          还没有配过图库目录。加一个，或者用下面的自动发现找找。
        </p>
      ) : (
        <ul className="mt-3 flex flex-col gap-1.5">
          {roots.map((root) => (
            <li
              key={root}
              className="flex items-center gap-2 rounded-md border border-line bg-sunken px-2.5 py-1.5"
            >
              <FolderSimple size={14} className="shrink-0 text-ink-3" />
              <span className="selectable min-w-0 flex-1 truncate text-xs text-ink-2" title={root}>
                {root}
              </span>
              <Button
                size="sm"
                variant="ghost"
                disabled={busy}
                onClick={() => void apply(roots.filter((each) => each !== root))}
                icon={<Trash size={13} />}
              >
                移除
              </Button>
            </li>
          ))}
        </ul>
      )}

      <div className="mt-3 flex flex-wrap items-center gap-2">
        <Button size="sm" disabled={busy} onClick={() => void addFolder()} icon={<Plus size={13} />}>
          添加目录
        </Button>
        <Button
          size="sm"
          variant="ghost"
          disabled={busy || roots.length === 0}
          onClick={() => {
            void rescan().then(() => stats.refresh());
          }}
          icon={<ArrowsClockwise size={13} />}
        >
          重新扫描
        </Button>
      </div>

      <div className="mt-5 border-t border-line pt-4">
        <p className="text-xs font-medium text-ink">自动发现</p>
        <p className="mt-1 max-w-[65ch] text-xs leading-relaxed text-ink-2">
          从你选的目录往下找，列出所有含照片的目录，由你勾选要把哪些加进图库。
          两个数字分别是「这个目录自己直接有多少张」和「连同子目录一共多少张」——
          相册的根目录往往自己一张都不放，全在它下面。
        </p>
        <div className="mt-2.5 flex flex-wrap items-center gap-2">
          <label className="flex items-center gap-1.5 text-xs text-ink-2">
            往下找
            <select
              value={depth}
              onChange={(event) => setDepth(Number(event.currentTarget.value))}
              className="h-7 rounded-md border border-line bg-surface-2 px-1.5 text-xs text-ink"
            >
              {[2, 3, 4, 5, 6].map((value) => (
                <option key={value} value={value}>
                  {value} 层
                </option>
              ))}
            </select>
          </label>
          <Button
            size="sm"
            disabled={busy}
            onClick={() => void runDiscovery()}
            icon={<MagnifyingGlass size={13} />}
          >
            选择起点并开始发现
          </Button>
          {report !== null ? (
            <Button
              size="sm"
              variant="primary"
              disabled={busy || picked.size === 0}
              onClick={() => void addPicked()}
            >
              添加所选（{picked.size}）
            </Button>
          ) : null}
        </div>

        {report !== null ? (
          <div className="mt-3">
            {report.truncated ? (
              // 悄悄返回半个结果会让人以为「有照片的目录就这些」，那是在撒谎。
              <p className="rounded-sm border border-warn-line bg-warn-soft p-2 text-xs leading-relaxed text-warn">
                结果不完整：走到一半撞上了遍历上限（已看 {report.visited.toLocaleString("zh-CN")}{" "}
                项）。换一个更小的起点，或者把层数调小，再试一次。
              </p>
            ) : (
              <p className="text-xs text-ink-3">
                在 {report.base} 下看了 {report.visited.toLocaleString("zh-CN")} 项，找到{" "}
                {report.candidates.length} 个含照片的目录。
              </p>
            )}

            <ul className="mt-2 flex max-h-[320px] flex-col gap-1 overflow-y-auto scroll-area pr-1">
              {report.candidates.map((candidate) => (
                <li key={candidate.path}>
                  <label
                    className={[
                      "flex cursor-pointer items-center gap-2 rounded-md border px-2.5 py-1.5 transition-colors",
                      candidate.alreadyAdded
                        ? "border-line bg-sunken opacity-60"
                        : "border-line hover:bg-surface-2",
                    ].join(" ")}
                  >
                    <input
                      type="checkbox"
                      // 已经在图库里的目录显示成选中且禁用：让它可取消勾选会
                      // 指向「点一下就从图库删掉」，而那个动作该在「移除」里做。
                      checked={candidate.alreadyAdded || picked.has(candidate.path)}
                      disabled={candidate.alreadyAdded || busy}
                      onChange={() => toggle(candidate.path)}
                    />
                    <span className="min-w-0 flex-1">
                      <span className="block truncate text-xs text-ink" title={candidate.path}>
                        {candidate.name}
                      </span>
                      <span className="block truncate text-2xs text-ink-3" title={candidate.path}>
                        {candidate.path}
                      </span>
                    </span>
                    <span className="tabular shrink-0 text-right text-2xs text-ink-3">
                      {candidate.alreadyAdded ? (
                        "已在图库"
                      ) : (
                        <>
                          本级 {candidate.photos.toLocaleString("zh-CN")}
                          <br />共 {candidate.photosDeep.toLocaleString("zh-CN")} 张
                        </>
                      )}
                    </span>
                  </label>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>

      {failure ? (
        <p className="mt-3 rounded-sm border border-danger-line bg-danger-soft p-2 text-xs leading-relaxed text-danger">
          {failure}
        </p>
      ) : null}
    </div>
  );
}
