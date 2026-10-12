import { useState } from "react";
import { CheckCircle, Copy, Scissors, Trash, WarningCircle, X } from "@phosphor-icons/react";
import { Button } from "./Button";
import { Switch } from "./Switch";
import { formatCount } from "../lib/format";
import type { BatchReport } from "../lib/types";

interface BatchBarProps {
  selectedCount: number;
  busy: boolean;
  /** 删除的二次确认态。null 表示不在确认中。 */
  awaitingConfirm: boolean;
  failure: string | null;
  report: BatchReport | null;
  clipboardMessage: string | null;
  onCopy(): void;
  onMove(): void;
  onDelete(): void;
  onConfirm(): void;
  onCancelConfirm(): void;
  onDismissReport(): void;
  onClearSelection(): void;
  onSelectAll(): void;
  onInvert(): void;
  /** 标记模式是否开启。选中一有东西就会整条换成当前条， */
  markMode: boolean;
  onToggleMarkMode(): void;
}

/**
 * 批量操作条。
 *
 * 它在工具栏的位置上整条替换掉筛选控件：有标记项的时候，
 * 「怎么浏览」让位给「对这批照片做什么」，两者不同时占着同一块地方。
 */
export function BatchBar({
  selectedCount,
  busy,
  awaitingConfirm,
  failure,
  report,
  clipboardMessage,
  onCopy,
  onMove,
  onDelete,
  onConfirm,
  onCancelConfirm,
  onDismissReport,
  onClearSelection,
  onSelectAll,
  onInvert,
  markMode,
  onToggleMarkMode,
}: BatchBarProps) {
  if (selectedCount === 0) return null;


  return (
    <div className="flex h-11 shrink-0 items-center gap-2 border-b border-line bg-surface px-3">
      <span className="tabular text-xs font-medium text-ink">
        已标记 {formatCount(selectedCount)} 项
      </span>

      <div className="h-5 w-px bg-line" />

      {awaitingConfirm ? (
        <>
          <span className="text-xs text-danger">
            删除后照片会进回收站，{formatCount(selectedCount)} 项都要删掉。
          </span>
          <div className="h-5 w-px bg-line" />
          <Button size="sm" variant="danger" disabled={busy} onClick={onConfirm}>
            确认删除 {formatCount(selectedCount)} 项
          </Button>
          <Button size="sm" variant="ghost" disabled={busy} onClick={onCancelConfirm}>
            算了
          </Button>
        </>
      ) : (
        <>
          <Button
            size="sm"
            variant="secondary"
            icon={<Copy size={13} />}
            disabled={busy}
            onClick={onCopy}
          >
            复制
          </Button>
          <Button
            size="sm"
            variant="secondary"
            icon={<Scissors size={13} />}
            disabled={busy}
            onClick={onMove}
          >
            剪切
          </Button>
          <Button
            size="sm"
            variant="ghost"
            icon={<Trash size={13} />}
            disabled={busy}
            onClick={onDelete}
          >
            删除
          </Button>
          <div className="h-5 w-px bg-line" />
          <Button size="sm" variant="ghost" disabled={busy} onClick={onSelectAll}>
            全选
          </Button>
          <Button size="sm" variant="ghost" disabled={busy} onClick={onInvert}>
            反选
          </Button>
        </>
      )}

      <div className="flex-1" />

      {busy ? <span className="text-2xs text-ink-3">正在处理…</span> : null}

      {failure ? (
        <span className="flex items-center gap-1 text-2xs text-danger">
          <WarningCircle size={12} />
          {failure}
        </span>
      ) : null}

      {clipboardMessage ? <span role="status" className="text-2xs text-accent">{clipboardMessage}</span> : null}
      {report ? <BatchOutcome report={report} onDismiss={onDismissReport} /> : null}

      <Button size="sm" variant="ghost" disabled={busy} onClick={onClearSelection}>
        取消标记
      </Button>

      {/*
        标记模式开关必须在 BatchBar 里再放一份:有选中项时 Toolbar 整条被本条
        替掉,用户在 BatchBar 上看不到原 Toolbar 的开关,就没路径退出标记模式。
        用 Switch 与 Toolbar 保持视觉一致。
      */}
      <Switch
        label="切换标记模式"
        on={markMode}
        onChange={onToggleMarkMode}
        onLabel="标记模式"
        offLabel="浏览模式"
      />
    </div>
  );
}

/**
 * 批量结果的汇报。
 *
 * 必须逐条说清楚：有几张成了、哪几张没成、为什么。
 * 只回一个「完成」会把「50 张里失败了 3 张」说成成功，而那 3 张用户以为已经删掉了。
 */
function BatchOutcome({ report, onDismiss }: { report: BatchReport; onDismiss(): void }) {
  const [open, setOpen] = useState(false);
  const failures = report.items.filter((item) => !item.ok);

  return (
    <div className="flex items-center gap-1.5">
      <button
        type="button"
        onClick={() => setOpen((value) => !value)}
        className={[
          "inline-flex items-center gap-1 rounded-md px-2 py-1 text-2xs transition-colors",
          report.failed > 0
            ? "bg-danger-soft text-danger hover:bg-danger hover:text-accent-ink"
            : "bg-accent-soft text-accent hover:bg-accent hover:text-accent-ink",
        ].join(" ")}
      >
        {report.failed > 0 ? <WarningCircle size={12} /> : <CheckCircle size={12} />}
        成功 {formatCount(report.succeeded)}
        {report.failed > 0 ? ` · 失败 ${formatCount(report.failed)}` : ""}
      </button>

      {open && failures.length > 0 ? (
        <span className="max-w-72 truncate text-2xs text-ink-3" title={failureSummary(failures)}>
          {failureSummary(failures)}
        </span>
      ) : null}

      <button
        type="button"
        onClick={onDismiss}
        aria-label="关闭结果提示"
        className="grid h-5 w-5 place-items-center rounded-xs text-ink-3 transition-colors hover:bg-surface-2 hover:text-ink"
      >
        <X size={12} />
      </button>
    </div>
  );
}

function failureSummary(failures: BatchReport["items"]): string {
  return failures
    .slice(0, 3)
    .map((item) => `${item.fileName}：${item.message}`)
    .join("；");
}
