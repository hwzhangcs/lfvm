import { FolderOpenOutlined } from "@ant-design/icons";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Alert, App, Button, Flex, Modal, Radio, Space, Spin, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import {
  api,
  errorMessage,
  type FileTargetArg,
  invalidateProject,
  newId,
  type OperationResult,
  type PickedFolder,
  type SourceRef,
} from "../api";
import { useTask } from "../hooks/useTask";
import { t } from "../locales/zh-CN";
import { formatBytes, formatTime } from "../utils/format";
import { OperationResultView } from "./OperationResultView";
import { TaskProgress } from "./TaskProgress";

interface Props {
  open: boolean;
  projectId: string;
  source: SourceRef;
  path: string;
  onClose: () => void;
}

/**
 * 恢复单个历史文件（SRS 3.3.2.4）：选择原位置或另存；目标已有文件时显示其路径和修改时间并要求确认；
 * 替换前先为原文件创建安全备份。来自版本、安全备份和搜索结果的恢复都用这个对话框。
 */
export function RestoreFileDialog({ open, projectId, source, path, onClose }: Props) {
  const { message } = App.useApp();
  const navigate = useNavigate();
  const qc = useQueryClient();
  const task = useTask();
  const [mode, setMode] = useState<"original" | "save_as">("original");
  const [dir, setDir] = useState<PickedFolder | null>(null);
  const [result, setResult] = useState<OperationResult | null>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: 每次打开对话框生成新的请求标识
  const requestId = useMemo(() => newId(), [open]);
  const name = path.split("/").pop() ?? path;

  useEffect(() => {
    if (open) {
      setMode("original");
      setDir(null);
      setResult(null);
    }
  }, [open]);

  const target: FileTargetArg | null =
    mode === "original" ? { kind: "original" } : dir ? { kind: "save_as", token: dir.token } : null;
  const check = useQuery({
    queryKey: ["restore-check", projectId, source, path, target],
    queryFn: () => api.checkFileRestore(projectId, source, path, target as FileTargetArg),
    enabled: open && target !== null && !result,
    gcTime: 0,
    staleTime: 0,
  });

  const pickDir = async () => {
    try {
      const f = await api.pickFolder("save_to");
      if (f) setDir(f);
    } catch (e) {
      message.error(errorMessage(e));
    }
  };

  const confirm = async () => {
    if (!target || !check.data) return;
    try {
      const r = await task.run((tid, ch) =>
        api.restoreFile(
          {
            project_id: projectId,
            request_id: requestId,
            source,
            path,
            confirm_token: check.data?.confirm_token ?? null,
          },
          target,
          tid,
          ch,
        ),
      );
      setResult(r);
      void invalidateProject(qc, projectId);
    } catch (e) {
      message.error(errorMessage(e));
      void check.refetch();
    }
  };

  const c = check.data;
  const blocked = !c || c.excluded;
  return (
    <Modal
      title={t.restoreFile.title(name)}
      open={open}
      width={600}
      onCancel={task.running ? undefined : onClose}
      closable={!task.running}
      maskClosable={false}
      destroyOnHidden
      footer={
        result ? (
          <Button type="primary" onClick={onClose}>
            {t.common.close}
          </Button>
        ) : task.running ? (
          <TaskProgress progress={task.progress} />
        ) : (
          <Space>
            <Button onClick={onClose}>{t.common.cancel}</Button>
            <Button type="primary" danger={c?.exists} disabled={blocked || check.isFetching} onClick={confirm}>
              {c?.exists ? t.restoreFile.confirmReplace : t.restoreFile.confirmCreate}
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
      ) : (
        <Flex vertical gap={16}>
          <Radio.Group value={mode} onChange={(e) => setMode(e.target.value)} disabled={task.running}>
            <Space direction="vertical">
              <Radio value="original">{t.restoreFile.original}</Radio>
              <Radio value="save_as">
                <Space>
                  {t.restoreFile.saveAs}
                  {mode === "save_as" && (
                    <Button size="small" icon={<FolderOpenOutlined />} onClick={pickDir}>
                      {t.restoreFile.pickDir}
                    </Button>
                  )}
                </Space>
              </Radio>
            </Space>
          </Radio.Group>

          {mode === "save_as" && !dir ? (
            <Typography.Text type="secondary">{t.restoreFile.needDir}</Typography.Text>
          ) : check.isFetching ? (
            <Spin />
          ) : check.error ? (
            <Alert type="error" showIcon title={errorMessage(check.error)} />
          ) : c ? (
            <Flex vertical gap={8}>
              <Typography.Text>
                {t.restoreFile.target}：<Typography.Text code>{c.target_path}</Typography.Text>
              </Typography.Text>
              {c.excluded && <Alert type="warning" showIcon title={t.restoreFile.excluded} />}
              {c.exists && !c.excluded && (
                <Alert
                  type="warning"
                  showIcon
                  title={t.restoreFile.exists}
                  description={t.restoreFile.existsDetail(
                    c.existing_modified_at ? formatTime(c.existing_modified_at) : "—",
                    c.existing_size !== null ? formatBytes(c.existing_size) : "—",
                  )}
                />
              )}
            </Flex>
          ) : null}
        </Flex>
      )}
    </Modal>
  );
}
