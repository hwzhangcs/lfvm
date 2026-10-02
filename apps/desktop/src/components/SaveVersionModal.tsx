import { Alert, App, Button, Flex, Form, Input, Modal, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import {
  api,
  type ChangeSet,
  errorCode,
  errorMessage,
  newId,
  type ProgressEvent,
  type SaveResult,
  withProgress,
} from "../api";
import { t } from "../locales/zh-CN";
import { ChangeTable } from "../pages/OverviewPage";
import { versionLabel } from "../utils/format";
import { TaskProgress } from "./TaskProgress";

interface Props {
  open: boolean;
  projectId: string;
  changes: ChangeSet;
  onClose: () => void;
  onDone: (r: SaveResult | null) => void;
}

/**
 * 保存版本（SRS 3.3.1.3 第 2～6 步）：显示本次新增、修改、删除的数量和列表，填写名称和说明，确认后保存。
 * “取消”只放弃本次保存，磁盘上的修改保持不变。
 */
export function SaveVersionModal({ open, projectId, changes, onClose, onDone }: Props) {
  const { message } = App.useApp();
  const [form] = Form.useForm<{ name: string; note: string }>();
  const [progress, setProgress] = useState<ProgressEvent | null>(null);
  const [taskId, setTaskId] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // 每次打开对话框生成新的请求标识；重复点击“确认保存”不会重复保存（LFVM-AT-13）
  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在打开时重新生成
  const requestId = useMemo(() => newId(), [open]);

  useEffect(() => {
    if (open) {
      form.resetFields();
      setError(null);
      setProgress(null);
    }
  }, [open, form]);

  const c = changes.counts;
  const running = taskId !== null;

  const submit = async () => {
    const { name = "", note = "" } = await form.validateFields();
    const id = newId();
    setTaskId(id);
    setError(null);
    try {
      const r = await withProgress(
        (task, ch) => api.saveVersion({ project_id: projectId, request_id: requestId, name, note }, task, ch),
        setProgress,
        { taskId: id },
      );
      message.success(t.save.success(versionLabel(r.seq)));
      onDone(r);
    } catch (e) {
      const code = errorCode(e);
      if (code === "CANCELLED") {
        message.info(t.result.cancelled);
        onClose();
      } else if (code === "CHANGED_EXTERNALLY" || code === "NOTHING_TO_SAVE") {
        message.warning(errorMessage(e));
        onDone(null);
      } else {
        setError(errorMessage(e));
      }
    } finally {
      setTaskId(null);
    }
  };

  return (
    <Modal
      title={t.save.title}
      open={open}
      width={720}
      maskClosable={false}
      closable={!running}
      keyboard={!running}
      onCancel={onClose}
      destroyOnHidden
      footer={
        running ? (
          <TaskProgress progress={progress} onCancel={() => taskId && void api.cancelTask(taskId)} />
        ) : (
          <Flex justify="space-between" align="center">
            <Typography.Text type="secondary">{t.save.cancelHint}</Typography.Text>
            <Flex gap={8}>
              <Button onClick={onClose}>{t.common.cancel}</Button>
              <Button type="primary" onClick={submit}>
                {t.save.confirm}
              </Button>
            </Flex>
          </Flex>
        )
      }
    >
      <Flex vertical gap={12}>
        <Typography.Text>
          {t.save.summary}
          {t.counts.added} {c.added}，{t.counts.modified} {c.modified}，{t.counts.deleted} {c.deleted}
          {(c.dirs_added > 0 || c.dirs_deleted > 0) && `；${t.counts.folders(c.dirs_added, c.dirs_deleted)}`}
        </Typography.Text>
        {changes.rules_changed && <Alert type="info" showIcon title={t.overview.rulesChanged} />}
        {changes.items.length > 0 && <ChangeTable rows={changes.items} height={220} />}
        <Form form={form} layout="vertical" disabled={running} requiredMark={false}>
          <Form.Item name="name" label={t.save.name} rules={[{ max: 100 }]}>
            <Input placeholder={t.save.namePlaceholder} maxLength={100} showCount />
          </Form.Item>
          <Form.Item name="note" label={t.save.note} rules={[{ max: 500 }]} style={{ marginBottom: 0 }}>
            <Input.TextArea placeholder={t.save.notePlaceholder} maxLength={500} showCount rows={3} />
          </Form.Item>
        </Form>
        {error && <Alert type="error" showIcon title={`${t.result.failed}：${error}`} />}
      </Flex>
    </Modal>
  );
}
