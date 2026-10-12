import { useCallback } from "react";
import {
  ArrowsDownUp,
  GridFour,
  MagnifyingGlass,
  SidebarSimple,
  SquaresFour,
} from "@phosphor-icons/react";
import { Slider } from "../components/Slider";
import { Button, IconButton } from "../components/Button";
import { Switch } from "../components/Switch";
import { BatchBar } from "../components/BatchBar";
import { useBatch } from "../state/batch";
import {
  ROW_HEIGHT_RANGE,
  WALL_CELL_RANGE,
  ui,
  useUi,
} from "../state/ui";
import type { SortKey } from "../lib/types";

const SORT_LABEL: Record<SortKey, string> = {
  "taken-desc": "拍摄时间 新到旧",
  "taken-asc": "拍摄时间 旧到新",
  name: "文件名",
  size: "文件大小",
  rating: "评分",
};

const SORT_ORDER: SortKey[] = ["taken-desc", "taken-asc", "name", "size", "rating"];

/**
 * 工具栏。
 *
 * 两种模式给的是两种工具栏，不是一种工具栏换文案：
 * 浏览模式下单击只是「看这一张」，所以工具栏保持排序/密度/视图这些浏览控件，
 * 不该冒出来「已标记 1 项 / 取消标记 / 复制 / 剪切」——点开一张照片就变成
 * 待批处理状态，是在替用户宣布一个他没做出的决定。
 * 只有标记模式下攒起东西，工具栏才整条换成批量条。
 *
 * 批量动作会真的改动磁盘上的文件,所以成功之后要重扫图库:
 * 资产 id 是路径的哈希,移动和删除都会让旧 id 失效。
 */
export function Toolbar({ onLibraryChanged }: { onLibraryChanged(): void }) {
  const sort = useUi((s) => s.sort);
  const rowHeight = useUi((s) => s.rowHeight);
  const wallCell = useUi((s) => s.wallCell);
  const layout = useUi((s) => s.layout);
  const wallGroups = useUi((s) => s.wallGroups);
  const markMode = useUi((s) => s.markMode);
  const selected = useUi((s) => s.selected);
  const inspectorOpen = useUi((s) => s.inspectorOpen);

  // 全选/反选都基于「已经加载出来的这一批」,而不是整个图库。
  const visibleIds = useUi((s) => s.visibleIds);
  const batch = useBatch([...selected], onLibraryChanged);

  const selectAll = useCallback(() => ui.selectAll(visibleIds), [visibleIds]);
  const invert = useCallback(() => ui.invertSelection(visibleIds), [visibleIds]);

  /**
   * 批量条只属于标记模式。
   *
   * 浏览模式下 `selected` 最多只有一张，含义是「正在看这一张」；
   * 让它触发批量条，工具栏就会为了一张根本没标记的照片宣布「已标记 1 项」。
   */
  if (markMode && selected.size > 0) {
    return (
      <BatchBar
        selectedCount={selected.size}
        busy={batch.busy}
        awaitingConfirm={batch.awaitingConfirm !== null}
        failure={batch.failure}
        report={batch.report}
        clipboardMessage={batch.clipboardMessage}
        onCopy={() => void batch.request("copy")}
        onMove={() => void batch.request("move")}
        onDelete={() => void batch.request("delete")}
        onConfirm={() => void batch.confirm()}
        onCancelConfirm={batch.cancel}
        onDismissReport={batch.dismissReport}
        onClearSelection={() => ui.clearSelection()}
        onSelectAll={selectAll}
        onInvert={invert}
        markMode={markMode}
        onToggleMarkMode={() => ui.setMarkMode(!markMode)}
      />
    );
  }

  return (
    <div className="flex h-11 shrink-0 items-center gap-2 border-b border-line bg-surface px-3">
      {/* 视图切换只在侧边栏出现一次。工具栏不再重复同一组入口。 */}
      <label className="inline-flex items-center gap-1.5 text-xs text-ink-3">
        <ArrowsDownUp size={14} />
        <span className="sr-only">排序方式</span>
        <select
          value={sort}
          onChange={(event) => ui.setSort(event.currentTarget.value as SortKey)}
          className="cursor-pointer rounded-md border border-transparent bg-transparent py-1 pr-1 pl-1.5 text-xs text-ink-2 transition-colors hover:border-line hover:bg-surface-2 hover:text-ink focus-visible:border-line-strong"
        >
          {SORT_ORDER.map((key) => (
            <option key={key} value={key}>
              {SORT_LABEL[key]}
            </option>
          ))}
        </select>
      </label>

      <div className="h-5 w-px bg-line" />

      <div
        role="group"
        aria-label="浏览方式"
        className="flex items-center rounded-md border border-line p-0.5"
      >
        <button
          type="button"
          aria-pressed={layout === "grid"}
          onClick={() => ui.setLayout("grid")}
          className={[
            "inline-flex h-6 items-center gap-1 rounded-xs px-1.5 text-xs transition-colors",
            layout === "grid"
              ? "bg-surface-3 text-ink"
              : "text-ink-3 hover:bg-surface-2 hover:text-ink-2",
          ].join(" ")}
        >
          <GridFour size={13} />
          网格
        </button>
        <button
          type="button"
          aria-pressed={layout === "wall"}
          onClick={() => ui.setLayout("wall")}
          className={[
            "inline-flex h-6 items-center gap-1 rounded-xs px-1.5 text-xs transition-colors",
            layout === "wall"
              ? "bg-surface-3 text-ink"
              : "text-ink-3 hover:bg-surface-2 hover:text-ink-2",
          ].join(" ")}
        >
          <SquaresFour size={13} />
          照片墙
        </button>
      </div>

      {layout === "wall" ? (
        <>
          <Slider
            label="格子"
            value={wallCell}
            min={WALL_CELL_RANGE.min}
            max={WALL_CELL_RANGE.max}
            step={16}
            valueText={`${wallCell}`}
            onChange={ui.setWallCell}
          />
          <Button
            size="sm"
            variant="ghost"
            aria-pressed={wallGroups}
            onClick={() => ui.setWallGroups(!wallGroups)}
          >
            {wallGroups ? "按月份分组" : "不分组"}
          </Button>
        </>
      ) : (
        <Slider
          label="密度"
          value={rowHeight}
          min={ROW_HEIGHT_RANGE.min}
          max={ROW_HEIGHT_RANGE.max}
          step={20}
          valueText={`${rowHeight}`}
          onChange={ui.setRowHeight}
        />
      )}

      {/*
        问题 4:标记模式改成 Switch 开关样式。开关的视觉特征明确表达「两种互斥模式」,
        比起之前可点可按的「标记模式」按钮,一眼就能看出当前在哪个模式。
        onLabel/offLabel 让用户即便不读 tooltip 也能从文字描述里看出来。
      */}
      <Switch
        label="切换标记模式"
        on={markMode}
        onChange={ui.setMarkMode}
        onLabel="标记模式"
        offLabel="浏览模式"
      />

      <div className="flex-1" />

      <IconButton label="搜索(/)" onClick={() => ui.setView("search")}>
        <MagnifyingGlass size={16} />
      </IconButton>

      {/*
        问题 7:检查器打开时,工具栏里这一处折叠按钮让位给检查器标题栏里的。
        之前两者同框,「2 个折叠按钮图标」就是这么来的——它们其实是同一动作。
      */}
      {!inspectorOpen ? (
        <IconButton
          label="展开检查器"
          onClick={() => ui.toggleInspector()}
        >
          <SidebarSimple size={16} />
        </IconButton>
      ) : null}
    </div>
  );
}
