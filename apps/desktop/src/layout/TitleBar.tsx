import { getCurrentWindow } from "@tauri-apps/api/window";
import {
  MagnifyingGlass,
  Minus,
  Moon,
  Square,
  Sun,
  Wrench,
  X,
} from "@phosphor-icons/react";
import { useEffect, useState, type ReactNode } from "react";
import { IconButton } from "../components/Button";
import { hostAvailable } from "../lib/ipc";
import { ui, useUi } from "../state/ui";

/**
 * 顶栏：产品标记 + 居中搜索 + 右侧动作与窗口控件。
 *
 * # 为什么搜索搬到顶栏
 *
 * 顶栏搜索胶囊是入口而不是第二个输入框：点击后在搜索页的唯一输入框输入。
 * 如果在这里边输入边切页，首字母会导致原输入框卸载并打断中文输入法。
 *
 * # 窗口是自己的
 *
 * `tauri.conf.json` 里 `decorations: false`，所以最小化/最大化/关闭必须由这里画。
 * 参考的那版界面是网页、没有这三个按钮，但我们不能省——省了窗口就关不掉了。
 */
const IS_MAC = typeof navigator !== "undefined" && /Mac/i.test(navigator.platform);

interface TitleBarProps {
  /** 扫描中时顶栏下沿亮一条呼吸光 */
  scanning: boolean;
}

export function TitleBar({ scanning }: TitleBarProps) {
  const view = useUi((s) => s.view);
  const query = useUi((s) => s.searchQuery);
  const theme = useUi((s) => s.theme);
  const [maximized, setMaximized] = useState(false);

  useEffect(() => {
    if (!hostAvailable) return;
    let alive = true;
    void getCurrentWindow()
      .isMaximized()
      .then((value) => {
        if (alive) setMaximized(value);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);

  const appWindow = hostAvailable ? getCurrentWindow() : null;

  return (
    <header
      data-tauri-drag-region
      className={[
        "drag-region relative flex h-12 shrink-0 items-center gap-3 border-b border-line bg-surface",
        IS_MAC ? "pl-[72px]" : "pl-3",
      ].join(" ")}
    >
      <div className="flex shrink-0 items-center gap-2">
        <BrandMark />
      </div>

      {/*
        搜索视图自己有更醒目的一整条搜索框，这里再放一个就会出现两个同步的输入框，
        用户得先想「该点哪个」。所以只在别的视图里显示顶栏这一个。
      */}
      {view === "search" ? (
        <div className="flex-1" />
      ) : (
        <div className="no-drag mx-auto w-full max-w-[520px] px-2">
          <div className="relative">
            <span className="pointer-events-none absolute inset-y-0 left-3 flex items-center text-ink-3">
              <MagnifyingGlass size={15} />
            </span>
            <button type="button" onClick={() => ui.setView("search")} aria-label="打开照片搜索"
              className="flex h-9 w-full items-center rounded-lg border border-line bg-surface-2 pr-3 pl-9 text-left text-sm text-ink-3 transition-colors hover:bg-surface-3 focus-visible:border-line-strong">
              <span className="truncate">{query || "输入一段描述搜索照片：大合照"}</span>
            </button>
          </div>
        </div>
      )}

      <div className="no-drag ml-auto flex shrink-0 items-center gap-1">
        <IconButton
          label={theme === "dark" ? "切换到亮色主题" : "切换到暗色主题"}
          onClick={() => ui.setTheme(theme === "dark" ? "light" : "dark")}
        >
          {theme === "dark" ? <Sun size={16} /> : <Moon size={16} />}
        </IconButton>
        <IconButton
          label="设置"
          active={view === "settings"}
          onClick={() => ui.setView("settings")}
        >
          <Wrench size={16} />
        </IconButton>

        {appWindow && !IS_MAC ? (
          <div className="ml-1 flex items-center">
            <WindowControl label="最小化" onClick={() => void appWindow.minimize()}>
              <Minus size={14} weight="bold" />
            </WindowControl>
            <WindowControl
              label={maximized ? "还原" : "最大化"}
              onClick={() => {
                void appWindow.toggleMaximize();
                void appWindow.isMaximized().then(setMaximized);
              }}
            >
              <Square size={11} weight="bold" />
            </WindowControl>
            <WindowControl label="关闭" danger onClick={() => void appWindow.close()}>
              <X size={14} weight="bold" />
            </WindowControl>
          </div>
        ) : null}
      </div>

      {/*
        扫描时的琥珀呼吸光。1px 渐变条，只在扫描态亮起。
        用 lf-edge-glow 的 CSS 类而不是内联 style，保证 token 体系里可追溯。
      */}
      <div
        aria-hidden="true"
        className="lf-edge-glow pointer-events-none absolute inset-x-0 bottom-0"
        data-active={scanning ? "true" : "false"}
      />
    </header>
  );
}

function BrandMark() {
  return (
    <span className="flex items-center gap-2 pr-1">
      {/* 产品标记：一个被折起的光栅角，不是装饰性插画 */}
      <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" className="text-accent">
        <path d="M1 13.5 13.5 1" stroke="currentColor" strokeWidth="2" strokeLinecap="round" />
        <path
          d="M8.5 13.5 13.5 8.5"
          stroke="currentColor"
          strokeWidth="2"
          strokeLinecap="round"
          opacity="0.55"
        />
      </svg>
      <span className="text-sm font-medium tracking-tight text-ink">Lumenfold</span>
    </span>
  );
}

function WindowControl({
  label,
  danger = false,
  onClick,
  children,
}: {
  label: string;
  danger?: boolean;
  onClick(): void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className={[
        "flex h-8 w-10 items-center justify-center rounded-md text-ink-2",
        "transition-colors duration-(--lf-duration-fast) hover:text-ink",
        danger ? "hover:bg-danger hover:text-accent-ink" : "hover:bg-surface-2",
      ].join(" ")}
    >
      {children}
    </button>
  );
}
