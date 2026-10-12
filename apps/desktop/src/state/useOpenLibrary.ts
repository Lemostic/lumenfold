import { useCallback, useRef, useState } from "react";
import { open } from "@tauri-apps/plugin-dialog";
import { dataSource } from "./library";
import { describeError, hostAvailable } from "../lib/ipc";

export interface OpenLibrary {
  /** 拉起系统目录选择器，成功后通知调用方刷新 */
  pick(): Promise<void>;
  rescan(): Promise<void>;
  busy: boolean;
  failure: string | null;
}

/**
 * 打开或重新扫描图库。
 * 目录选择器来自 tauri-plugin-dialog；浏览器里没有这个宿主能力，
 * 所以这里如实报错，而不是给一个点了没反应的按钮。
 */
export function useOpenLibrary(onChanged: () => void): OpenLibrary {
  const [busy, setBusy] = useState(false);
  const [failure, setFailure] = useState<string | null>(null);

  // 父组件传进来的通常是内联箭头函数，放进 ref 才能让 pick/rescan 保持稳定身份。
  const changed = useRef(onChanged);
  changed.current = onChanged;

  /** 包一层 busy 与错误。动作成功才通知调用方刷新，失败就地展示，不外抛。 */
  const execute = useCallback(async (action: () => Promise<unknown>) => {
    setBusy(true);
    setFailure(null);
    try {
      await action();
      changed.current();
    } catch (error: unknown) {
      setFailure(describeError(error));
    } finally {
      setBusy(false);
    }
  }, []);

  /**
   * 选择目录。busy 覆盖整个流程，包括系统对话框还开着的时候，
   * 否则按钮一直是可点的，连按两下会拉起两个对话框。
   *
   * 用户在对话框里点取消既不是成功也不是失败：不刷新图库，也不留一条红色错误。
   * 之前这里把 pick 套进 execute、pick 内部又调了一次 execute，
   * 于是取消和选中都会走两遍 onChanged。
   */
  const pick = useCallback(async () => {
    setBusy(true);
    setFailure(null);
    try {
      const chosen = await open({ directory: true, multiple: false, title: "选择照片目录" });
      if (typeof chosen !== "string") return;
      await dataSource.open(chosen);
      changed.current();
    } catch (error: unknown) {
      setFailure(describeError(error));
    } finally {
      setBusy(false);
    }
  }, []);

  return {
    pick,
    rescan: () => execute(() => dataSource.rescan()),
    busy,
    failure,
  };
}

export const canPickDirectory = hostAvailable;
