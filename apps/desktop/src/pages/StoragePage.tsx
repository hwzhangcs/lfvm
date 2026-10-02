import { CheckCircleTwoTone, CloseCircleTwoTone, DeleteOutlined } from "@ant-design/icons";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import {
  Alert,
  App,
  Button,
  Card,
  Flex,
  Modal,
  Space,
  Spin,
  Statistic,
  Table,
  type TableColumnsType,
  Tabs,
  Tag,
  Tooltip,
  Typography,
} from "antd";
import { useState } from "react";
import {
  api,
  type BackupUsage,
  type ClearResult,
  errorMessage,
  inDesktop,
  invalidateProject,
  newId,
  queryKeys,
  type SchemeUsage,
  type VersionUsage,
} from "../api";
import { t } from "../locales/zh-CN";
import { formatBytes, formatTime, versionLabel } from "../utils/format";

function bytes(n: number) {
  return <span title={`${n} 字节`}>{formatBytes(n)}</span>;
}

function protectTag(reason: string | null) {
  return reason ? (
    <Tooltip title={reason}>
      <Tag>{reason}</Tag>
    </Tooltip>
  ) : (
    <Tag color="green">{t.storage.clearable}</Tag>
  );
}

/** 存储管理（SRS 3.3.5.1、表 8-3）：分类占用统计；可清理项与受保护原因；清理确认和实际释放量。 */
export function StoragePage() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const { modal, message } = App.useApp();
  const qc = useQueryClient();
  const report = useQuery({
    queryKey: queryKeys.storage(projectId),
    queryFn: () => api.storageReport(projectId),
    enabled: inDesktop,
  });
  const info = useQuery({ queryKey: queryKeys.appInfo, queryFn: api.appInfo, enabled: inDesktop });
  const [versions, setVersions] = useState<string[]>([]);
  const [backups, setBackups] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [result, setResult] = useState<ClearResult | null>(null);

  const clear = (req: { versions: string[]; backups: string[]; schemes: string[] }, title: string, extra?: string) => {
    const n = req.versions.length + req.backups.length + req.schemes.length;
    if (n === 0) return;
    modal.confirm({
      title,
      content: (
        <Flex vertical gap={8}>
          {extra && <Typography.Text>{extra}</Typography.Text>}
          <Typography.Text type="danger">{t.storage.confirmDesc}</Typography.Text>
        </Flex>
      ),
      okText: t.storage.clear,
      okButtonProps: { danger: true },
      cancelText: t.common.cancel,
      onOk: async () => {
        setBusy(true);
        try {
          const r = await api.clearStorage(projectId, { request_id: newId(), ...req });
          setResult(r);
          setVersions([]);
          setBackups([]);
        } catch (e) {
          message.error(errorMessage(e));
        } finally {
          setBusy(false);
          void invalidateProject(qc, projectId);
        }
      },
    });
  };

  if (report.error) return <Alert type="error" showIcon title={errorMessage(report.error)} />;
  const r = report.data;
  if (!r) return <Spin style={{ display: "block", marginTop: 80 }} />;
  const u = r.usage;

  const versionCols: TableColumnsType<VersionUsage> = [
    {
      title: t.storage.version,
      key: "v",
      render: (_, v) => (
        <Space>
          <Typography.Text strong>{versionLabel(v.version.seq)}</Typography.Text>
          {v.version.name}
          <Typography.Text type="secondary">{formatTime(v.version.created_at)}</Typography.Text>
        </Space>
      ),
    },
    {
      title: t.storage.scheme,
      dataIndex: "scheme_name",
      width: 140,
      render: (n: string | null) => n ?? t.schemes.defaultHistory,
    },
    { title: t.storage.size, dataIndex: "total_bytes", width: 110, align: "right", render: bytes },
    {
      title: <Tooltip title={t.storage.exclusiveTip}>{t.storage.exclusive}</Tooltip>,
      dataIndex: "exclusive_bytes",
      width: 110,
      align: "right",
      render: bytes,
    },
    { title: t.storage.status, dataIndex: "protected", width: 220, ellipsis: true, render: protectTag },
  ];
  const backupCols: TableColumnsType<BackupUsage> = [
    { title: t.backups.time, dataIndex: "created_at", width: 170, render: (ms: number) => formatTime(ms) },
    { title: t.backups.reason, dataIndex: "reason", ellipsis: true },
    { title: t.backups.files, dataIndex: "file_count", width: 90 },
    { title: t.storage.size, dataIndex: "total_bytes", width: 110, align: "right", render: bytes },
    { title: t.storage.exclusive, dataIndex: "exclusive_bytes", width: 110, align: "right", render: bytes },
    { title: t.storage.status, dataIndex: "protected", width: 200, ellipsis: true, render: protectTag },
  ];
  const schemeCols: TableColumnsType<SchemeUsage> = [
    { title: t.schemes.name, dataIndex: "name" },
    { title: t.storage.status, dataIndex: "protected", ellipsis: true, render: protectTag },
    {
      title: "",
      key: "a",
      width: 140,
      align: "right",
      render: (_, s) => (
        <Button
          danger
          type="link"
          icon={<DeleteOutlined />}
          disabled={!!s.protected || busy}
          onClick={() =>
            clear(
              { versions: [], backups: [], schemes: [s.scheme_id] },
              `${t.storage.clearScheme}：${s.name}`,
              t.storage.clearSchemeDesc,
            )
          }
        >
          {t.storage.clearScheme}
        </Button>
      ),
    },
  ];

  const selectable = r.versions.filter((v) => !v.protected).length + r.backups.filter((b) => !b.protected).length;

  return (
    <Flex vertical gap={16}>
      <Card title={t.storage.title}>
        <Flex vertical gap={16}>
          <Typography.Paragraph type="secondary" style={{ margin: 0 }}>
            {t.storage.desc}
          </Typography.Paragraph>
          <Flex gap={40} wrap>
            <Statistic title={t.storage.total} value={formatBytes(u.total_bytes)} />
            <Statistic title={t.storage.versions} value={formatBytes(u.version_bytes)} />
            <Statistic title={t.storage.backups} value={formatBytes(u.backup_bytes)} />
            <Statistic title={t.storage.metadata} value={formatBytes(u.metadata_bytes)} />
            <Statistic title={t.storage.cache} value={formatBytes(u.cache_bytes)} />
            {u.pending_gc_bytes > 0 && <Statistic title={t.storage.pending} value={formatBytes(u.pending_gc_bytes)} />}
          </Flex>
          <Typography.Text type="secondary">
            {t.storage.sharedNote}
            {info.data && `　${t.storage.location(info.data.data_dir)}`}
          </Typography.Text>
        </Flex>
      </Card>

      <Card
        extra={
          <Button
            danger
            type="primary"
            icon={<DeleteOutlined />}
            loading={busy}
            disabled={versions.length + backups.length === 0}
            onClick={() =>
              clear({ versions, backups, schemes: [] }, t.storage.confirmTitle(versions.length + backups.length))
            }
          >
            {t.storage.clear}（{versions.length + backups.length}）
          </Button>
        }
      >
        {selectable === 0 && r.schemes.every((s) => s.protected) && (
          <Alert type="info" showIcon title={t.storage.nothing} style={{ marginBottom: 12 }} />
        )}
        <Tabs
          items={[
            {
              key: "v",
              label: `${t.storage.tabVersions}（${r.versions.length}）`,
              children: (
                <Table
                  rowKey={(v) => v.version.version_id}
                  size="small"
                  columns={versionCols}
                  dataSource={r.versions}
                  pagination={{ pageSize: 50, hideOnSinglePage: true }}
                  rowSelection={{
                    selectedRowKeys: versions,
                    onChange: (k) => setVersions(k as string[]),
                    getCheckboxProps: (v) => ({ disabled: !!v.protected }),
                  }}
                />
              ),
            },
            {
              key: "b",
              label: `${t.storage.tabBackups}（${r.backups.length}）`,
              children: (
                <Table
                  rowKey="backup_id"
                  size="small"
                  columns={backupCols}
                  dataSource={r.backups}
                  pagination={{ pageSize: 50, hideOnSinglePage: true }}
                  rowSelection={{
                    selectedRowKeys: backups,
                    onChange: (k) => setBackups(k as string[]),
                    getCheckboxProps: (b) => ({ disabled: !!b.protected }),
                  }}
                />
              ),
            },
            {
              key: "s",
              label: `${t.storage.tabSchemes}（${r.schemes.length}）`,
              children: (
                <Table rowKey="scheme_id" size="small" columns={schemeCols} dataSource={r.schemes} pagination={false} />
              ),
            },
          ]}
        />
      </Card>

      <Modal
        open={!!result}
        title={t.storage.resultTitle}
        onCancel={() => setResult(null)}
        footer={
          <Button type="primary" onClick={() => setResult(null)}>
            {t.common.close}
          </Button>
        }
      >
        {result && (
          <Flex vertical gap={12}>
            <Typography.Title level={4} style={{ margin: 0 }}>
              {t.storage.freed(formatBytes(result.freed_bytes))}
            </Typography.Title>
            {result.pending_gc_bytes > 0 && (
              <Alert type="warning" showIcon title={t.storage.pendingNote(formatBytes(result.pending_gc_bytes))} />
            )}
            <Flex vertical gap={6}>
              {result.items.map((i) => (
                <Space key={`${i.label}-${i.message}`}>
                  {i.ok ? <CheckCircleTwoTone twoToneColor="#52c41a" /> : <CloseCircleTwoTone twoToneColor="#ff4d4f" />}
                  <Typography.Text strong>{i.label}</Typography.Text>
                  <Typography.Text type="secondary">{i.message}</Typography.Text>
                </Space>
              ))}
            </Flex>
          </Flex>
        )}
      </Modal>
    </Flex>
  );
}
