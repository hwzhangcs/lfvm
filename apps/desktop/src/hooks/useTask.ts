import type { Channel } from "@tauri-apps/api/core";
import { useCallback, useRef, useState } from "react";
import { api, newId, type ProgressEvent, withProgress } from "../api";

/** 运行一个带进度、可取消的长任务（LFVM-P-10）。 */
export function useTask() {
  const [progress, setProgress] = useState<ProgressEvent | null>(null);
  const [running, setRunning] = useState(false);
  const taskId = useRef<string | null>(null);

  const run = useCallback(async <T>(start: (task: string, ch: Channel<ProgressEvent>) => Promise<T>): Promise<T> => {
    const id = newId();
    taskId.current = id;
    setRunning(true);
    setProgress(null);
    try {
      return await withProgress(start, setProgress, { taskId: id });
    } finally {
      taskId.current = null;
      setRunning(false);
    }
  }, []);

  const cancel = useCallback(() => {
    if (taskId.current) void api.cancelTask(taskId.current);
  }, []);

  return { run, cancel, progress, running };
}
