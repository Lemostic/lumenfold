import { SidebarSimple } from "@phosphor-icons/react";
import type { ReactNode } from "react";
import { Rating } from "../components/Rating";
import { IconButton } from "../components/Button";
import { useAssetMetadata } from "../state/library";
import { ui, useUi } from "../state/ui";
import {
  formatAperture,
  formatBytes,
  formatDateTime,
  formatFocalLength,
  formatGps,
  formatMegapixels,
} from "../lib/format";
import type { DemoAsset } from "../lib/fixtures";

interface InspectorProps {
  asset: DemoAsset | null;
  selectedCount: number;
}

/**
 * 检查器。窄窗(<1180px)下脱离布局列、浮在内容之上;宽窗下是常驻的第三列,
 * 元数据不占用内容区。
 *
 * 问题 7:之前工具栏右上角有一个「展开/收起检查器」图标按钮,
 * 检查器自己的标题栏上还有一个收起按钮,两个图标都是 SidebarSimple。
 * 现在工具栏那一处只在检查器关闭时显示,检查器打开时让位给检查器自身。
 */
export function Inspector({ asset, selectedCount }: InspectorProps) {
  const inspectorOpen = useUi((s) => s.inspectorOpen);

  if (!inspectorOpen) return null;

  return (
    <aside
      aria-label="检查器"
      className={[
        "flex w-72 shrink-0 flex-col border-l border-line bg-surface",
        // 窄窗(<1180px)下内容区优先,检查器浮在上层,不挤压网格宽度
        "max-[1179px]:absolute max-[1179px]:inset-y-0 max-[1179px]:right-0",
        "max-[1179px]:z-[var(--lf-z-overlay)] max-[1179px]:shadow-pop",
      ].join(" ")}
    >
      <div className="flex h-11 shrink-0 items-center justify-between gap-2 border-b border-line px-3">
        <h2 className="min-w-0 truncate text-sm font-medium text-ink">
          {asset ? asset.fileName : selectedCount > 0 ? `已选 ${selectedCount} 项` : "检查器"}
        </h2>
        <IconButton label="收起检查器" onClick={() => ui.toggleInspector()}>
          <SidebarSimple size={16} />
        </IconButton>
      </div>

      <div className="min-h-0 flex-1 overflow-y-auto scroll-area">
        {asset ? <AssetDetail asset={asset} /> : <BulkDetail count={selectedCount} />}
      </div>
    </aside>
  );
}

