import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import type { Channel } from "@tauri-apps/api/core";
import {
  Alert,
  App,
  Button,
  Empty,
  Flex,
  Modal,
  Segmented,
  Space,
  Statistic,
  Table,
  type TableColumnsType,
  Tag,
  Typography,
} from "antd";
import { type ReactNode, useEffect, useMemo, useState } from "react";
import {
  type Action,
  errorMessage,
  type ImpactPlan,
  invalidateProject,
  newId,
  type OperationResult,
  type PlanItem,
  type ProgressEvent,
} from "../api";
import { useTask } from "../hooks/useTask";
import { t } from "../locales/zh-CN";
import { formatBytes } from "../utils/format";
import { OperationResultView } from "./OperationResultView";
import { TaskProgress } from "./TaskProgress";

const ACTION_COLOR: Record<Action, string> = {
  create: "success",
  replace: "processing",
  delete: "error",
  keep: "default",
};

interface Props {
  open: boolean;
  projectId: string;
  title: string;
  /** 计算影响清单（只读）。 */
  plan: (task: string, ch: Channel<ProgressEvent>) => Promise<ImpactPlan>;
  /** 用户确认后执行；fingerprint 用于核对确认后文件夹没有再变化。 */
  run: (fingerprint: string, requestId: string, task: string, ch: Channel<ProgressEvent>) => Promise<OperationResult>;
  onClose: () => void;
  confirmText?: string;
  /** 影响清单上方的附加说明。 */
  intro?: ReactNode;
}

/**
 * 会写入或删除文件的操作在执行前显示影响范围并要求确认；执行中显示进度、可取消；
 * 结果以四种状态之一显示（SRS 3.3.5.2 第 3、4 步）。
 */
export function ImpactDialog({ open, projectId, title, plan, run, onClose, confirmText, intro }: Props) {
  const { message } = App.useApp();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const planTask = useTask();
  const runTask = useTask();
  const [impact, setImpact] = useState<ImpactPlan | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [result, setResult] = useState<OperationResult | null>(null);
  const [filter, setFilter] = useState<Action | "all">("all");
  // biome-ignore lint/correctness/useExhaustiveDependencies: 每次打开对话框生成新的请求标识
  const requestId = useMemo(() => newId(), [open]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在打开时计算一次
  useEffect(() => {
    if (!open) return;
    setImpact(null);
    setError(null);
    setResult(null);
    setFilter("all");
    planTask
      .run(plan)
      .then(setImpact)
      .catch((e) => setError(errorMessage(e)));
  }, [open]);

  const confirm = async () => {
    if (!impact) return;
    try {
      const r = await runTask.run((task, ch) => run(impact.fingerprint, requestId, task, ch));
      setResult(r);
    } catch (e) {
      message.error(errorMessage(e));
    } finally {
      void invalidateProject(qc, projectId);
    }
  };

  const changes = impact?.items.filter((i) => i.action !== "keep").length ?? 0;
  const rows = useMemo(
    () => (impact ? (filter === "all" ? impact.items : impact.items.filter((i) => i.action === filter)) : []),
    [impact, filter],
  );
  const columns: TableColumnsType<PlanItem> = [
    {
      title: t.overview.path,
      dataIndex: "path",
      ellipsis: true,
      render: (p: string, r) => (
        <span>
          {p}
          {r.entry_type === "directory" && (
            <Typography.Text type="secondary">　（{t.overview.folder}）</Typography.Text>
          )}
        </span>
      ),
    },
    {
      title: t.overview.status,
      dataIndex: "action",
      width: 170,
      render: (a: Action, r) => (
        <Space size={4}>
          <Tag color={ACTION_COLOR[a]}>{t.restoreVersion.actions[a]}</Tag>
          {r.type_change && <Tag>{t.restoreVersion.typeChange}</Tag>}
        </Space>
      ),
    },
    {
      title: t.overview.size,
      key: "size",
      width: 90,
      align: "right",
      render: (_, r) => {
        const s = r.after_size ?? r.before_size;
        return s === null ? "" : formatBytes(s);
      },
    },
  ];

  const busy = planTask.running || runTask.running;
  const blocked = !impact || impact.conflicts.length > 0 || changes === 0;

  return (
    <Modal
      title={title}
      open={open}
      width={820}
      onCancel={busy ? undefined : onClose}
      closable={!busy}
      maskClosable={false}
      destroyOnHidden
      footer={
        result ? (
          <Button type="primary" onClick={onClose}>
            {t.common.close}
          </Button>
        ) : runTask.running ? (
          <TaskProgress progress={runTask.progress} onCancel={runTask.cancel} />
        ) : (
          <Space>
            <Button onClick={onClose} disabled={busy}>
              {t.common.cancel}
            </Button>
            <Button type="primary" danger disabled={blocked || busy} onClick={confirm}>
              {confirmText ?? t.restoreVersion.confirm}
            </Button>
          </Space>
        )
      }
    >
      {result ? (
        <OperationResultView
          result={result}
          onOpenBackup={() => {
            onClose();
            navigate({
              to: "/projects/$projectId/backups",
              params: { projectId },
              search: { op: result.operation_id },
            });
          }}
        />
      ) : planTask.running ? (
        <Flex vertical gap={8}>
          <Typography.Text type="secondary">{t.restoreVersion.planning}</Typography.Text>
          <TaskProgress progress={planTask.progress} onCancel={planTask.cancel} />
        </Flex>
      ) : error ? (
        <Alert type="error" showIcon title={error} />
      ) : impact ? (
        <Flex vertical gap={12}>
          {intro}
          {impact.conflicts.length > 0 && (
            <Alert
              type="error"
              showIcon
              title={t.restoreVersion.conflicts}
              description={
                <ul style={{ margin: 0, paddingInlineStart: 20 }}>
                  {impact.conflicts.slice(0, 20).map((c) => (
                    <li key={c.path}>
                      <Typography.Text code>{c.path}</Typography.Text>　{c.message}
                    </li>
                  ))}
                </ul>
              }
            />
          )}
          {changes === 0 && impact.conflicts.length === 0 ? (
            <Empty description={t.restoreVersion.nothing} />
          ) : (
            <>
              <Typography.Text>{t.restoreVersion.summary}</Typography.Text>
              <Flex gap={32} wrap>
                <Statistic title={t.restoreVersion.actions.create} value={impact.counts.create} />
                <Statistic title={t.restoreVersion.actions.replace} value={impact.counts.replace} />
                <Statistic title={t.restoreVersion.actions.delete} value={impact.counts.delete} />
                <Statistic title={t.restoreVersion.actions.keep} value={impact.counts.keep} />
              </Flex>
              <Alert
                type="info"
                showIcon
                title={t.restoreVersion.backupNote(impact.counts.backup_files)}
                description={t.restoreVersion.keepNote}
              />
              <Segmented
                value={filter}
                onChange={(v) => setFilter(v as Action | "all")}
                options={[
                  { value: "all", label: `${t.overview.filterAll}（${impact.items.length}）` },
                  ...(["create", "replace", "delete", "keep"] as Action[])
                    .filter((a) => impact.items.some((i) => i.action === a))
                    .map((a) => ({
                      value: a,
                      label: `${t.restoreVersion.actions[a]}（${impact.items.filter((i) => i.action === a).length}）`,
                    })),
                ]}
              />
              <Table
                rowKey={(r) => `${r.action}:${r.path}`}
                size="small"
                columns={columns}
                dataSource={rows}
                pagination={false}
                virtual
                scroll={{ y: 280 }}
              />
            </>
          )}
        </Flex>
      ) : null}
    </Modal>
  );
}
