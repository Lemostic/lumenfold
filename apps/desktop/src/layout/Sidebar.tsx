import {
  ClockCounterClockwise,
  Images,
  FolderOpen,
  GearSix,
  Sparkle,
  SidebarSimple,
  SquaresFour,
  UsersThree,
} from "@phosphor-icons/react";
import { ui, useUi, type ViewId } from "../state/ui";
import { useMediaQuery } from "../lib/useMediaQuery";
import { useTagCounts } from "../state/library";
import { TAG_META, libraryLabel, type LibraryStats } from "../lib/types";

interface NavItem {
  id: ViewId;
  label: string;
  icon: typeof Images;
}

/**
 * 有真实视图支撑的导航项。
 *
 * 只放有真实视图或可操作设置支撑的入口：图库、时间线、人物、图库目录与分类。
 * 参考界面中的地图、相册、共享、回收站等尚无数据模型，不能画空入口。
 */
const BROWSE: NavItem[] = [
  { id: "library", label: "图库", icon: Images },
  { id: "timeline", label: "时间线", icon: ClockCounterClockwise },
];

const ORGANISE: NavItem[] = [{ id: "people", label: "人物", icon: UsersThree }];

interface SidebarProps {
  /**
   * 图库概况由 AppShell 传下来。
   * 侧边栏自己再取一次 stats，就会在首屏多打一遍 library_stats：
   * 同一份数据两个请求，代价落在冷启动预算上。
   * null 表示宿主状态还在读。
   */
  stats: LibraryStats | null;
}

export function Sidebar({ stats }: SidebarProps) {
  const userCollapsed = useUi((s) => s.sidebarCollapsed);
  const view = useUi((s) => s.view);
  // 窄窗自动收成图标栏：内容区优先，导航保住图标与可访问名称。
  // 钩子必须无条件调用，所以先把结果取出来再合并。
  const narrow = useMediaQuery("(max-width: 1023px)");
  const collapsed = userCollapsed || narrow;

  return (
    <nav
      aria-label="主导航"
      data-collapsed={collapsed ? "true" : "false"}
      className={[
        "flex shrink-0 flex-col overflow-hidden border-r border-line bg-surface",
        "transition-[width] duration-(--lf-duration) ease-(--lf-ease)",
        collapsed ? "w-14" : "w-56",
      ].join(" ")}
    >
      <div className="flex min-h-0 flex-1 flex-col gap-1 overflow-y-auto p-2 scroll-area">
        <NavGroup items={BROWSE} collapsed={collapsed} active={view} />
        <button type="button" onClick={() => ui.setView("settings")} aria-label="图库目录设置" title="图库目录设置" className="flex h-8 w-full items-center gap-2.5 rounded-md px-2 text-left text-ink-2 transition-colors hover:bg-surface-2 hover:text-ink">
          <FolderOpen size={17} className="shrink-0" />
          {!collapsed ? <span className="truncate text-sm">图库目录</span> : null}
        </button>
        <Divider />
        <NavGroup items={ORGANISE} collapsed={collapsed} active={view} />
        <Divider />
        <CategoryGroup collapsed={collapsed} />
        <Divider />
                 <button type="button" onClick={() => ui.setView("enhancement-tasks")} aria-current={view === "enhancement-tasks" ? "page" : undefined} aria-label="照片增强任务" title="照片增强任务" className={`flex h-8 w-full items-center gap-2.5 rounded-md px-2 text-left transition-colors ${view === "enhancement-tasks" ? "bg-surface-3 font-medium text-ink" : "text-ink-2 hover:bg-surface-2 hover:text-ink"}`}>
           <Sparkle size={17} className="shrink-0" />
           {!collapsed ? <span className="truncate text-sm">照片增强任务</span> : null}
         </button>
         <button type="button" onClick={() => ui.setView("settings")} aria-current={view === "settings" ? "page" : undefined} aria-label="设置" title="设置" className={`flex h-8 w-full items-center gap-2.5 rounded-md px-2 text-left transition-colors ${view === "settings" ? "bg-surface-3 font-medium text-ink" : "text-ink-2 hover:bg-surface-2 hover:text-ink"}`}>
          <GearSix size={17} className="shrink-0" />
          {!collapsed ? <span className="truncate text-sm">设置</span> : null}
        </button>
      </div>

      {!collapsed ? <LibrarySummary stats={stats} /> : null}

      <div className="shrink-0 border-t border-line p-1.5">
        <button
          type="button"
          onClick={() => ui.toggleSidebar()}
          aria-label={collapsed ? "展开侧边栏" : "折叠侧边栏"}
          title={collapsed ? "展开侧边栏" : "折叠侧边栏"}
          className={[
            "flex h-8 w-full items-center gap-2.5 rounded-md px-2",
            "transition-colors duration-(--lf-duration-fast) ease-(--lf-ease)",
            "text-ink-3 hover:bg-surface-2 hover:text-ink",
          ].join(" ")}
        >
          <SidebarSimple size={17} weight="regular" className="shrink-0" />
          {!collapsed ? <span className="text-sm">收起</span> : null}
        </button>
      </div>
    </nav>
  );
}

