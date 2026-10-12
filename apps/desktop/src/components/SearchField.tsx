import { MagnifyingGlass, X } from "@phosphor-icons/react";
import { useEffect, useRef } from "react";
import { Segmented } from "./Segmented";
import { IconButton } from "./Button";
import { ui, useUi } from "../state/ui";

interface SearchFieldProps {
  /** inline:工具栏一行的紧凑样式 / hero:居中大搜索框 */
  variant?: "inline" | "hero";
  /** 自动聚焦。默认 hero 聚焦,inline 不聚焦 */
  autoFocus?: boolean;
}

/**
 * 搜索框。斜杠聚焦是照片工具的肌肉记忆。
 * 输入即查询:索引在本机,防抖只会拖慢反馈。
 *
 * v2 提供两种形态:
 * - inline:贴在工具栏里,显示在文件名搜索的紧凑条
 * - hero: 居中,类似搜索引擎,用于 SearchView 进入态
 */
export function SearchField({ variant = "inline", autoFocus }: SearchFieldProps) {
  const searchQuery = useUi((s) => s.searchQuery);
  const searchMode = useUi((s) => s.searchMode);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    if (autoFocus || variant === "hero") {
      inputRef.current?.focus();
    }
  }, [variant, autoFocus]);

  if (variant === "hero") {
    return (
      <HeroSearch query={searchQuery} mode={searchMode} inputRef={inputRef} />
    );
  }

  return (
    <div className="flex h-11 shrink-0 items-center gap-2 border-b border-line bg-surface px-3">
      <MagnifyingGlass size={16} className="shrink-0 text-ink-3" />
      <input
        ref={inputRef}
        type="search"
        value={searchQuery}
        onChange={(event) => ui.setSearchQuery(event.currentTarget.value)}
        placeholder="搜索文件名、关键词与说明"
        aria-label="搜索照片"
        className="min-w-0 flex-1 bg-transparent text-sm text-ink placeholder:text-ink-3 focus-visible:outline-none"
      />
      <Segmented
        label="检索方式"
        value={searchMode}
        onChange={(next) => ui.setSearchMode(next)}
        options={[
          { value: "text", label: "文本" },
          { value: "semantic", label: "语义" },
        ]}
      />
      {searchQuery ? (
        <IconButton label="清除搜索" onClick={() => ui.setSearchQuery("")}>
          <X size={16} />
        </IconButton>
      ) : null}
    </div>
  );
}

/**
 * Hero 搜索框:SearchView 进入态与结果态通用。
 * 进入态:垂直水平居中,标签标题 + 大输入框 + 模式切换
 * 结果态:贴顶,紧凑条,显示当前 query 与模式
 *
 * 两种状态共享同一份 state,所以从「正在搜索」切到「搜到结果」时不会跳一下。
 */
function HeroSearch({
  query,
  mode,
  inputRef,
}: {
  query: string;
  mode: string;
  inputRef: React.RefObject<HTMLInputElement>;
}) {
  return (
    <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line bg-surface px-6">
      <MagnifyingGlass size={18} className="shrink-0 text-ink-3" />
      <input
        ref={inputRef}
        type="search"
        value={query}
        onChange={(event) => ui.setSearchQuery(event.currentTarget.value)}
        placeholder="搜索文件名、关键词与说明文字"
        aria-label="搜索照片"
        className="min-w-0 flex-1 bg-transparent text-md text-ink placeholder:text-ink-3 focus-visible:outline-none"
      />
      <Segmented
        label="检索方式"
        value={mode as "text" | "semantic"}
        onChange={(next) => ui.setSearchMode(next)}
        options={[
          { value: "text", label: "文本" },
          { value: "semantic", label: "语义" },
        ]}
      />
      {query ? (
        <IconButton label="清除搜索" onClick={() => ui.setSearchQuery("")}>
          <X size={16} />
        </IconButton>
      ) : null}
    </div>
  );
}
