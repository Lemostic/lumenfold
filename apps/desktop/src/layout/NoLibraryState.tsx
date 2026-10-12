import { FolderOpen } from "@phosphor-icons/react";
import { Button } from "../components/Button";
import { canPickDirectory, useOpenLibrary } from "../state/useOpenLibrary";

interface NoLibraryStateProps {
  onChanged(): void;
}

/**
 * 没有打开图库时的内容区。
 * 这是应用真正的首屏,不是一个错误:一个还没指向任何目录的图库管理器是正常状态,
 * 把它渲染成错误页会让用户以为软件坏了。
 */
export function NoLibraryState({ onChanged }: NoLibraryStateProps) {
  const { pick, rescan, busy, failure } = useOpenLibrary(onChanged);

  return (
    <div className="flex h-full min-h-0 items-center justify-center px-8 py-12">
      <div className="max-w-sm">
        <div className="mb-5 flex h-12 w-12 items-center justify-center rounded-lg border border-line bg-surface text-accent">
          <FolderOpen size={22} />
        </div>
        <h2 className="text-md font-medium tracking-tight text-ink">还没有打开图库</h2>
        <p className="mt-2 text-sm leading-relaxed text-ink-2 text-pretty">
          选一个装照片的目录,Lumenfold 会就地读取并建立索引。原图不会被移动或复制,索引与缩略图留在本机。
        </p>

        {failure ? (
          <p className="mt-3 rounded-md border border-danger-line bg-danger-soft p-2.5 text-xs leading-relaxed text-danger">
            {failure}
          </p>
        ) : null}

        <div className="mt-5 flex items-center gap-2">
          <Button
            variant="primary"
            size="md"
            icon={<FolderOpen size={15} />}
            disabled={!canPickDirectory || busy}
            onClick={() => void pick()}
          >
            选择目录
          </Button>
          <Button variant="ghost" size="md" disabled={!canPickDirectory || busy} onClick={() => void rescan()}>
            重新扫描
          </Button>
        </div>

        {!canPickDirectory ? (
          <p className="mt-4 text-xs leading-relaxed text-ink-3">
            目录选择器只在桌面应用里可用。浏览器里跑 <code className="rounded bg-sunken px-1 py-0.5 text-2xs">pnpm dev</code> 时使用的是内置演示图库。
          </p>
        ) : null}
      </div>
    </div>
  );
}
