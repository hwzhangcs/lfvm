import type { HistoryEntry } from "../../bindings";

export interface HNode {
  key: string;
  name: string;
  isDir: boolean;
  entry: HistoryEntry | null;
  children: HNode[];
}

/**
 * 把扁平的历史清单整理成目录树。清单中缺少的上级文件夹（如安全备份只含受影响的文件）会补出来。
 * 文件夹排在文件前面，同类按名称排序。
 */
export function buildTree(entries: HistoryEntry[]): HNode[] {
  const root: HNode = { key: "", name: "", isDir: true, entry: null, children: [] };
  const nodes = new Map<string, HNode>([["", root]]);

  const ensureDir = (path: string): HNode => {
    const found = nodes.get(path);
    if (found) return found;
    const cut = path.lastIndexOf("/");
    const parent = ensureDir(cut < 0 ? "" : path.slice(0, cut));
    const node: HNode = { key: path, name: path.slice(cut + 1), isDir: true, entry: null, children: [] };
    parent.children.push(node);
    nodes.set(path, node);
    return node;
  };

  for (const e of entries) {
    if (e.entry_type === "directory") {
      ensureDir(e.path).entry = e;
    } else {
      const cut = e.path.lastIndexOf("/");
      const parent = ensureDir(cut < 0 ? "" : e.path.slice(0, cut));
      parent.children.push({ key: e.path, name: e.path.slice(cut + 1), isDir: false, entry: e, children: [] });
    }
  }

  const sort = (n: HNode) => {
    n.children.sort((a, b) => (a.isDir === b.isDir ? a.name.localeCompare(b.name, "zh-CN") : a.isDir ? -1 : 1));
    n.children.forEach(sort);
  };
  sort(root);
  return root.children;
}
