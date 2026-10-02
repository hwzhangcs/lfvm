import { describe, expect, it } from "vitest";
import type { HistoryEntry } from "../../bindings";
import { buildTree } from "./buildTree";

const f = (path: string): HistoryEntry => ({ path, entry_type: "file", size: 1, hash: "h", available: true });
const d = (path: string): HistoryEntry => ({ path, entry_type: "directory", size: null, hash: null, available: true });

describe("buildTree", () => {
  it("文件夹在前、空文件夹保留", () => {
    const tree = buildTree([f("b.txt"), d("a"), f("a/x.txt"), d("empty")]);
    expect(tree.map((n) => n.name)).toEqual(["a", "empty", "b.txt"]);
    expect(tree[0]?.children.map((n) => n.key)).toEqual(["a/x.txt"]);
    expect(tree[1]?.children).toEqual([]);
  });

  it("补出清单中缺少的上级文件夹", () => {
    const tree = buildTree([f("x/y/z.txt")]);
    expect(tree[0]?.key).toBe("x");
    expect(tree[0]?.children[0]?.key).toBe("x/y");
    expect(tree[0]?.children[0]?.children[0]?.key).toBe("x/y/z.txt");
  });
});
