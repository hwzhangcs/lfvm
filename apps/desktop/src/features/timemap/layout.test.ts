import { describe, expect, it } from "vitest";
import type { MapNode, TimeMap } from "../../bindings";
import { ancestors, layoutMap, matchNodes } from "./layout";

function node(seq: number, parent: number | null, scheme: string | null = null, extra: Partial<MapNode> = {}): MapNode {
  return {
    version_id: `v${seq}`,
    seq,
    parent_version_id: parent === null ? null : `v${parent}`,
    origin_scheme_id: scheme,
    origin_scheme_name: scheme,
    name: "",
    note: "",
    created_at: seq * 1000,
    file_count: 0,
    directory_count: 0,
    added: 0,
    modified: 0,
    deleted: 0,
    cleared: false,
    ...extra,
  };
}

// 默认历史 v1→v2→v4；方案 A 从 v2 产生：v2→v3→v5
const map: TimeMap = {
  nodes: [node(1, null), node(2, 1), node(3, 2, "A"), node(4, 2), node(5, 3, "A")],
  schemes: [
    { scheme_id: "A", name: "方案A", base_version_id: "v2", head_version_id: "v5", created_at: 0, cleared: false },
  ],
  default_head: "v4",
  active_scheme_id: null,
  over_scale: false,
};

describe("时间地图布局", () => {
  it("按方案分泳道，共同祖先只出现一次", () => {
    const l = layoutMap(map, { kind: "all" }, "默认历史", (s) => s.name);
    expect(l.nodes).toHaveLength(5);
    const lane = Object.fromEntries(l.nodes.map((p) => [p.node.version_id, p.lane]));
    expect(lane).toEqual({ v1: 0, v2: 0, v3: 1, v4: 0, v5: 1 });
    expect(l.edges).toHaveLength(4);
    expect(l.markers.find((m) => m.kind === "base")?.versionId).toBe("v2");
  });

  it("单个方案视图包含其共同祖先（AC-0026）", () => {
    const l = layoutMap(map, { kind: "scheme", schemeId: "A" }, "默认历史", (s) => s.name);
    expect(l.nodes.map((p) => p.node.version_id)).toEqual(["v1", "v2", "v3", "v5"]);
    const d = layoutMap(map, { kind: "default" }, "默认历史", (s) => s.name);
    expect(d.nodes.map((p) => p.node.version_id)).toEqual(["v1", "v2", "v4"]);
  });

  it("沿父版本追溯", () => {
    expect([...ancestors(map, "v5")]).toEqual(["v5", "v3", "v2", "v1"]);
    expect(ancestors(map, null).size).toBe(0);
  });

  it("按日期和关键字定位", () => {
    const nodes = [node(1, null, null, { note: "交给老师" }), node(2, 1, null, { name: "最终版" }), node(3, 2)];
    expect(matchNodes(nodes, { keyword: "老师" })).toEqual(["v1"]);
    expect(matchNodes(nodes, { keyword: "最终" })).toEqual(["v2"]);
    expect(matchNodes(nodes, { from: 2000, to: 3000 })).toEqual(["v2", "v3"]);
  });
});
