import { Moon, Prohibit, Sun } from "@phosphor-icons/react";
import { formatBytes, formatCount } from "../lib/format";
import { ui, useUi } from "../state/ui";
import type { LibraryStats } from "../lib/types";
import { libraryLabel } from "../lib/types";

interface StatusBarProps {
  stats: LibraryStats | null;
  /** 演示图库在浏览器里跑,状态栏必须让人知道看到的数据不是真的 */
  demoMode: boolean;
  onCancelScan?(): void;
  onRescan?(): void;
}

/**
 * 状态栏。数字一律用等宽表格数字,扫读时列能对齐。
 * 演示模式必须显式说明,否则用户会把占位图库当成真实索引结果。
 *
 * 扫描进行中时,「取消」就摆在这一行:全应用只有这一个地方能叫停扫描,
 * 放在工具栏里会被批量操作条顶掉,放在标题栏又会和窗口按钮抢注意力。
 *
 * v2:左侧三个分栏用 1px 分隔线分开,字号统一到 11px,数字走 tabular。
 */
export function StatusBar({ stats, demoMode, onCancelScan, onRescan }: StatusBarProps) {
  const selectedCount = useUi((s) => s.selected.size);
  const theme = useUi((s) => s.theme);

  const scanning = stats?.scanning === true;
  const cancelled = stats?.cancelled === true;

  return (
    <footer className="flex h-7 shrink-0 items-center gap-3 border-t border-line bg-surface px-3 text-2xs text-ink-3">
      {demoMode ? (
        <span className="rounded-full border border-accent-line bg-accent-soft px-2 py-px text-accent">
          演示图库
        </span>
      ) : null}

      <span className="tabular">
        {stats && stats.roots.length > 0
          ? `${formatCount(stats.assetCount)} 项`
          : "未打开图库"}
      </span>

      {scanning ? (
        <>
          <span className="h-3 w-px bg-line" />
          <span className="flex items-center gap-1.5 text-accent">
            <span className="relative flex h-1.5 w-1.5">
              <span className="absolute inset-0 animate-ping rounded-full bg-accent opacity-60" />
              <span className="relative h-1.5 w-1.5 rounded-full bg-accent" />
            </span>
            正在扫描目录
          </span>
          {onCancelScan ? (
            <button
              type="button"
              onClick={onCancelScan}
              className="inline-flex items-center gap-1 rounded-xs px-1 py-px text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink"
              title="停止扫描。已经扫到的照片会保留下来。"
            >
              <Prohibit size={11} />
              取消
            </button>
          ) : null}
        </>
      ) : cancelled ? (
        <>
          <span className="h-3 w-px bg-line" />
          <span className="text-ink-2">扫描已取消,已保留上面这些照片</span>
          {onRescan ? (
            <button
              type="button"
              onClick={onRescan}
              className="rounded-xs px-1 py-px text-accent transition-colors hover:underline"
            >
              继续扫描
            </button>
          ) : null}
        </>
      ) : null}

      <span className="h-3 w-px bg-line" />

      <span className="tabular">已选 {formatCount(selectedCount)}</span>

      <span className="h-3 w-px bg-line" />

      <span className="tabular">缓存 {stats ? formatBytes(stats.cacheBytes) : "0 B"}</span>

      {/* 增量扫描到底跑没跑，得让人能看见。「上次扫描」那一句只在真的
          扫过一遍、而且至少复用过一张的时候才出现——没有快照时它没话可说。 */}
      {!scanning && stats && !demoMode && (stats.scanReused > 0 || stats.scanProbed > 0) ? (
        <>
          <span className="h-3 w-px bg-line" />
          <span
            className="tabular"
            title={`复用 ${stats.scanReused} 张，没有重新读取；重新读取 ${stats.scanProbed} 张。`}
          >
            复用 {formatCount(stats.scanReused)} / 重读 {formatCount(stats.scanProbed)}
          </span>
        </>
      ) : null}

      <div className="flex-1" />

      <span className="truncate" title={stats?.roots.join("\n")}>
        {/* 多个目录时只显示「第一个 等 N 个目录」：把五条路径全塞进状态栏
            只会把真正该看的东西挤没，完整列表在设置页里。 */}
        {libraryLabel(stats?.roots ?? []) ?? "未打开图库"}
      </span>

      <button
        type="button"
        onClick={() => ui.setTheme(theme === "dark" ? "light" : "dark")}
        aria-label={theme === "dark" ? "切换到亮色主题" : "切换到暗色主题"}
        title={theme === "dark" ? "切换到亮色主题" : "切换到暗色主题"}
        className="inline-flex h-5 w-5 items-center justify-center rounded-xs text-ink-3 transition-colors hover:bg-surface-2 hover:text-ink"
      >
        {theme === "dark" ? <Moon size={13} /> : <Sun size={13} />}
      </button>
    </footer>
  );
}
