import { HistoryOutlined } from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import {
  Alert,
  Button,
  Card,
  DatePicker,
  Descriptions,
  Drawer,
  Empty,
  Flex,
  Select,
  Space,
  Spin,
  Table,
  type TableColumnsType,
  Tabs,
  Tag,
} from "antd";
import type { Dayjs } from "dayjs";
import { useMemo, useState } from "react";
import {
  api,
  type BackupSummary,
  errorMessage,
  type HistoryEntry,
  type ItemState,
  inDesktop,
  type OperationSummary,
  type OpStatus,
  type OpType,
  queryKeys,
} from "../api";
import { RestoreFileDialog } from "../components/RestoreFileDialog";
import { HistoryBrowser } from "../features/history/HistoryBrowser";
import { t } from "../locales/zh-CN";
import { formatTime } from "../utils/format";

export const STATUS_COLOR: Record<OpStatus, string> = {
  running: "processing",
  succeeded: "success",
  cancelled: "default",
  failed: "error",
  incomplete: "warning",
};

function backupTag(b: BackupSummary | null) {
  if (!b) return <Tag>{t.backups.noBackup}</Tag>;
  if (b.status === "cleared") return <Tag>{t.backups.contentCleared}</Tag>;
  if (b.status === "failed" || b.status === "creating") return <Tag color="error">{t.backups.backupFailed}</Tag>;
  if (!b.recoverable) return <Tag color="error">{t.backups.unrecoverable}</Tag>;
  return <Tag color="green">{b.file_count}</Tag>;
}

/** 安全备份与操作记录（SRS 3.3.3.2、3.3.3.3，表 8-3“安全备份”）。 */
export function BackupsPage() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const { op } = useSearch({ from: "/projects/$projectId/backups" });
  const navigate = useNavigate();
  const [range, setRange] = useState<[Dayjs | null, Dayjs | null] | null>(null);
  const [type, setType] = useState<OpType | "all">("all");
  const ops = useQuery({
    queryKey: queryKeys.operations(projectId),
    queryFn: () => api.listOperations(projectId),
    enabled: inDesktop,
  });

  const rows = useMemo(() => {
    const from = range?.[0]?.startOf("day").valueOf();
    const to = range?.[1]?.endOf("day").valueOf();
    return (ops.data ?? []).filter(
      (o) =>
        (type === "all" || o.op_type === type) &&
        (from === undefined || o.created_at >= from) &&
        (to === undefined || o.created_at <= to),
    );
  }, [ops.data, range, type]);

  const select = (id: string | undefined) =>
    navigate({ to: "/projects/$projectId/backups", params: { projectId }, search: id ? { op: id } : {} });

  const columns: TableColumnsType<OperationSummary> = [
    { title: t.backups.time, dataIndex: "created_at", width: 170, render: (ms: number) => formatTime(ms) },
    {
      title: t.backups.operation,
      key: "op",
      render: (_, o) => (
        <Space>
          <Tag>{t.ops.types[o.op_type]}</Tag>
          {o.target_label}
          {o.retry_of && <Tag icon={<HistoryOutlined />}>{t.backups.retry}</Tag>}
        </Space>
      ),
    },
    { title: t.backups.reason, key: "reason", ellipsis: true, render: (_, o) => o.backup?.reason ?? "—" },
    { title: t.backups.files, key: "files", width: 110, render: (_, o) => backupTag(o.backup) },
    {
      title: t.backups.status,
      dataIndex: "status",
      width: 110,
      render: (s: OpStatus, o) => (
        <Space size={4}>
          <Tag color={STATUS_COLOR[s]}>{t.ops.status[s]}</Tag>
          {s === "incomplete" && o.resolved && <Tag>{t.ops.resolved}</Tag>}
        </Space>
      ),
    },
  ];

  return (
    <Card title={t.backups.title}>
      <Flex vertical gap={12}>
        <Alert type="info" showIcon title={t.backups.desc} />
        <Flex gap={12} wrap>
          <DatePicker.RangePicker value={range} onChange={setRange} allowEmpty={[true, true]} />
          <Select
            style={{ width: 160 }}
            value={type}
            onChange={setType}
            options={[
              { value: "all", label: t.overview.filterAll },
              { value: "restore", label: t.ops.types.restore },
              { value: "switch", label: t.ops.types.switch },
            ]}
          />
        </Flex>
        {ops.error ? (
          <Alert type="error" showIcon title={errorMessage(ops.error)} />
        ) : (
          <Table
            rowKey="operation_id"
            size="small"
            loading={ops.isLoading}
            columns={columns}
            dataSource={rows}
            pagination={{ pageSize: 20, hideOnSinglePage: true }}
            locale={{ emptyText: <Empty description={t.backups.empty} /> }}
            onRow={(o) => ({ onClick: () => select(o.operation_id), style: { cursor: "pointer" } })}
          />
        )}
      </Flex>
      <Drawer title={t.backups.detail} size={980} open={!!op} onClose={() => select(undefined)} destroyOnHidden>
        {op && <OperationDetailView projectId={projectId} operationId={op} />}
      </Drawer>
    </Card>
  );
}

