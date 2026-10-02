import { FolderOpenOutlined } from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { Alert, App, Button, Flex, Form, Input, Modal, Result, Select, Space, Tag, Typography } from "antd";
import { useEffect, useMemo, useState } from "react";
import {
  api,
  errorMessage,
  inDesktop,
  newId,
  type OutputKind,
  type OutputResult,
  type OutputSource,
  type PickedFolder,
  queryKeys,
} from "../../api";
import { TaskProgress } from "../../components/TaskProgress";
import { useTask } from "../../hooks/useTask";
import { t } from "../../locales/zh-CN";
import { STATUS_COLOR } from "../../pages/BackupsPage";

/** 要输出的内容：一个版本或方案；或“并排展开两个方案”。 */
export type OutputTarget = { mode: "single"; kind: OutputKind; source: OutputSource; label: string } | { mode: "dual" };

/**
 * 展开 / 导出（SRS 3.3.4.3、3.3.4.4）：选择输出位置和文件夹名称，输出后逐项校验；
 * 并排展开时两个方案分别输出、分别报告，允许一个成功一个失败（“部分完成”）。
 */
export function OutputDialog({
  projectId,
  target,
  onClose,
}: {
  projectId: string;
  target: OutputTarget | null;
  onClose: () => void;
}) {
  const { message } = App.useApp();
  const task = useTask();
  const [form] = Form.useForm<{ name: string; nameB?: string; schemeA?: string; schemeB?: string }>();
  const [location, setLocation] = useState<PickedFolder | null>(null);
  const [results, setResults] = useState<OutputResult[] | null>(null);
  const schemes = useQuery({
    queryKey: queryKeys.schemes(projectId),
    queryFn: () => api.listSchemes(projectId),
    enabled: inDesktop && target?.mode === "dual",
  });
  const kind: OutputKind = target?.mode === "single" ? target.kind : "expand";
  const kindLabel = kind === "expand" ? t.output.expand : t.output.export;
  // biome-ignore lint/correctness/useExhaustiveDependencies: 每次打开生成新的请求标识
  const requestIds = useMemo(() => [newId(), newId()], [target]);

  // biome-ignore lint/correctness/useExhaustiveDependencies: 只在目标变化时初始化
  useEffect(() => {
    if (!target) return;
    setLocation(null);
    setResults(null);
    form.resetFields();
    if (target.mode === "single") {
      api
        .suggestOutputName(projectId, target.source)
        .then((n) => form.setFieldValue("name", kind === "expand" ? `${n}${t.output.expandSuffix}` : n))
        .catch(() => {});
    }
  }, [target]);

  const suggestFor = (field: "name" | "nameB", schemeId: string) =>
    api
      .suggestOutputName(projectId, { kind: "scheme", scheme_id: schemeId })
      .then((n) => form.setFieldValue(field, n))
      .catch(() => {});

  const pick = async () => {
    try {
      const f = await api.pickFolder(kind === "expand" ? "expand_to" : "export_to");
      if (f) setLocation(f);
    } catch (e) {
      message.error(errorMessage(e));
    }
  };

  const run = async () => {
    if (!target || !location) {
      message.warning(t.output.needLocation);
      return;
    }
    const v = await form.validateFields();
    const jobs: { source: OutputSource; name: string }[] =
      target.mode === "single"
        ? [{ source: target.source, name: v.name }]
        : [
            { source: { kind: "scheme", scheme_id: v.schemeA as string }, name: v.name },
            { source: { kind: "scheme", scheme_id: v.schemeB as string }, name: v.nameB as string },
          ];
    if (target.mode === "dual") {
      if (v.schemeA === v.schemeB) return message.warning(t.output.sameScheme);
      if (v.name.trim() === v.nameB?.trim()) return message.warning(t.output.sameName);
    }
    const out: OutputResult[] = [];
    for (const [i, job] of jobs.entries()) {
      try {
        out.push(
          await task.run((tid, ch) =>
            api.outputVersion(
              projectId,
              requestIds[i] as string,
              kind,
              job.source,
              location.token,
              job.name.trim(),
              tid,
              ch,
            ),
          ),
        );
      } catch (e) {
        // 写入前就被拒绝（如目标非空、与项目重叠）：作为失败结果报告，另一个照常进行
        out.push({
          operation_id: "",
          status: "failed",
          message: errorMessage(e),
          path: job.name,
          source_label: target.mode === "single" ? target.label : job.name,
          files_written: 0,
          total_files: 0,
          failed: [],
        });
      }
    }
    setResults(out);
  };

  if (!target) return null;
  const title =
    target.mode === "dual"
      ? t.output.dualTitle
      : kind === "expand"
        ? t.output.expandTitle(target.label)
        : t.output.exportTitle(target.label);
  const schemeOptions = (schemes.data?.schemes ?? []).map((s) => ({ value: s.scheme_id, label: s.name }));
  const ok = results?.filter((r) => r.status === "succeeded").length ?? 0;

  return (
    <Modal
      open
      title={title}
      width={640}
      onCancel={task.running ? undefined : onClose}
      closable={!task.running}
      maskClosable={false}
      destroyOnHidden
      footer={
        results ? (
          <Button type="primary" onClick={onClose}>
            {t.common.close}
          </Button>
        ) : task.running ? (
          <TaskProgress progress={task.progress} onCancel={task.cancel} />
        ) : (
          <Space>
            <Button onClick={onClose}>{t.common.cancel}</Button>
            <Button type="primary" onClick={run} disabled={!location}>
              {t.output.start(kindLabel)}
            </Button>
          </Space>
        )
      }
    >
      {results ? (
        <Flex vertical gap={12}>
          {results.length > 1 && (
            <Result
              status={ok === results.length ? "success" : ok === 0 ? "error" : "warning"}
              title={ok === results.length ? t.result.succeeded : ok === 0 ? t.result.failed : t.output.partial}
            />
          )}
          {results.map((r) => (
            <Alert
              key={`${r.operation_id}-${r.path}`}
              type={r.status === "succeeded" ? "success" : r.status === "cancelled" ? "info" : "error"}
              showIcon
              title={
                <Space wrap>
                  <Tag color={STATUS_COLOR[r.status]}>{t.ops.status[r.status]}</Tag>
                  {r.source_label}
                </Space>
              }
              description={
                <Flex vertical gap={4}>
                  <Typography.Text>{r.message}</Typography.Text>
                  <Typography.Text type="secondary" copyable>
                    {r.path}
                  </Typography.Text>
                  {r.total_files > 0 && (
                    <Typography.Text type="secondary">
                      {t.output.written(r.files_written, r.total_files)}
                    </Typography.Text>
                  )}
                </Flex>
              }
              action={
                r.status === "succeeded" && (
                  <Button
                    size="small"
                    icon={<FolderOpenOutlined />}
                    onClick={() =>
                      api.revealOutput(projectId, r.operation_id).catch((e) => message.error(errorMessage(e)))
                    }
                  >
                    {t.output.openFolder}
                  </Button>
                )
              }
            />
          ))}
        </Flex>
      ) : (
        <Flex vertical gap={12}>
          <Typography.Paragraph type="secondary" style={{ margin: 0 }}>
            {target.mode === "dual" ? t.output.dualDesc : kind === "expand" ? t.output.expandDesc : t.output.exportDesc}
          </Typography.Paragraph>
          <Flex gap={8} align="center" wrap>
            <Typography.Text>{t.output.location}：</Typography.Text>
            <Button icon={<FolderOpenOutlined />} onClick={pick} disabled={task.running}>
              {t.output.pickLocation}
            </Button>
            {location && <Typography.Text code>{location.path}</Typography.Text>}
          </Flex>
          <Form form={form} layout="vertical" requiredMark={false} disabled={task.running}>
            {target.mode === "dual" && (
              <Form.Item name="schemeA" label={t.output.schemeA} rules={[{ required: true }]}>
                <Select options={schemeOptions} onChange={(v: string) => suggestFor("name", v)} />
              </Form.Item>
            )}
            <Form.Item
              name="name"
              label={t.output.folderName}
              extra={t.output.folderRule}
              rules={[{ required: true, whitespace: true }, { max: 200 }]}
            >
              <Input />
            </Form.Item>
            {target.mode === "dual" && (
              <>
                <Form.Item name="schemeB" label={t.output.schemeB} rules={[{ required: true }]}>
                  <Select options={schemeOptions} onChange={(v: string) => suggestFor("nameB", v)} />
                </Form.Item>
                <Form.Item
                  name="nameB"
                  label={t.output.folderName}
                  rules={[{ required: true, whitespace: true }, { max: 200 }]}
                >
                  <Input />
                </Form.Item>
              </>
            )}
          </Form>
        </Flex>
      )}
    </Modal>
  );
}