function Divider() {
  return <div aria-hidden="true" className="my-1 h-px bg-line" />;
}

function NavGroup({
  items,
  collapsed,
  active,
}: {
  items: NavItem[];
  collapsed: boolean;
  active: ViewId;
}) {
  return (
    <ul className="flex flex-col gap-0.5">
      {items.map((item) => {
        const Icon = item.icon;
        const isActive = item.id === active;
        return (
          <li key={item.id}>
            <button
              type="button"
              onClick={() => ui.setView(item.id)}
              aria-current={isActive ? "page" : undefined}
              aria-label={item.label}
              title={item.label}
              className={[
                "flex h-8 w-full items-center gap-2.5 rounded-md px-2 text-left",
                "transition-colors duration-(--lf-duration-fast) ease-(--lf-ease)",
                isActive
                  ? "bg-surface-3 font-medium text-ink"
                  : "text-ink-2 hover:bg-surface-2 hover:text-ink",
              ].join(" ")}
            >
              <Icon size={17} weight={isActive ? "fill" : "regular"} className="shrink-0" />
              {!collapsed ? <span className="truncate text-sm">{item.label}</span> : null}
            </button>
          </li>
        );
      })}
    </ul>
  );
}

/**
 * 分类。
 *
 * 之前它是图库视图顶上的一条横栏。搬进侧栏是因为它和「图库 / 时间线」是同一类
 * 东西——都是「看哪一部分照片」——而横栏占的那一行高度，在照片为主的界面里
 * 是最贵的一行。
 *
 * 计数直接来自宿主。加载中显示「—」而不是 0：0 是一个事实，「还没问到」不是。
 */
function CategoryGroup({ collapsed }: { collapsed: boolean }) {
  const active = useUi((s) => s.tagFilter);
  const counts = useTagCounts();

  const rows = counts.status === "ready" ? counts.value : null;
  const total = rows?.[0]?.total ?? null;
  const classified = rows?.[0]?.classified ?? null;
  /**
   * 分类是扫描时逐条补上的：老索引里的记录没有 CLASSIFIED 位，要等增量扫描
   * 把容器头重读一遍。不把这个差说出来，用户会以为自己图库里真的没有动图。
   */
  const pending =
    total !== null && classified !== null && classified < total ? total - classified : 0;

  const countFor = (id: string): number | null => {
    if (rows === null) return null;
    return rows.find((row) => row.id === id)?.count ?? 0;
  };

  const pick = (tag: string | null) => {
    ui.setTagFilter(tag);
    ui.setView("library");
  };

  if (collapsed) {
    return (
      <ul className="flex flex-col gap-0.5">
        <li>
          <button type="button" onClick={() => pick(null)} aria-label="全部照片" title="全部照片" aria-pressed={active === null}
            className="flex h-8 w-full items-center justify-center rounded-md text-ink-2 hover:bg-surface-2 hover:text-ink">
            <SquaresFour size={16} />
          </button>
        </li>
        {TAG_META.map((entry) => (
          <li key={entry.id}>
            <button type="button" onClick={() => pick(entry.id)} aria-label={entry.label} title={entry.label} aria-pressed={active === entry.id}
              className={`flex h-8 w-full items-center justify-center rounded-md text-xs ${active === entry.id ? "bg-accent-soft text-accent" : "text-ink-2 hover:bg-surface-2 hover:text-ink"}`}>
              {entry.label.slice(0, 1)}
            </button>
          </li>
        ))}
      </ul>
    );
  }

  return (
    <div>
      <p className="px-2 pb-1 text-2xs font-medium tracking-wide text-ink-3 uppercase">分类</p>
      <ul className="flex flex-col gap-0.5">
        <CategoryRow
          label="全部"
          count={total}
          active={active === null}
          onClick={() => pick(null)}
        />
        {TAG_META.map((entry) => (
          <CategoryRow
            key={entry.id}
            label={entry.label}
            hint={entry.hint}
            count={countFor(entry.id)}
            active={active === entry.id}
            // 再点一次当前分类就是取消筛选，省掉一次「先点全部」的往返。
            onClick={() => pick(active === entry.id ? null : entry.id)}
          />
        ))}
      </ul>

      {pending > 0 ? (
        <p className="tabular mt-1 px-2 text-2xs leading-relaxed text-ink-3">
          还有 {pending.toLocaleString("zh-CN")} 张没分过类
        </p>
      ) : null}
    </div>
  );
}

