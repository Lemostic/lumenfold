import { useEffect, useRef, useState } from "react";
import { ArrowsClockwise } from "@phosphor-icons/react";
import { TitleBar } from "./TitleBar";
import { Sidebar } from "./Sidebar";
import { Toolbar } from "./Toolbar";
import { StatusBar } from "./StatusBar";
import { Inspector } from "./Inspector";
import { SelectionPanel } from "./SelectionPanel";
import { NoLibraryState } from "./NoLibraryState";
import { BusyState, EmptyState, ErrorState } from "../components/States";
import { LibraryView } from "../views/LibraryView";
import { TimelineView } from "../views/TimelineView";
import { PeopleView } from "../views/PeopleView";
import { SearchView } from "../views/SearchView";
import { SettingsView } from "../views/SettingsView";
import { EnhancementTasksView } from "../views/EnhancementTasksView";
import { assetById, cancelScan, dataSource, rescan, useLibraryStats } from "../state/library";
import { bridge, hostAvailable } from "../lib/ipc";
import { ui, useUi } from "../state/ui";
import { libraryLabel } from "../lib/types";
import type { DemoAsset } from "../lib/fixtures";

/** 扫描进行中时的轮询间隔。扫描在后台线程，这里只负责把进度显示出来。 */
const SCAN_POLL_MS = 400;

export function AppShell() {
  const view = useUi((s) => s.view);
  const inspectorOpen = useUi((s) => s.inspectorOpen);
  const selectedCount = useUi((s) => s.selected.size);
  const markMode = useUi((s) => s.markMode);
  const selectionOrder = useUi((s) => s.selectionOrder);
  const stats = useLibraryStats();
  const reloadStats = stats.reload;
  const refreshStats = stats.refresh;

  // 首屏有四种互斥状态，之前只留了一个 hasLibrary 布尔量，
  // 于是「正在读库」「读库失败」「真的没有库」被画成同一张欢迎页。
  const ready = stats.status === "ready" ? stats.value : null;
  const hasLibrary = ready !== null && ready.roots.length > 0;
  /** 已指定目录但索引还是空的：正在建库，不是图库空 */
  const indexing = ready !== null && ready.scanning && ready.assetCount === 0;

  // 检查器只在浏览视图里有意义，别的视图不占这一列
  const showInspector = inspectorOpen && (view === "library" || view === "timeline");
  const inspected = useInspectedAsset(view === "library" || view === "timeline");

  // 工具栏只服务浏览视图。人物与设置是整页视图，排序与密度在那里没有意义。
  const showToolbar = view === "library" || view === "timeline";

  // 已选栏优先于检查器占位。两者都是「这次操作的上下文」，而标记模式下
  // 用户正在跟着一个批次走，这时候先看自己刚勾了哪些，不是先去查一张的参数。
  const showSelectionPanel = markMode && selectionOrder.length > 0;

  const enhanceNotice = useEnhancementNotifications();

  useGlobalShortcuts();
  useScanPolling(ready !== null && ready.scanning, refreshStats);
  useResetSelectionOnLibraryChange(libraryLabel(ready?.roots ?? []) ?? null);

  return (
    <div className="flex h-full flex-col overflow-hidden bg-bg">
      <TitleBar scanning={ready?.scanning ?? false} />

      {/* 检查器在窄窗下脱离布局列、浮在这一层之上，所以这一层是它的定位上下文 */}
      <div className="relative flex min-h-0 flex-1">
        <Sidebar stats={ready} />

        <main className="flex min-w-0 flex-1 flex-col">
          {hasLibrary ? (
            showToolbar ? (
              <Toolbar onLibraryChanged={reloadStats} />
            ) : null
          ) : null}
          <div className="min-h-0 flex-1 overflow-hidden">
            {stats.status === "loading" ? (
              <BusyState label="正在打开图库" detail="读取宿主状态与索引计数。" />
            ) : stats.status === "error" ? (
              // 读不到状态和没有库是两回事。这里如实报故障，
              // 之前它落进 NoLibraryState，用户看到的是「还没有打开图库」和一个能点的按钮。
              <ErrorState
                title="读不到图库状态"
                description="没能从宿主拿到图库计数。库可能没打开，也可能是宿主连接出了问题，下面是它返回的原始信息。"
                detail={stats.message}
                onRetry={reloadStats}
              />
            ) : !hasLibrary ? (
              <NoLibraryState onChanged={reloadStats} />
            ) : indexing && showToolbar ? (
              // 扫描刚开始时图库里确实还读不出照片，但原因不是「这个图库里还没有照片」。
              // 只挡浏览视图：设置与人物在扫描期间照样能用。
              <EmptyState
                icon={<ArrowsClockwise size={22} />}
                title="正在建立索引"
                description={`正在读取 ${libraryLabel(ready.roots)} 下的照片。扫描在后台进行，照片数量会在状态栏实时更新；不想等了可以在右下角取消。`}
              />
            ) : (
              <>
                {view === "search" ? <SearchView /> : null}
                {view === "library" ? <LibraryView /> : null}
                {view === "timeline" ? <TimelineView /> : null}
                {view === "people" ? <PeopleView /> : null}
                {view === "settings" ? <SettingsView /> : null}
                 {view === "enhancement-tasks" ? <EnhancementTasksView /> : null}
              </>
            )}
          </div>
        </main>

        {showSelectionPanel && hasLibrary ? (
          <SelectionPanel order={selectionOrder} />
        ) : showInspector && hasLibrary ? (
          <Inspector asset={inspected} selectedCount={selectedCount} />
        ) : null}
      </div>

      {enhanceNotice ? <div role="status" aria-live="polite" className="fixed right-4 bottom-12 z-[var(--lf-z-toast)] rounded-md border border-line bg-popover px-3 py-2 text-sm text-ink shadow-pop">{enhanceNotice}</div> : null}
      <StatusBar
        stats={ready}
        demoMode={dataSource.kind === "demo"}
        onCancelScan={hasLibrary ? () => void cancelScan() : undefined}
        onRescan={hasLibrary ? () => void rescan() : undefined}
      />
    </div>
  );
}

