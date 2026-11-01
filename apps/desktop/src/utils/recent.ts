/**
 * 记录各项目最近一次打开的时间，用于首页“继续上次的工作”和排序。
 * 只存在本机的浏览器存储中；读写失败（如存储被禁用）时当作没有记录，不影响使用。
 */
const KEY = "lfvm.recentProjects";

export function recentOpened(): Record<string, number> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    return v && typeof v === "object" ? v : {};
  } catch {
    return {};
  }
}

export function markOpened(projectId: string): void {
  try {
    localStorage.setItem(KEY, JSON.stringify({ ...recentOpened(), [projectId]: Date.now() }));
  } catch {
    // 忽略：没有记录只影响首页排序
  }
}
