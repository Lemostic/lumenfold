/**
 * @lumenfold/tokens
 *
 * 供插件读取的令牌类型定义。CSS 变量是运行时真值,这里只描述形状。
 * 插件读取令牌的正确方式是读 CSS 变量(getComputedStyle),而不是硬编码色值,
 * 这样用户在宿主里切主题时插件面板会跟着变。
 */

export type ThemeName = "dark" | "light";

/** 语义令牌名,与 index.css 中的 --lf-* 一一对应(去掉 --lf- 前缀)。 */
export type TokenName =
  | "bg"
  | "bg-sunken"
  | "surface"
  | "surface-2"
  | "surface-3"
  | "popover"
  | "overlay"
  | "border"
  | "border-strong"
  | "border-subtle"
  | "text"
  | "text-2"
  | "text-3"
  | "text-disabled"
  | "accent"
  | "accent-hover"
  | "accent-active"
  | "accent-ink"
  | "accent-soft"
  | "accent-line"
  | "accent-glow"
  | "link"
  | "link-soft"
  | "danger"
  | "danger-soft"
  | "danger-line"
  | "warn"
  | "warn-soft"
  | "warn-line"
  | "success"
  | "success-soft"
  | "success-line";

/** 圆角档位。2px 缩略图 / 6px 控件与面板 / 10px 卡片 / 999px 芯片与头像。 */
export const RADIUS_SCALE = {
  thumbnail: 2,
  control: 6,
  card: 10,
  chip: 999,
} as const;

export const MOTION = {
  /** 110ms:悬停、按下、焦点环等即时反馈 */
  fast: 110,
  /** 160ms:面板展开、对话框、选择态变化 */
  base: 160,
  /** 240ms:模态进入、抽屉滑入 */
  slow: 240,
  ease: "cubic-bezier(0.2, 0.6, 0.2, 1)",
  easeOut: "cubic-bezier(0.16, 1, 0.3, 1)",
} as const;

/** 正文基准字号(px)。工具界面密度由它决定。 */
export const FONT_SIZE_BASE = 13;

/** 字号阶梯。组件不允许自行发明字号,只能从这里挑。 */
export const FONT_SIZE = {
  "2xs": 11,
  xs: 12,
  sm: 13,
  md: 14,
  lg: 16,
  xl: 19,
  "2xl": 24,
} as const;

/** 字重阶梯。三档,不允许组件里出现 700/800。 */
export const FONT_WEIGHT = {
  regular: 400,
  medium: 500,
  semibold: 600,
} as const;

/** chrome 尺寸。插件面板必须尊重这些尺寸,否则会盖住宿主控件。 */
export const CHROME = {
  titlebar: 36,
  toolbar: 44,
  statusbar: 28,
  searchbar: 56,
  sidebar: 220,
  sidebarCollapsed: 56,
  inspector: 288,
  controlSm: 28,
  controlMd: 32,
  controlLg: 40,
  iconSm: 24,
  iconMd: 28,
  iconLg: 32,
} as const;

/** 间距栅格(4px 基线)。组件里不直接写 px 值,统一从这里走。 */
export const SPACE = {
  0: 0,
  1: 4,
  2: 8,
  3: 12,
  4: 16,
  5: 20,
  6: 24,
  8: 32,
  10: 40,
  12: 48,
} as const;

/** 层级表。组件不允许自行发明 z-index。 */
export const Z = {
  base: 0,
  sticky: 10,
  overlay: 20,
  popover: 30,
  dialog: 40,
  toast: 50,
} as const;

/** 读取当前主题下某个语义令牌的实际颜色值。 */
export function readToken(name: TokenName, root: Element = document.documentElement): string {
  return getComputedStyle(root).getPropertyValue(`--lf-${name}`).trim();
}

/**
 * 读取 chrome 尺寸(返回 px 数字)。插件面板在计算可用区域时,
 * 必须从宿主拿 chrome 高度再决定自己的最大高度。
 */
export function readChrome(
  key: keyof typeof CHROME,
  root: Element = document.documentElement,
): number {
  const name: Record<keyof typeof CHROME, string> = {
    titlebar: "--lf-chrome-titlebar",
    toolbar: "--lf-chrome-toolbar",
    statusbar: "--lf-chrome-statusbar",
    searchbar: "--lf-chrome-searchbar",
    sidebar: "--lf-chrome-sidebar",
    sidebarCollapsed: "--lf-chrome-sidebar-collapsed",
    inspector: "--lf-chrome-inspector",
    controlSm: "--lf-control-sm",
    controlMd: "--lf-control-md",
    controlLg: "--lf-control-lg",
    iconSm: "--lf-control-icon-sm",
    iconMd: "--lf-control-icon-md",
    iconLg: "--lf-control-icon-lg",
  };
  const raw = getComputedStyle(root).getPropertyValue(name[key]).trim();
  return Number.parseInt(raw, 10) || CHROME[key];
}

export function currentTheme(root: Element = document.documentElement): ThemeName {
  return root.getAttribute("data-theme") === "light" ? "light" : "dark";
}
