import { useSyncExternalStore } from "react";
import type { SearchMode, SortKey } from "../lib/types";

export type ViewId = "library" | "timeline" | "people" | "search" | "settings" | "enhancement-tasks";

/** 图库里的两种浏览方式。 */
export type LayoutId = "grid" | "wall";

export interface UiState {
  view: ViewId;
  /** 网格的缩略图行高目标（px）。行高统一是虚拟滚动不掉帧的前提。 */
  rowHeight: number;
  /** 照片墙的格子边长（px）。与 rowHeight 互不影响，各自记忆。 */
  wallCell: number;
  layout: LayoutId;
  /** 照片墙是否按拍摄年月分组。关掉就是纯粹的一格格。 */
  wallGroups: boolean;
  /**
   * 标记模式：单击是「打标记 / 取消标记」，而不是「只选这一张」。
   *
   * 用户可以在标记模式下挨个点下去攒一批再统一处理；默认关闭标记模式。
   * 关闭标记模式时，裸单击只选中当前照片，键盘操作两者一致。
   *
   * 两种模式共用一个 `selected`，但含义不同：
   * 浏览模式下它是「正在看这一张」，最多只有一张，
   * 所以浏览时不该出现「已标记 N 项 / 取消标记 / 复制剪切」这类批量控件；
   * 标记模式下它是「攒下来待处理的一批」，才会带出批量条。
   * 进出标记模式会清掉选区：留着上一种模式的选区会让工具栏说出一句假话。
   */
  markMode: boolean;
  sort: SortKey;
  selected: ReadonlySet<string>;
  /**
   * 选中项的**选取顺序**，最新选的在最前。
   *
   * Set 只能回答「在不在选中」，回答不了「刚选的是哪张」。
   * 右侧的已选栏要展示最近一次选中的预览，也要按选取顺序排列并且能在列表里取消，
   * 所以这里保一份有序副本。它和 selected 始终同步，不允许只改其中一个。
   */
  selectionOrder: readonly string[];
  /**
   * 当前浏览视图里已经加载出来的资产 id，按显示顺序。
   *
   * 批量条上的「全选」「反选」需要一个顺序明确的全集，而宿主没有「给我前 N 万条」
   * 这种命令：已加载的分页就是全集。浏览视图负责在翻页/排序后更新它。
   */
  visibleIds: readonly string[];
  /** 键盘光标所在条目，与选区分离：方向键移动光标，空格才改变选区 */
  cursorId: string | null;
  inspectorOpen: boolean;
  sidebarCollapsed: boolean;
  theme: "dark" | "light";
  searchQuery: string;
  searchMode: SearchMode;
  personId: string | null;
  /**
   * 图库的类别筛选，取 `TAG_META` 里的 id；`null` 表示不筛。
   *
   * 放在共享状态里而不是 LibraryView 的局部 state：网格和照片墙是同一个图库的
   * 两种布局，用户在一种布局里选好的类别，切到另一种不该丢掉。
   */
  tagFilter: string | null;
}

const MIN_ROW_HEIGHT = 120;
const MAX_ROW_HEIGHT = 360;
const MIN_WALL_CELL = 96;
const MAX_WALL_CELL = 320;

let state: UiState = {
  view: "library",
  rowHeight: 200,
  wallCell: 168,
  layout: "grid",
  wallGroups: true,
  markMode: false,
  sort: "taken-desc",
  selected: new Set<string>(),
  selectionOrder: [],
  visibleIds: [],
  cursorId: null,
  inspectorOpen: true,
  sidebarCollapsed: false,
  theme:
    document.documentElement.getAttribute("data-theme") === "light" ? "light" : "dark",
  searchQuery: "",
  searchMode: "text",
  personId: null,
  tagFilter: null,
};

const listeners = new Set<() => void>();

function emit() {
  listeners.forEach((listener) => listener());
}

function set(patch: Partial<UiState>) {
  state = { ...state, ...patch };
  emit();
}

/**
 * 唯一的选区写入点。Set 用于成员测试，数组用于顺序，两者不允许分开。
 *
 * 分开存的后果是很难查的错位：右侧列表看着右张预览展示的是左张，
 * 而状态栏说已选 3 项——两边都“说了紧”，去哪里找都找不到。
 */
function setSelection(next: Set<string>, order: readonly string[]) {
  state = { ...state, selected: next, selectionOrder: order };
  emit();
}

/** 把一个 id 推到最前面；已在列表里就原地返回原数组，避免无赁的重排。 */
function toFront(order: readonly string[], id: string): string[] {
  const index = order.indexOf(id);
  if (index <= 0) return order as string[];
  return [id, ...order.slice(0, index), ...order.slice(index + 1)];
}

