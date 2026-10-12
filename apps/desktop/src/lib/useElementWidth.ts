import { useEffect, useRef, useState } from "react";

interface ElementSize {
  width: number;
  height: number;
}

/**
 * 容器尺寸。justified 网格必须按实际像素重算,
 * 用 ResizeObserver 而不是 window.resize,窗口缩放之外的布局变化也能跟上。
 *
 * 之所以返回 width 和 height 两个值,是因为 WallView 单图大图模式需要按容器高度
 * 计算主图区域的最大尺寸——它在受限的高度下保持宽高比居中。
 */
export function useElementWidth<T extends HTMLElement>() {
  const ref = useRef<T>(null);
  const [size, setSize] = useState<ElementSize>({ width: 0, height: 0 });

  useEffect(() => {
    const element = ref.current;
    if (!element) return;

    /**
     * 取整到整数像素。
     *
     * 子像素宽度会因为滚动条的存在而与真实宽度有差,若把它直接当宽度用,
     * `justifyRows` 的 memo 键就会持续变化、整个网格一直重算。整数化让宽度真正变了才重算。
     */
    const round = (value: number) => Math.round(value);

    const apply = (rect: { width: number; height: number }) =>
      setSize((prev) => {
        const next = { width: round(rect.width), height: round(rect.height) };
        if (prev.width === next.width && prev.height === next.height) return prev;
        return next;
      });

    const observer = new ResizeObserver((entries) => {
      const entry = entries[0];
      if (entry) apply(entry.contentRect);
    });
    observer.observe(element);
    apply(element.getBoundingClientRect());

    return () => observer.disconnect();
  }, []);

  // 同时保留 width 字段,避免下游 .width 解构断裂。
  return { ref, width: size.width, height: size.height };
}