const ITEM_COLOR: Record<ItemState, string> = {
  pending: "default",
  applying: "processing",
  done: "success",
  failed: "error",
  skipped: "default",
};

function OperationDetailView({ projectId, operationId }: { projectId: string; operationId: string }) {
  const detail = useQuery({
    queryKey: queryKeys.operation(projectId, operationId),
    queryFn: () => api.operationDetail(projectId, operationId),
    enabled: inDesktop,
  });
  const [restoring, setRestoring] = useState<HistoryEntry | null>(null);
  if (detail.error) return <Alert type="error" showIcon title={errorMessage(detail.error)} />;
  const d = detail.data;
  if (!d) return <Spin />;
  const s = d.summary;
  const b = s.backup;
  const items = d.items.filter((i) => i.action !== "keep");

  return (
    <Flex vertical gap={16}>
      <Descriptions column={2} size="small" bordered>
        <Descriptions.Item label={t.backups.operation}>
          {t.ops.types[s.op_type]} · {s.target_label}
        </Descriptions.Item>
        <Descriptions.Item label={t.backups.status}>
          <Tag color={STATUS_COLOR[s.status]}>{t.ops.status[s.status]}</Tag>
        </Descriptions.Item>
        <Descriptions.Item label={t.backups.time}>{formatTime(s.created_at)}</Descriptions.Item>
        <Descriptions.Item label={t.backups.reason}>{b?.reason ?? t.backups.noBackup}</Descriptions.Item>
        <Descriptions.Item label={t.save.note} span={2}>
          {s.message}
        </Descriptions.Item>
        {b?.external_root && (
          <Descriptions.Item label={t.backups.browse} span={2}>
            {t.backups.external(b.external_root)}
          </Descriptions.Item>
        )}
      </Descriptions>
      <Tabs
        items={[
          {
            key: "items",
            label: `${t.backups.items}（${items.length}）`,
            children: (
              <Table
                rowKey={(i) => `${i.action}:${i.path}`}
                size="small"
                dataSource={items}
                pagination={false}
                virtual
                scroll={{ y: 360 }}
                columns={[
                  { title: t.overview.path, dataIndex: "path", ellipsis: true },
                  {
                    title: t.overview.status,
                    dataIndex: "action",
                    width: 90,
                    render: (a: keyof typeof t.restoreVersion.actions) => t.restoreVersion.actions[a],
                  },
                  {
                    title: t.backups.status,
                    dataIndex: "state",
                    width: 90,
                    render: (st: ItemState) => <Tag color={ITEM_COLOR[st]}>{t.ops.itemState[st]}</Tag>,
                  },
                  { title: "", dataIndex: "error", ellipsis: true, render: (e: string | null) => e ?? "" },
                ]}
              />
            ),
          },
          ...(b && b.status === "ready" && b.recoverable
            ? [
                {
                  key: "backup",
                  label: `${t.backups.browse}（${b.file_count}）`,
                  children: (
                    <HistoryBrowser
                      projectId={projectId}
                      source={{ kind: "backup", backup_id: b.backup_id }}
                      actions={(entry) => (
                        <Button type="primary" onClick={() => setRestoring(entry)}>
                          {t.restoreFile.button}
                        </Button>
                      )}
                    />
                  ),
                },
              ]
            : []),
        ]}
      />
      {b && restoring && (
        <RestoreFileDialog
          open
          projectId={projectId}
          source={{ kind: "backup", backup_id: b.backup_id }}
          path={restoring.path}
          onClose={() => setRestoring(null)}
        />
      )}
    </Flex>
  );
}