export const ui = {
  get: () => state,
  subscribe(listener: () => void) {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },

  setView(view: ViewId) {
    set({ view, personId: view === "people" ? state.personId : null });
  },

  setRowHeight(rowHeight: number) {
    set({ rowHeight: Math.min(MAX_ROW_HEIGHT, Math.max(MIN_ROW_HEIGHT, rowHeight)) });
  },

  setWallCell(wallCell: number) {
    set({ wallCell: Math.min(MAX_WALL_CELL, Math.max(MIN_WALL_CELL, wallCell)) });
  },

  setLayout(layout: LayoutId) {
    set({ layout });
  },

  setWallGroups(wallGroups: boolean) {
    set({ wallGroups });
  },

  setMarkMode(markMode: boolean) {
    if (markMode === state.markMode) return;
    /**
     * 进出标记模式都清掉选区。
     *
     * 关掉标记模式时不清，选区里剩下的那几张在浏览模式下就没有任何控件可以处理——
     * 批量条只属于标记模式，于是它们既看不见也动不了，成了一个静默的死角。
     * 打开标记模式时同样清掉：刚点过的那一张在浏览模式下只是「正在看」，
     * 一进标记模式它就成了「已标记」，那不是用户的意思。
     */
    set({ markMode });
    setSelection(new Set<string>(), []);
  },

  setVisibleIds(visibleIds: readonly string[]) {
    /**
     * 按内容比较，而不是按引用。
     *
     * 浏览视图每次推进分页都会产出一个内容相同、引用不同的数组。
     * 按引用比较意味着每推一页，整个 store 的所有订阅者（工具栏、状态栏、网格、检查器）
     * 都要重新渲染一遍，而它们并不关心 visibleIds 变了。
     */
    const current = state.visibleIds;
    if (current.length === visibleIds.length && current.every((id, i) => id === visibleIds[i])) return;
    set({ visibleIds });
  },

  /** 切出一个与选区相反的集合：全选之后想临时去掉几张时用得上。 */
  invertSelection(ids: readonly string[]) {
    const added: string[] = [];
    const kept: string[] = [];
    for (const id of ids) {
      if (state.selected.has(id)) kept.push(id);
      else added.push(id);
    }
    // 新加的那批才是刚发生的动作，放在最前面：用户刚反选过，
    // 右侧预览就该是他刚勾的那张。
    setSelection(new Set([...added, ...kept]), [...added, ...kept]);
  },

  setSort(sort: SortKey) {
    set({ sort });
  },

  /**
   * 切换类别筛选。
   *
   * 换类别时清空选区：上一类的选中项在新类别的结果集里可能根本不存在，
   * 留着它们会让批量条指向一批看不见的照片，而右侧已选栏还在认真列它们。
   * 这和进出标记模式清选区是同一条理由。
   */
  setTagFilter(tagFilter: string | null) {
    if (tagFilter === state.tagFilter) return;
    set({ tagFilter });
    setSelection(new Set<string>(), []);
  },

  setTheme(theme: UiState["theme"]) {
    document.documentElement.setAttribute("data-theme", theme);
    localStorage.setItem("lumenfold.theme", theme);
    set({ theme });
  },

  toggleSidebar() {
    set({ sidebarCollapsed: !state.sidebarCollapsed });
  },

  toggleInspector() {
    set({ inspectorOpen: !state.inspectorOpen });
  },

  setSearchQuery(searchQuery: string) {
    set({ searchQuery, view: searchQuery.length > 0 ? "search" : state.view });
  },

  setSearchMode(searchMode: SearchMode) {
    set({ searchMode });
  },

  openPerson(personId: string | null) {
    set({ personId, view: personId ? "library" : "people" });
  },

  moveCursor(cursorId: string | null) {
    set({ cursorId });
  },

  /**
   * 加入或移出标记，**只动这一张**。
   *
   * 这个函数以前叫 `toggleSelection(id, anchorId, orderedIds)`，收到锚点后会顺手
   * 把锚点到目标之间的所有照片也加进选区。结果是「点第 5 张 → 前 5 张全被选中」：
   * 用户以为自己在挨个打标记，实际拿到的是一段区间，而区间操作本来是
   * shift+单击的语义。挨个标记和连选是两件事，现在分开放在两个函数里。
   */
  toggleMark(id: string) {
    const next = new Set(state.selected);
    if (next.has(id)) {
      next.delete(id);
      setSelection(next, state.selectionOrder.filter((each) => each !== id));
    } else {
      next.add(id);
      // 刚勾的那张到最前面，右侧预览跟着变。
      setSelection(next, toFront(state.selectionOrder, id));
    }
    set({ cursorId: id });
  },

  /** 取消其中一项，不改光标。供右侧已选列表里的「取消」按钮直接调用。 */
  deselect(id: string) {
    if (!state.selected.has(id)) return;
    const next = new Set(state.selected);
    next.delete(id);
    setSelection(next, state.selectionOrder.filter((each) => each !== id));
  },

  selectOnly(id: string) {
    setSelection(new Set([id]), [id]);
    set({ cursorId: id });
  },

  /** shift+单击 / shift+方向键：把选区换成锚点到目标之间的整段。 */
  selectRange(anchorId: string, targetId: string, orderedIds: readonly string[]) {
    const from = orderedIds.indexOf(anchorId);
    const to = orderedIds.indexOf(targetId);
    if (from < 0 || to < 0) {
      setSelection(new Set([targetId]), [targetId]);
      set({ cursorId: targetId });
      return;
    }
    const [start, end] = from <= to ? [from, to] : [to, from];
    const order: string[] = [];
    for (let i = start; i <= end; i += 1) {
      const candidate = orderedIds[i];
      if (candidate !== undefined) order.push(candidate);
    }
    // 连选的末端是刚点的那张，它回到列表首位。
    setSelection(new Set(order), toFront(order, targetId));
    set({ cursorId: targetId });
  },

  selectAll(ids: readonly string[]) {
    setSelection(new Set(ids), [...ids]);
  },

  clearSelection() {
    setSelection(new Set<string>(), []);
    set({ cursorId: null });
  },
};

export function useUi<T>(select: (s: UiState) => T): T {
  return useSyncExternalStore(
    ui.subscribe,
    () => select(state),
    () => select(state),
  );
}

export const ROW_HEIGHT_RANGE = { min: MIN_ROW_HEIGHT, max: MAX_ROW_HEIGHT } as const;
export const WALL_CELL_RANGE = { min: MIN_WALL_CELL, max: MAX_WALL_CELL } as const;