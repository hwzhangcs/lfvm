import { type ReactNode, useState } from "react";

/**
 * 固定行高的虚拟列表：只渲染可见范围内的行，10 万行文本也能流畅滚动（SRS 3.3.2.2 行数上限）。
 */
export function VirtualRows<T>({
  items,
  rowHeight = 20,
  height,
  render,
  overscan = 20,
}: {
  items: T[];
  rowHeight?: number;
  height: number | string;
  render: (item: T, index: number) => ReactNode;
  overscan?: number;
}) {
  const [scrollTop, setScrollTop] = useState(0);
  const [viewport, setViewport] = useState(600);
  const first = Math.max(0, Math.floor(scrollTop / rowHeight) - overscan);
  const last = Math.min(items.length, Math.ceil((scrollTop + viewport) / rowHeight) + overscan);

  return (
    <div
      style={{ height, overflow: "auto", position: "relative" }}
      onScroll={(e) => setScrollTop(e.currentTarget.scrollTop)}
      ref={(el) => {
        if (el && el.clientHeight !== viewport) setViewport(el.clientHeight);
      }}
    >
      <div style={{ height: items.length * rowHeight, position: "relative" }}>
        {items.slice(first, last).map((item, i) => (
          <div
            // 虚拟列表按行号定位，行号就是稳定的键
            // biome-ignore lint/suspicious/noArrayIndexKey: 行号即位置
            key={first + i}
            style={{ position: "absolute", top: (first + i) * rowHeight, left: 0, right: 0, height: rowHeight }}
          >
            {render(item, first + i)}
          </div>
        ))}
      </div>
    </div>
  );
}
