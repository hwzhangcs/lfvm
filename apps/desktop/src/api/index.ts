/**
 * 后端调用入口。页面只通过这里访问后端，不直接 import bindings。
 * bindings.ts 由 Rust 端自动生成（`cargo test -p lfvm-desktop` 或 `pnpm tauri dev`）。
 */
import { Channel, isTauri } from "@tauri-apps/api/core";
import { commands, events, type ProgressEvent } from "../bindings";

export type {
  AddCheck,
  AppInfo,
  ChangeItem,
  ChangeKind,
  ChangeSet,
  CoreError,
  DirChild,
  ErrorCode,
  ExclusionRule,
  PickedFolder,
  ProgressEvent,
  ProjectOverview,
  ProjectSummary,
  RuleType,
  RuleView,
  SaveResult,
  Stage,
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
};

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