function CategoryRow({
  label,
  hint,
  count,
  active,
  onClick,
}: {
  label: string;
  hint?: string;
  /** `null` 表示「还没问到」，和 0 不是一回事 */
  count: number | null;
  active: boolean;
  onClick(): void;
}) {
  return (
    <li>
      <button
        type="button"
        onClick={onClick}
        aria-pressed={active}
        // 原生 title 而不是 Tooltip 组件：这段说明是补充性的，
        // 而侧栏里每一项都套一层气泡容器会让这一列多出十几个包层。
        title={hint}
        className={[
          "flex h-7 w-full items-center gap-2 rounded-md px-2 text-left",
          "transition-colors duration-(--lf-duration-fast) ease-(--lf-ease)",
          active ? "bg-accent-soft text-accent" : "text-ink-2 hover:bg-surface-2 hover:text-ink",
        ].join(" ")}
      >
        <span className="min-w-0 flex-1 truncate text-sm">{label}</span>
        <span className="tabular shrink-0 text-2xs text-ink-3">
          {count === null ? "—" : count.toLocaleString("zh-CN")}
        </span>
      </button>
    </li>
  );
}

/**
 * 图库概况。只显示宿主真正报告的量：目录与资产数。
 * 三种状态分开说：还没读到、读到了且没目录、读到了有目录。
 * 读库途中不能先写「还没有选择目录」——那是在替宿主下结论。
 */
function LibrarySummary({ stats }: { stats: LibraryStats | null }) {
  const label = stats === null ? null : libraryLabel(stats.roots);

  return (
    <div className="mx-2 mb-2 rounded-md border border-line bg-sunken p-2.5">
      <div className="flex items-center gap-1.5 text-2xs font-medium tracking-wide text-ink-3 uppercase">
        <SquaresFour size={12} />
        本地图库
      </div>
      {stats === null ? (
        <p className="mt-1.5 text-xs leading-relaxed text-ink-3">正在读取图库状态</p>
      ) : label !== null ? (
        <ul className="mt-1.5 flex flex-col gap-1">
          {/* 多个目录时只显示「第一个 等 N 个目录」；完整列表在设置页里。 */}
          <li className="truncate text-xs text-ink-2" title={stats.roots.join("\n")}>
            {label}
          </li>
          <Row label="资产" value={stats.assetCount.toLocaleString("zh-CN")} />
          <Row label="索引" value={indexState(stats)} />
        </ul>
      ) : (
        <p className="mt-1.5 text-xs leading-relaxed text-ink-3">还没有选择目录，索引为空</p>
      )}
    </div>
  );
}

/**
 * 索引这一行该说什么。
 *
 * 之前这里写的是「读取方式：只读」。它既没用也不对：只读暗示存在另一种写法，
 * 而应用里根本没有——批量操作会真的移动和删除文件，图库并不是只读的。
 * 一个没有出口的开关摆在界面上，用户只会去找那个不存在的另一半。
 *
 * 换成宿主真正报告的索引状态，三种情况都能对上「现在能不能查」这个问题：
 * 正在扫（结果还在补）、扫到一半被叫停（结果不全）、已就绪（可以随便查）。
 */
function indexState(stats: LibraryStats): string {
  if (stats.scanning) return "扫描中";
  if (stats.cancelled) return "未扫完";
  return "已就绪";
}

function Row({ label, value }: { label: string; value: string }) {
  return (
    <li className="flex items-baseline justify-between gap-2 text-xs">
      <span className="text-ink-3">{label}</span>
      <span className="tabular text-ink-2">{value}</span>
    </li>
  );
}