function AssetDetail({ asset }: { asset: DemoAsset }) {
  const metadata = useAssetMetadata(asset.id);

  return (
    <div className="flex flex-col">
      <section className="border-b border-line p-3">
        <img
          src={asset.thumbUrl}
          alt=""
          className="aspect-[3/2] w-full rounded-md border border-line bg-sunken object-cover"
          loading="lazy"
          decoding="async"
        />
        <div className="mt-3 flex items-center justify-between gap-2">
          <Rating value={asset.rating} />
          <span className="tabular text-2xs text-ink-3">
            {asset.rating === 0 ? "未评分" : `${asset.rating} 星`}
          </span>
        </div>
      </section>

      <section className="border-b border-line p-3">
        <h3 className="mb-2 text-xs font-medium text-ink-2">文件</h3>
        <Meta>
          <Term>尺寸</Term>
          <Value>
            {asset.width} × {asset.height}
            <span className="text-ink-3"> · {formatMegapixels(asset.width, asset.height)}</span>
          </Value>
          <Term>大小</Term>
          <Value>{formatBytes(asset.bytes)}</Value>
          <Term>拍摄时间</Term>
          <Value>{formatDateTime(asset.takenAt)}</Value>
          <Term>方向</Term>
          <Value>
            {asset.orientation === "square"
              ? "方形"
              : asset.orientation === "landscape"
                ? "横向"
                : "纵向"}
          </Value>
          <Term>路径</Term>
          <Value>
            <span className="selectable break-all">{asset.path}</span>
          </Value>
        </Meta>
      </section>

      <section className="border-b border-line p-3">
        <h3 className="mb-2 text-xs font-medium text-ink-2">拍摄参数</h3>
        {metadata.status === "loading" ? (
          <div className="flex flex-col gap-1.5">
            {[0, 1, 2, 3].map((i) => (
              <div key={i} className="skeleton h-3 rounded-xs" style={{ width: `${70 - i * 8}%` }} />
            ))}
          </div>
        ) : metadata.status === "error" ? (
          <p className="text-xs leading-relaxed text-ink-3">
            读不到拍摄参数:{metadata.message}
          </p>
        ) : metadata.value ? (
          <Meta>
            <Term>机身</Term>
            <Value>
              {[metadata.value.cameraMake, metadata.value.cameraModel]
                .filter(Boolean)
                .join(" ") || "未知"}
            </Value>
            <Term>镜头</Term>
            <Value>{metadata.value.lens ?? "未知"}</Value>
            <Term>光圈</Term>
            <Value>{formatAperture(metadata.value.aperture)}</Value>
            <Term>快门</Term>
            <Value>{metadata.value.shutter ?? "未知"}</Value>
            <Term>感光度</Term>
            <Value>{metadata.value.iso === null ? "未知" : `ISO ${metadata.value.iso}`}</Value>
            <Term>焦距</Term>
            <Value>{formatFocalLength(metadata.value.focalLengthMm)}</Value>
            <Term>位置</Term>
            <Value>
              <span className="tabular">{formatGps(metadata.value.gpsLat, metadata.value.gpsLon)}</span>
            </Value>
          </Meta>
        ) : null}
      </section>

      {metadata.status === "ready" && metadata.value ? (
        <>
          <section className="border-b border-line p-3">
            <h3 className="mb-2 text-xs font-medium text-ink-2">关键词</h3>
            {metadata.value.keywords.length === 0 ? (
              <p className="text-xs text-ink-3">这张照片还没有关键词</p>
            ) : (
              <ul className="flex flex-wrap gap-1.5">
                {metadata.value.keywords.map((keyword) => (
                  <li
                    key={keyword}
                    className="rounded-full border border-line bg-surface-2 px-2 py-0.5 text-2xs text-ink-2"
                  >
                    {keyword}
                  </li>
                ))}
              </ul>
            )}
          </section>

          <section className="p-3">
            <h3 className="mb-2 text-xs font-medium text-ink-2">说明</h3>
            <p className="selectable text-xs leading-relaxed whitespace-pre-wrap text-ink-2">
              {metadata.value.caption ?? "还没有说明文字"}
            </p>
          </section>
        </>
      ) : null}
    </div>
  );
}

function BulkDetail({ count }: { count: number }) {
  if (count === 0) {
    return (
      <div className="p-4">
        <p className="text-xs leading-relaxed text-ink-3">
          选中一张照片后,这里显示它的拍摄参数与路径。
          <br />
          方向键移动光标,空格加入选区。
        </p>
      </div>
    );
  }

  return (
    <div className="p-4">
      <p className="tabular text-xs text-ink-2">共 {count} 项</p>
      <p className="mt-2 text-xs leading-relaxed text-ink-3">
        批量评分、关键词合并与 XMP 写回会在写入功能交付后出现在这一栏。
      </p>
    </div>
  );
}

function Meta({ children }: { children: ReactNode }) {
  return <dl className="grid grid-cols-[5rem_1fr] gap-x-2 gap-y-1.5 text-xs">{children}</dl>;
}

function Term({ children }: { children: ReactNode }) {
  return <dt className="truncate text-ink-3">{children}</dt>;
}

function Value({ children }: { children: ReactNode }) {
  return <dd className="min-w-0 break-words text-ink-2 tabular-nums">{children}</dd>;
}
