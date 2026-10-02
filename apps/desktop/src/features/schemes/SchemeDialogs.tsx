import { useQuery, useQueryClient } from "@tanstack/react-query";
import { Alert, App, Button, Flex, Form, Input, Modal, Select, Space, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import {
  api,
  errorMessage,
  type ImpactPlan,
  inDesktop,
  invalidateProject,
  newId,
  queryKeys,
  type SchemeInfo,
  type SwitchCheck,
  type SwitchTarget,
} from "../../api";
import { ImpactDialog } from "../../components/ImpactDialog";
import { TaskProgress } from "../../components/TaskProgress";
import { useTask } from "../../hooks/useTask";
import { t } from "../../locales/zh-CN";
import { versionLabel } from "../../utils/format";

/**
 * 创建方案（SRS 3.3.4.1）：输入名称后创建，随后询问“是否立即切换到该方案”。
 * `versionId` 为空时让用户选择起始版本。
 */
export function CreateSchemeModal({
  open,
  projectId,
  versionId,
  onClose,
  onSwitch,
}: {
  open: boolean;
  projectId: string;
  versionId: string | null;
  onClose: () => void;
  onSwitch: (s: SchemeInfo) => void;
}) {
  const { message, modal } = App.useApp();
  const qc = useQueryClient();
  const [form] = Form.useForm<{ name: string; version: string }>();
  const [busy, setBusy] = useState(false);
  const map = useQuery({
    queryKey: queryKeys.timeMap(projectId),
    queryFn: () => api.timeMap(projectId),
    enabled: inDesktop && open,
  });
  const versions = useMemo(
    () =>
      (map.data?.nodes ?? [])
        .filter((n) => !n.cleared)
        .slice()
        .reverse()
        .map((n) => ({ value: n.version_id, label: `${versionLabel(n.seq)} ${n.name}`.trim() })),
    [map.data],
  );
  const fixed = versionId ? map.data?.nodes.find((n) => n.version_id === versionId) : undefined;

  useEffect(() => {
    if (open) form.resetFields();
  }, [open, form]);

  const submit = async () => {
    const v = await form.validateFields();
    const from = versionId ?? v.version;
    setBusy(true);
    try {
      const s = await api.createScheme(projectId, from, v.name);
      message.success(t.schemes.created(s.name));
      void invalidateProject(qc, projectId);
      onClose();
      modal.confirm({
        title: t.schemes.switchNowTitle(s.name),
        content: t.schemes.switchNowDesc,
        okText: t.schemes.switchNow,
        cancelText: t.schemes.later,
        onOk: () => onSwitch(s),
      });
    } catch (e) {
      message.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={fixed ? t.schemes.createTitle(versionLabel(fixed.seq)) : t.schemes.createPickTitle}
      open={open}
      onCancel={onClose}
      onOk={submit}
      okText={t.common.ok}
      cancelText={t.common.cancel}
      confirmLoading={busy}
      destroyOnHidden
    >
      <Form form={form} layout="vertical" requiredMark={false} preserve={false}>
        {!versionId && (
          <Form.Item name="version" label={t.schemes.pickVersion} rules={[{ required: true }]}>
            <Select showSearch optionFilterProp="label" options={versions} loading={map.isLoading} />
          </Form.Item>
        )}
        <Form.Item
          name="name"
          label={t.schemes.name}
          extra={t.schemes.nameRule}
          rules={[{ required: true, whitespace: true, message: t.schemes.nameRule }, { max: 100 }]}
        >
          <Input placeholder={t.schemes.namePlaceholder} maxLength={100} showCount autoFocus />
        </Form.Item>
      </Form>
    </Modal>
  );
}

export function RenameSchemeModal({
  scheme,
  projectId,
  onClose,
}: {
  scheme: SchemeInfo | null;
  projectId: string;
  onClose: () => void;
}) {
  const { message } = App.useApp();
  const qc = useQueryClient();
  const [form] = Form.useForm<{ name: string }>();
  const [busy, setBusy] = useState(false);
  useEffect(() => {
    if (scheme) form.setFieldsValue({ name: scheme.name });
  }, [scheme, form]);

  const submit = async () => {
    if (!scheme) return;
    const { name } = await form.validateFields();
    setBusy(true);
    try {
      await api.renameScheme(projectId, scheme.scheme_id, name);
      message.success(t.schemes.renamed);
      void invalidateProject(qc, projectId);
      onClose();
    } catch (e) {
      message.error(errorMessage(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Modal
      title={t.schemes.renameTitle}
      open={!!scheme}
      onCancel={onClose}
      onOk={submit}
      confirmLoading={busy}
      okText={t.common.ok}
      cancelText={t.common.cancel}
      destroyOnHidden
    >
      <Form form={form} layout="vertical" requiredMark={false}>
        <Form.Item
          name="name"
          label={t.schemes.name}
          extra={t.schemes.nameRule}
          rules={[{ required: true, whitespace: true, message: t.schemes.nameRule }, { max: 100 }]}
        >
          <Input maxLength={100} showCount autoFocus />
        </Form.Item>
      </Form>
    </Modal>
  );
}

/**
 * 切换方案（SRS 3.3.4.2）：先检查未保存的变化——有则只提供“保存并继续”和“取消切换”；
 * 没有则显示影响清单，确认后先备份再写入。
 */
export function SwitchFlow({
  projectId,
  target,
  label,
  onClose,
}: {
  projectId: string;
  target: SwitchTarget | null;
  label: string;
  onClose: () => void;
}) {
  const { message } = App.useApp();
  const qc = useQueryClient();
  const check = useTask();
  const save = useTask();
  const [state, setState] = useState<SwitchCheck | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [form] = Form.useForm<{ name: string; note: string }>();
  // biome-ignore lint/correctness/useExhaustiveDependencies: 每次切换生成新的保存请求标识
  const saveRequestId = useMemo(() => newId(), [target, state?.unsaved]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在目标变化时检查
  useEffect(() => {
    if (!target) return;
    setState(null);
    setError(null);
    form.resetFields();
    check
      .run((task, ch) => api.checkSwitch(projectId, target, task, ch))
      .then(setState)
      .catch((e) => setError(errorMessage(e)));
  }, [target]);

  const saveAndContinue = async () => {
    if (!target) return;
    const { name = "", note = "" } = await form.validateFields();
    try {
      await save.run((task, ch) =>
        api.saveVersion({ project_id: projectId, request_id: saveRequestId, name, note }, task, ch),
      );
      void invalidateProject(qc, projectId);
      const again = await check.run((task, ch) => api.checkSwitch(projectId, target, task, ch));
      setState(again);
    } catch (e) {
      message.error(errorMessage(e));
    }
  };

  if (!target) return null;
  if (state && !state.unsaved && state.impact) {
    const impact: ImpactPlan = state.impact;
    return (
      <ImpactDialog
        open
        projectId={projectId}
        title={t.schemes.switchTitle(label)}
        confirmText={t.schemes.confirmSwitch}
        intro={<Alert type="info" showIcon title={t.schemes.switchIntro(label)} />}
        plan={() => Promise.resolve(impact)}
        run={(fingerprint, requestId, task, ch) =>
          api.switchScheme({ project_id: projectId, request_id: requestId, target, fingerprint }, task, ch)
        }
        onClose={onClose}
      />
    );
  }

  const busy = check.running || save.running;
  return (
    <Modal
      open
      title={t.schemes.switchTitle(label)}
      onCancel={busy ? undefined : onClose}
      closable={!busy}
      maskClosable={false}
      footer={
        busy ? (
          <TaskProgress progress={save.running ? save.progress : check.progress} />
        ) : state?.unsaved ? (
          <Space>
            <Button onClick={onClose}>{t.schemes.cancelSwitch}</Button>
            <Button type="primary" onClick={saveAndContinue}>
              {t.schemes.saveAndContinue}
            </Button>
          </Space>
        ) : (
          <Button onClick={onClose}>{t.common.close}</Button>
        )
      }
    >
      {error ? (
        <Alert type="error" showIcon title={error} />
      ) : check.running && !state ? (
        <Typography.Text type="secondary">{t.schemes.checking}</Typography.Text>
      ) : state?.unsaved ? (
        <Flex vertical gap={12}>
          <Alert type="warning" showIcon title={t.schemes.unsavedTitle} description={t.schemes.unsavedDesc} />
          <Form form={form} layout="vertical" disabled={busy} requiredMark={false}>
            <Form.Item name="name" label={t.save.name} rules={[{ max: 100 }]}>
              <Input placeholder={t.save.namePlaceholder} maxLength={100} showCount />
            </Form.Item>
            <Form.Item name="note" label={t.save.note} rules={[{ max: 500 }]} style={{ marginBottom: 0 }}>
              <Input.TextArea placeholder={t.save.notePlaceholder} maxLength={500} showCount rows={2} />
            </Form.Item>
          </Form>
        </Flex>
      ) : null}
    </Modal>
  );
}