function useEnhancementNotifications(): string | null {
  const [notice, setNotice] = useState<string | null>(null);
  const seen = useRef<Set<string> | null>(null);
  useEffect(() => {
    if (!hostAvailable) return;
    const tick = () => void bridge.enhancementTasks().then((rows) => {
      const terminal = rows.filter((row) => row.status === "succeeded" || row.status === "failed");
      if (seen.current === null) { seen.current = new Set(terminal.map((row) => row.id)); return; }
      const fresh = terminal.find((row) => !seen.current?.has(row.id));
      terminal.forEach((row) => seen.current?.add(row.id));
      if (fresh) {
        setNotice(fresh.status === "succeeded" ? `${fresh.fileName} 的照片增强已完成` : `${fresh.fileName} 的照片增强失败`);
        window.setTimeout(() => setNotice(null), 5000);
      }
    }).catch(() => undefined);
    tick();
    const timer = window.setInterval(tick, 1000);
    return () => window.clearInterval(timer);
  }, []);
  return notice;
}


function useInspectedAsset(active: boolean): DemoAsset | null {
  const selected = useUi((s) => s.selected);
  const cursorId = useUi((s) => s.cursorId);

  if (!active || selected.size > 1) return null;
  const id = selected.size === 1 ? [...selected][0] : cursorId;
  return id === undefined || id === null ? null : assetById(id);
}

/**
 * 扫描期间轮询状态。
 *
 * 用 `refresh` 而不是 `reload`：reload 会把状态打回 loading，
 * 主区域于是每 400ms 被换成「正在打开图库」再换回来——整屏闪烁跳动就是这么来的。
 * refresh 保留上一份计数，屏幕上只有数字在变。
 */
function useScanPolling(scanning: boolean, refresh: () => void) {
  useEffect(() => {
    if (!scanning) return;
    const timer = window.setInterval(refresh, SCAN_POLL_MS);
    return () => window.clearInterval(timer);
  }, [scanning, refresh]);
}

/**
 * 换图库后清掉旧选区。
 * 选区存的是资产 id，指向的是上一个目录里的照片；
 * 留着会让检查器显示不相干的元数据，批量计数也是错的。
 */
function useResetSelectionOnLibraryChange(root: string | null) {
  const previous = useRef<string | null>(root);

  useEffect(() => {
    if (previous.current === root) return;
    previous.current = root;
    ui.clearSelection();
  }, [root]);
}

/**
 * 全局快捷键只保留斜杠聚焦搜索。
 * 单字母快捷键在照片工具里太容易误触，除非和菜单项一一对应，否则不做。
 */
function useGlobalShortcuts() {
  useEffect(() => {
    const onKeyDown = (event: KeyboardEvent) => {
      const target = event.target;
      if (target instanceof HTMLElement) {
        const tag = target.tagName;
        if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT" || target.isContentEditable) {
          return;
        }
      }

      if (event.key === "/") {
        event.preventDefault();
        ui.setView("search");
      }
    };

    window.addEventListener("keydown", onKeyDown);
    return () => window.removeEventListener("keydown", onKeyDown);
  }, []);
}
