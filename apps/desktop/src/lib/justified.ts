/**
 * justified 行布局（Google Photos 那一套）。
 *
 * 关键取舍：行高固定，只有列宽浮动。
 * 变高行需要逐行测量盒子，十万张滚动时测量开销会让帧率塌掉；
 * 固定行高让虚拟滚动可以纯算术定位，缩略图加载也不产生布局跳动。
 * 代价是每行右端可能留一点空隙，这正是 Google Photos 的观感。
 */

export interface JustifyMeasure {
  width: number;
  height: number;
}

export interface JustifyRow<T> {
  cells: Array<{ data: T; width: number; height: number }>;
  /** 行的自然宽度，可能略小于容器宽度 */
  width: number;
  /** 统一行高。超出容器宽度的极端竖图会有一行更矮。 */
  height: number;
}

/**
 * 把有序条目切成若干行。
 * @param measure 从条目取出原始宽高。列表元素通常就是 { width, height }，
 *   但真实资产可能缺尺寸，需要在测量时决定占位比例，所以 measure 单独传入。
 */
export function justifyRows<T>(
  items: readonly T[],
  measure: (item: T) => JustifyMeasure,
  containerWidth: number,
  targetHeight: number,
  gap: number,
): JustifyRow<T>[] {
  if (items.length === 0 || containerWidth <= 0) return [];

  const rows: JustifyRow<T>[] = [];
  let cells: JustifyRow<T>["cells"] = [];
  let used = 0;

  const flush = () => {
    if (cells.length === 0) return;
    rows.push({ cells, width: used, height: targetHeight });
    cells = [];
    used = 0;
  };

  for (const item of items) {
    const { width: sourceWidth, height: sourceHeight } = measure(item);
    const aspect = sourceWidth / sourceHeight;
    const cellWidth = aspect * targetHeight;

    // 竖图在目标行高下宽过容器：单独成行并按容器宽度等比压下去
    if (cellWidth > containerWidth) {
      flush();
      const fitted = { data: item, width: containerWidth, height: containerWidth / aspect };
      rows.push({ cells: [fitted], width: containerWidth, height: fitted.height });
      continue;
    }

    // 先判断当前照片能否放入这一行；如果不能，结束旧行后必须从
    // 当前照片重新计算新行的宽度，不能沿用溢出前的累计值。
    const nextUsed = used === 0 ? cellWidth : used + gap + cellWidth;
    if (nextUsed > containerWidth && cells.length > 0) {
      flush();
    }

    used = used === 0 ? cellWidth : used + gap + cellWidth;
    cells.push({ data: item, width: cellWidth, height: targetHeight });
  }

  flush();
  return rows;
}
