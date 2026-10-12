import { useCallback, useState } from "react";
import { dataSource } from "./library";
import { ui } from "./ui";
import { bridge, describeError, hostAvailable } from "../lib/ipc";
import { type BatchAction, type BatchReport } from "../lib/types";

export interface BatchState {
  /** 正在等用户点「确认」的那个动作。删除是不可撤销的一半，必须二次确认。 */
  awaitingConfirm: BatchAction | null;
  busy: boolean;
  failure: string | null;
  /** 最近一次批量操作的结果，用于如实汇报哪几张没成 */
  report: BatchReport | null;
  clipboardMessage: string | null;
  /** 复制/剪切写入系统剪贴板；删除先进入确认态。 */
  request(action: BatchAction): Promise<void>;
  /** 用户在确认态点了「确认执行」 */
  confirm(): Promise<void>;
  /** 用户在确认态点了「算了」 */
  cancel(): void;
  dismissReport(): void;
}

/**
 * 批量操作的编排：复制/剪切写入剪贴板，删除二次确认并汇报结果。
 *
 * 删除之所以要多一步，不是因为它要显得危险，而是因为它和复制不对称：
 * 复制出问题顶多多一个文件，删除出问题照片就没了。所以按钮会先变成
 * 「确认删除 N 项」，第二次点击才真的执行。
 *
 * 选区由调用方传入而不是在这里订阅：这个 hook 只关心「拿到一批 id 之后怎么做」，
 * 不该知道选区存在 ui store 的哪个字段上。
 */
export function useBatch(selectedIds: readonly string[], onChanged: () => void): BatchState {
  const [awaitingConfirm, setAwaitingConfirm] = useState<BatchAction | null>(null);
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);
  const [report, setReport] = useState<BatchReport | null>(null);
  const [clipboardMessage, setClipboardMessage] = useState<string | null>(null);

  const send = useCallback(
    async (action: BatchAction, destination: string | null, selectedIds: string[]) => {
      setBusy(true);
      setFailure(null);
      try {
        const result = await dataSource.batchOperate({
          assetIds: selectedIds,
          action,
          destination,
        });
        setReport(result);
        if (result.succeeded > 0) {
          // 资产 id 是路径的哈希，移动和删除之后这批 id 要么变了要么不存在了。
          // 还留着它们，下一次批量操作就会收到一串「找不到文件」——
          // 那是上一轮操作的残渣，不是用户新挑的东西。
          ui.clearSelection();
          onChanged();
        }
      } catch (error: unknown) {
        setFailure(describeError(error));
      } finally {
        setBusy(false);
      }
    },
    [onChanged],
  );

  const run = useCallback(
    async (action: BatchAction) => {
      setReport(null);
      setClipboardMessage(null);
      const ids = [...selectedIds];
      if (ids.length === 0) return;

      if (action === "copy" || action === "move") {
        if (!hostAvailable) {
          setFailure("演示图库没有真实文件，无法写入系统剪贴板");
          return;
        }
        setBusy(true);
        setFailure(null);
        try {
          const count = await bridge.clipboardFiles(ids, action === "move");
          setClipboardMessage(`已${action === "move" ? "剪切" : "复制"} ${count} 个文件，可在资源管理器中粘贴`);
        } catch (error: unknown) {
          setFailure(describeError(error));
        } finally {
          setBusy(false);
        }
        return;
      }

      setAwaitingConfirm(action);
    },
    [selectedIds],
  );

  const confirm = useCallback(async () => {
    const action = awaitingConfirm;
    if (action === null) return;
    setAwaitingConfirm(null);
    await send(action, null, [...selectedIds]);
  }, [awaitingConfirm, selectedIds, send]);

  return {
    awaitingConfirm,
    busy,
    failure,
    report,
    clipboardMessage,
    request: run,
    confirm,
    cancel: () => setAwaitingConfirm(null),
    dismissReport: () => setReport(null),
  };
}
