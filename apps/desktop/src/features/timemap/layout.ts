/**
 * 时间地图布局（SRS 3.3.2.1）。
 *
 * 首版不做方案合并，每个版本只有一个父版本，历史是一棵树：
 * 默认历史是一条链，每个方案是从起点长出的一条链。因此按“保存先后”排横轴、
 * 按“保存时所在方案”分泳道即可，不需要通用的有向图布局。共同祖先只出现一次。
 */
import type { MapNode, MapScheme, TimeMap } from "../../bindings";

export const COL = 72;
export const ROW = 72;
export const PAD = 48;

/** 方案配色（第 0 个用于默认历史）。 */
export const PALETTE = [
  "#1677ff",
  "#52c41a",
  "#fa8c16",
  "#eb2f96",
  "#722ed1",
  "#13c2c2",
  "#faad14",
  "#a0d911",
  "#2f54eb",
  "#f5222d",
];

export function laneColor(lane: number): string {
  return PALETTE[lane % PALETTE.length] ?? "#1677ff";
}

/** 显示范围：全部、仅默认历史、或某个方案（含其共同祖先）。 */
export type MapFilter = { kind: "all" } | { kind: "default" } | { kind: "scheme"; schemeId: string };

export interface PlacedNode {
  node: MapNode;
  x: number;
  y: number;
  lane: number;
  color: string;
}

export interface Edge {
  from: PlacedNode;
  to: PlacedNode;
  color: string;
}

export interface Lane {
  lane: number;
  label: string;
  color: string;
  y: number;
  cleared: boolean;
}

export interface Marker {
  versionId: string;
  label: string;
  color: string;
  kind: "base" | "head";
}

export interface MapLayout {
  nodes: PlacedNode[];
  edges: Edge[];
  lanes: Lane[];
  markers: Marker[];
  width: number;
  height: number;
}

/** 从某个版本沿父版本追溯到最早版本（含自身）。 */
export function ancestors(map: TimeMap, from: string | null): Set<string> {
  const byId = new Map(map.nodes.map((n) => [n.version_id, n]));
  const out = new Set<string>();
  let cur = from ? byId.get(from) : undefined;
  while (cur && !out.has(cur.version_id)) {
    out.add(cur.version_id);
    cur = cur.parent_version_id ? byId.get(cur.parent_version_id) : undefined;
  }
  return out;
}

export function visibleIds(map: TimeMap, filter: MapFilter): Set<string> | null {
  switch (filter.kind) {
    case "all":
      return null;
    case "default":
      return ancestors(map, map.default_head);
    case "scheme": {
      const s = map.schemes.find((x) => x.scheme_id === filter.schemeId);
      return ancestors(map, s?.head_version_id ?? null);
    }
  }
}

export function layoutMap(
  map: TimeMap,
  filter: MapFilter,
  defaultLabel: string,
  schemeLabel: (s: MapScheme) => string,
): MapLayout {
  const laneOf = new Map<string, number>();
  map.schemes.forEach((s, i) => {
    laneOf.set(s.scheme_id, i + 1);
  });
  const visible = visibleIds(map, filter);
  const shown = map.nodes.filter((n) => !visible || visible.has(n.version_id));

  // 只为实际出现的泳道分配纵向位置，避免空行
  const usedLanes = [...new Set(shown.map((n) => (n.origin_scheme_id ? (laneOf.get(n.origin_scheme_id) ?? 0) : 0)))];
  if (!usedLanes.includes(0)) usedLanes.unshift(0);
  usedLanes.sort((a, b) => a - b);
  const rowOf = new Map(usedLanes.map((l, i) => [l, i]));

  const placed: PlacedNode[] = shown.map((node, i) => {
    const lane = node.origin_scheme_id ? (laneOf.get(node.origin_scheme_id) ?? 0) : 0;
    return { node, lane, color: laneColor(lane), x: PAD + i * COL, y: PAD + (rowOf.get(lane) ?? 0) * ROW };
  });
  const byId = new Map(placed.map((p) => [p.node.version_id, p]));
  const edges: Edge[] = [];
  for (const p of placed) {
    const parent = p.node.parent_version_id ? byId.get(p.node.parent_version_id) : undefined;
    if (parent) edges.push({ from: parent, to: p, color: p.color });
  }

  const lanes: Lane[] = usedLanes.map((l) => {
    const s = l > 0 ? map.schemes[l - 1] : undefined;
    return {
      lane: l,
      label: s ? schemeLabel(s) : defaultLabel,
      color: laneColor(l),
      y: PAD + (rowOf.get(l) ?? 0) * ROW,
      cleared: s?.cleared ?? false,
    };
  });

  const markers: Marker[] = [];
  map.schemes.forEach((s, i) => {
    if (s.cleared) return;
    const color = laneColor(i + 1);
    if (byId.has(s.base_version_id)) markers.push({ versionId: s.base_version_id, label: s.name, color, kind: "base" });
    if (byId.has(s.head_version_id)) markers.push({ versionId: s.head_version_id, label: s.name, color, kind: "head" });
  });
  if (map.default_head && byId.has(map.default_head)) {
    markers.push({ versionId: map.default_head, label: defaultLabel, color: laneColor(0), kind: "head" });
  }

  return {
    nodes: placed,
    edges,
    lanes,
    markers,
    width: PAD * 2 + Math.max(0, placed.length - 1) * COL + 160,
    height: PAD * 2 + Math.max(0, usedLanes.length - 1) * ROW + 40,
  };
}

/** 按日期范围（本机时区，起始日 00:00，结束日含全天）和说明关键字（不区分大小写、包含）筛选。 */
export function matchNodes(nodes: MapNode[], opts: { from?: number; to?: number; keyword?: string }): string[] {
  const kw = opts.keyword?.trim().toLowerCase();
  return nodes
    .filter(
      (n) =>
        (opts.from === undefined || n.created_at >= opts.from) && (opts.to === undefined || n.created_at <= opts.to),
    )
    .filter((n) => !kw || n.note.toLowerCase().includes(kw) || n.name.toLowerCase().includes(kw))
    .map((n) => n.version_id);
}
