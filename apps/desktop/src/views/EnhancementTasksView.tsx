import { useEffect, useState } from "react";
import { bridge, hostAvailable } from "../lib/ipc";
import type { EnhancementTask } from "../lib/types";

export function EnhancementTasksView() {
  const [tasks, setTasks] = useState<EnhancementTask[]>([]);
  const refresh = () => { if (hostAvailable) void bridge.enhancementTasks().then(setTasks); };
  useEffect(() => { refresh(); const timer = window.setInterval(refresh, 1000); return () => window.clearInterval(timer); }, []);
  return <section className="scroll-area h-full overflow-y-auto p-6">
    <header className="mb-5"><h1 className="text-lg font-semibold text-ink">照片增强任务</h1><p className="mt-1 text-sm text-ink-3">查看每张照片的处理进度、耗时和输入输出大小。</p></header>
    {!hostAvailable ? <p className="rounded-md border border-line bg-sunken p-4 text-sm text-ink-3">演示模式没有后台增强任务。</p> : tasks.length === 0 ? <p className="rounded-md border border-line bg-sunken p-4 text-sm text-ink-3">还没有增强任务。</p> : <div className="overflow-hidden rounded-lg border border-line bg-surface"><table className="w-full text-left text-xs"><thead className="border-b border-line bg-sunken text-ink-3"><tr><th className="p-3">照片</th><th className="p-3">状态</th><th className="p-3">开始时间</th><th className="p-3">完成时间</th><th className="p-3">耗时</th><th className="p-3">大小</th></tr></thead><tbody>{[...tasks].reverse().map((task) => <tr key={task.id} className="border-b border-line last:border-0"><td className="max-w-64 truncate p-3 text-ink" title={task.fileName}>{task.fileName}</td><td className="p-3">{statusLabel(task.status)}{task.error ? <span className="ml-2 text-danger" title={task.error}>失败详情</span> : null}</td><td className="p-3 tabular text-ink-3">{formatTime(task.startedAt)}</td><td className="p-3 tabular text-ink-3">{task.finishedAt ? formatTime(task.finishedAt) : "处理中"}</td><td className="p-3 tabular text-ink-3">{task.durationMs ? formatDuration(task.durationMs) : "处理中"}</td><td className="p-3 tabular text-ink-3">{formatBytes(task.inputBytes)} → {task.outputBytes ? formatBytes(task.outputBytes) : "处理中"}</td></tr>)}</tbody></table></div>}
  </section>;
}
function statusLabel(status: EnhancementTask["status"]) { return ({ running: "处理中", succeeded: "成功", failed: "失败", cancelled: "已取消", interrupted: "中断" })[status]; }
function formatTime(ms: number) { return new Date(ms).toLocaleString("zh-CN", { hour12: false }); }
function formatDuration(ms: number) { return ms < 1000 ? `${ms} ms` : `${(ms / 1000).toFixed(1)} 秒`; }
function formatBytes(bytes: number) { if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} KB`; return `${(bytes / (1024 * 1024)).toFixed(2)} MB`; }
