import { Button, Flex, Progress, Typography } from "antd";
import type { ProgressEvent } from "../api";
import { t } from "../locales/zh-CN";
import { formatBytes } from "../utils/format";

/** 长任务进度：显示当前阶段、进度条和“停止”按钮（LFVM-P-10）。 */
export function TaskProgress({ progress, onCancel }: { progress: ProgressEvent | null; onCancel?: () => void }) {
  const stage = progress ? t.stage[progress.stage] : t.common.working;
  const known = progress && progress.total > 0;
  const percent = known ? Math.min(100, Math.floor((progress.done / progress.total) * 100)) : undefined;
  // 计算摘要和保存文件阶段的进度单位是字节
  const byBytes = progress?.stage === "hashing" || progress?.stage === "storing";
  const detail =
    progress && known
      ? byBytes
        ? `${formatBytes(progress.done)} / ${formatBytes(progress.total)}`
        : `${progress.done} / ${progress.total}`
      : progress && progress.done > 0
        ? `${progress.done}`
        : "";

  return (
    <Flex vertical gap={4} style={{ width: "100%" }}>
      <Flex justify="space-between" align="center">
        <Typography.Text>{stage}</Typography.Text>
        <Flex gap={8} align="center">
          <Typography.Text type="secondary">{detail}</Typography.Text>
          {onCancel && (
            <Button size="small" onClick={onCancel}>
              {t.common.stop}
            </Button>
          )}
        </Flex>
      </Flex>
      <Progress percent={percent ?? 100} status="active" showInfo={known ?? false} />
    </Flex>
  );
}
