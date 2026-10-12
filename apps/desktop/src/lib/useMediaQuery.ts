import { useEffect, useState } from "react";

/**
 * 媒体查询。只用于两处窗口级行为：侧边栏自动折叠、检查器转为覆盖层。
 * 其余布局交给 CSS，这里的 JS 只负责「这个组件在窄窗下该换哪种形态」这种
 * 无法用类名表达的判断。
 */
export function useMediaQuery(query: string): boolean {
  const [matches, setMatches] = useState(() => window.matchMedia(query).matches);

  useEffect(() => {
    const list = window.matchMedia(query);
    const onChange = (event: MediaQueryListEvent) => setMatches(event.matches);
    setMatches(list.matches);
    list.addEventListener("change", onChange);
    return () => list.removeEventListener("change", onChange);
  }, [query]);

  return matches;
}
