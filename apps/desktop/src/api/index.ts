/**
 * 后端调用入口。页面只通过这里访问后端，不直接 import bindings。
 * bindings.ts 由 Rust 端自动生成（`cargo test -p lfvm-desktop` 或 `pnpm tauri dev`）。
 */
import { Channel, convertFileSrc, isTauri } from "@tauri-apps/api/core";
import { commands, events, type ProgressEvent } from "../bindings";

export type {
  Action,
  AddCheck,
  AppInfo,
  BackupSummary,
  ChangeItem,
  ChangeKind,
  ChangeSet,
  CompareItem,
  CompareKind,
  CompareResult,
  CoreError,
  DiffLine,
  DirChild,
  ErrorCode,
  ExclusionRule,
  FileDiff,
  FilePreview,
  FileRestoreCheck,
  FileTargetArg,
  HistoryEntry,
  ImageSide,
  ImpactPlan,
  ItemState,
  MapNode,
  MapScheme,
  OperationDetail,
  OperationResult,
  OperationSummary,
  OpStatus,
  OpType,
  PickedFolder,
  PlanItem,
  PreviewContent,
  ProgressEvent,
  ProjectOverview,
  ProjectSummary,
  RuleType,
  RuleView,
  SaveResult,
  SourceRef,
  Stage,
  TimeMap,
  VersionBrief,
} from "../bindings";

/** 是否运行在桌面程序中（否则是在浏览器里预览界面）。 */
export const inDesktop = isTauri();

export const api = commands;
export const appEvents = events;

/** 统一的查询键，便于操作完成后刷新相关数据。 */
export const queryKeys = {
  appInfo: ["app-info"] as const,
  projects: ["projects"] as const,
  overview: (projectId: string) => ["overview", projectId] as const,
  changes: (projectId: string) => ["changes", projectId] as const,
  exclusions: (projectId: string) => ["exclusions", projectId] as const,
  workspaceDir: (projectId: string, dir: string | null) => ["workspace-dir", projectId, dir] as const,
  timeMap: (projectId: string) => ["time-map", projectId] as const,
  sourceFiles: (projectId: string, sourceKey: string) => ["source-files", projectId, sourceKey] as const,
  preview: (projectId: string, sourceKey: string, path: string) => ["preview", projectId, sourceKey, path] as const,
  compare: (projectId: string, a: string, b: string, scope: string | null) =>
    ["compare", projectId, a, b, scope] as const,
  diff: (projectId: string, a: string, b: string, path: string) => ["diff", projectId, a, b, path] as const,
  operations: (projectId: string) => ["operations", projectId] as const,
  operation: (projectId: string, operationId: string) => ["operation", projectId, operationId] as const,
  incomplete: (projectId: string) => ["incomplete", projectId] as const,
};

/** 改写工作区或历史的操作完成后，刷新该项目的全部相关数据。 */
export function invalidateProject(qc: import("@tanstack/react-query").QueryClient, projectId: string) {
  return qc.invalidateQueries({
    predicate: (q) => Array.isArray(q.queryKey) && q.queryKey[1] === projectId,
  });
}

/** 历史图片的地址：由预览协议提供，只能取到本项目已登记的 PNG/JPEG 内容（LFVM-Q-07）。 */
export function imageUrl(projectId: string, hash: string): string {
  return convertFileSrc(`${projectId}.${hash}`, "lfvm-preview");
}

/** 用作查询键的来源标识。 */
export function sourceKey(s: import("../bindings").SourceRef): string {
  return s.kind === "version" ? `v:${s.version_id}` : `b:${s.backup_id}`;
}

export function newId(): string {
  return crypto.randomUUID();
}

/** 后端返回的错误（CoreError）或其他异常的说明文字。 */
export function errorMessage(e: unknown): string {
  if (e && typeof e === "object" && "message" in e && typeof e.message === "string") return e.message;
  if (typeof e === "string") return e;
  return "发生了未知错误";
}

export function errorCode(e: unknown): string | undefined {
  return e && typeof e === "object" && "code" in e && typeof e.code === "string" ? e.code : undefined;
}

/**
 * 运行带进度的长任务。`start` 收到任务 ID 和进度通道并发起命令；
 * 传入 `signal` 时，中止会请求后端取消该任务。
 */
export function withProgress<T>(
  start: (taskId: string, channel: Channel<ProgressEvent>) => Promise<T>,
  onProgress: (p: ProgressEvent) => void,
  options: { taskId?: string; signal?: AbortSignal } = {},
): Promise<T> {
  const taskId = options.taskId ?? newId();
  const channel = new Channel<ProgressEvent>();
  channel.onmessage = onProgress;
  options.signal?.addEventListener("abort", () => void api.cancelTask(taskId));
  return start(taskId, channel);
}
