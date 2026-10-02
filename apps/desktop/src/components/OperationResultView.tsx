import { Button, Collapse, Result, Typography } from "antd";
import type { OperationResult, OpStatus } from "../api";
import { t } from "../locales/zh-CN";

const RESULT_STATUS: Record<OpStatus, "success" | "info" | "error" | "warning"> = {
  running: "info",
  succeeded: "success",
  cancelled: "info",
  failed: "error",
  incomplete: "warning",
};

/**
 * 操作结果统一以“成功”“已取消”“失败”“未完成”显示，并给出原因和下一步可执行的操作（SRS 3.3.5.2 第 4 步）。
 */
export function OperationResultView({
  result,
  onOpenBackup,
  extra,
}: {
  result: OperationResult;
  onOpenBackup?: () => void;
  extra?: React.ReactNode;
}) {
  const list = (paths: string[]) => (
    <ul style={{ margin: 0, paddingInlineStart: 20, maxHeight: 160, overflow: "auto" }}>
      {paths.map((p) => (
        <li key={p}>
          <Typography.Text code>{p}</Typography.Text>
        </li>
      ))}
    </ul>
  );
  const sections = [
    result.failed.length > 0 && {
      key: "failed",
      label: t.ops.failedItems(result.failed.length),
      children: (
        <ul style={{ margin: 0, paddingInlineStart: 20 }}>
          {result.failed.map((f) => (
            <li key={f.path}>
              <Typography.Text code>{f.path}</Typography.Text>　{f.message}
            </li>
          ))}
        </ul>
      ),
    },
    result.status !== "succeeded" &&
      result.completed.length > 0 && {
        key: "done",
        label: t.ops.completed(result.completed.length),
        children: list(result.completed),
      },
    result.status !== "succeeded" &&
      result.pending.length > 0 && {
        key: "pending",
        label: t.ops.pendingItems(result.pending.length),
        children: list(result.pending),
      },
  ].filter(Boolean) as { key: string; label: string; children: React.ReactNode }[];

  return (
    <Result
      status={RESULT_STATUS[result.status]}
      title={t.ops.status[result.status]}
      subTitle={result.message}
      extra={[
        result.backup_id && onOpenBackup && (
          <Button key="backup" onClick={onOpenBackup}>
            {t.ops.openBackup}
          </Button>
        ),
        extra,
      ]}
    >
      {sections.length > 0 && <Collapse size="small" items={sections} defaultActiveKey={["failed"]} />}
    </Result>
  );
}
