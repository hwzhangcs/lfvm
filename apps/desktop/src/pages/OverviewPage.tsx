import { FilterOutlined, FolderOpenOutlined, ReloadOutlined, SaveOutlined } from "@ant-design/icons";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import {
  Alert,
  App,
  Button,
  Card,
  Descriptions,
  Empty,
  Flex,
  Segmented,
  Space,
  Statistic,
  Table,
  type TableColumnsType,
  Tag,
  Typography,
} from "antd";
import { useState } from "react";
import {
  api,
  type ChangeItem,
  type ChangeKind,
  type ChangeSet,
  errorMessage,
  inDesktop,
  type ProgressEvent,
  queryKeys,
  withProgress,
} from "../api";
import { ExclusionsDrawer } from "../components/ExclusionsDrawer";
import { SaveVersionModal } from "../components/SaveVersionModal";
import { TaskProgress } from "../components/TaskProgress";
import { t } from "../locales/zh-CN";
import { formatBytes, formatTime, versionLabel } from "../utils/format";

const KIND_COLOR: Record<ChangeKind, string> = {
  added: "success",
  modified: "processing",
  deleted: "error",
  type_changed: "warning",
  excluded_now: "default",
};

/** 项目概览：当前活动方案、比较基准、当前变化，以及保存版本和排除项设置入口（表 8-3）。 */
export function OverviewPage() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const { message } = App.useApp();
  const queryClient = useQueryClient();
  const [scanProgress, setScanProgress] = useState<ProgressEvent | null>(null);
  const [filter, setFilter] = useState<ChangeKind | "all">("all");
  const [saving, setSaving] = useState(false);
  const [excluding, setExcluding] = useState(false);

  const overview = useQuery({
    queryKey: queryKeys.overview(projectId),
    queryFn: () => api.openProject(projectId),
    enabled: inDesktop,
  });
  const changes = useQuery({
    queryKey: queryKeys.changes(projectId),
    queryFn: ({ signal }) => {
      setScanProgress(null);
      return withProgress((task, ch) => api.currentChanges(projectId, task, ch), setScanProgress, { signal });
    },
    enabled: inDesktop,
  });

  const refresh = () => queryClient.invalidateQueries({ queryKey: queryKeys.changes(projectId) });
  const stopScan = () => queryClient.cancelQueries({ queryKey: queryKeys.changes(projectId) });

  const ov = overview.data;
  const cs = changes.data;

  return (
    <Flex vertical gap={16}>
      <Card title={t.nav.overview}>
        <Descriptions column={{ xs: 1, md: 2 }} size="small">
          <Descriptions.Item label={t.overview.route}>
            {ov?.active_scheme ? t.overview.scheme(ov.active_scheme.name) : t.overview.defaultRoute}
          </Descriptions.Item>
          <Descriptions.Item label={t.overview.baseline}>
            {ov?.baseline ? (
              <Space>
                <Tag color="blue">{versionLabel(ov.baseline.seq)}</Tag>
                {ov.baseline.name && <Typography.Text>{ov.baseline.name}</Typography.Text>}
                <Typography.Text type="secondary">{formatTime(ov.baseline.created_at)}</Typography.Text>
              </Space>
            ) : (
              <Typography.Text type="secondary">{t.overview.noVersion}</Typography.Text>
            )}
          </Descriptions.Item>
          <Descriptions.Item label={t.overview.versionCount}>{ov?.version_count ?? "—"}</Descriptions.Item>
          <Descriptions.Item label={t.overview.location}>
            <Space>
              <Typography.Text ellipsis={{ tooltip: ov?.project.root_path }} style={{ maxWidth: 360 }}>
                {ov?.project.root_path}
              </Typography.Text>
              <Button
                size="small"
                type="link"
                icon={<FolderOpenOutlined />}
                onClick={() => api.revealProjectFolder(projectId).catch((e) => message.error(errorMessage(e)))}
              >
                {t.overview.reveal}
              </Button>
            </Space>
          </Descriptions.Item>
        </Descriptions>
      </Card>

      <Card
        title={t.overview.changesTitle}
        extra={
          <Space wrap>
            <Button icon={<ReloadOutlined />} onClick={refresh} disabled={changes.isFetching}>
              {t.common.refresh}
            </Button>
            <Button icon={<FilterOutlined />} onClick={() => setExcluding(true)}>
              {t.overview.exclusions}
            </Button>
            <Button
              type="primary"
              icon={<SaveOutlined />}
              disabled={!cs || changes.isFetching || !cs.has_changes || cs.problems.length > 0}
              onClick={() => setSaving(true)}
            >
              {t.overview.save}
            </Button>
          </Space>
        }
      >
        {changes.isFetching ? (
          <TaskProgress progress={scanProgress} onCancel={stopScan} />
        ) : changes.error ? (
          <Alert type="error" showIcon title={errorMessage(changes.error)} />
        ) : cs ? (
          <ChangesBody cs={cs} filter={filter} onFilter={setFilter} />
        ) : null}
      </Card>

      {cs && (
        <SaveVersionModal
          open={saving}
          projectId={projectId}
          changes={cs}
          onClose={() => setSaving(false)}
          onDone={() => {
            setSaving(false);
            void queryClient.invalidateQueries({ queryKey: queryKeys.overview(projectId) });
            void queryClient.invalidateQueries({ queryKey: queryKeys.timeMap(projectId) });
            void refresh();
          }}
        />
      )}
      <ExclusionsDrawer
        open={excluding}
        projectId={projectId}
        onClose={() => setExcluding(false)}
        onSaved={() => {
          setExcluding(false);
          void refresh();
        }}
      />
    </Flex>
  );
}

function ChangesBody({
  cs,
  filter,
  onFilter,
}: {
  cs: ChangeSet;
  filter: ChangeKind | "all";
  onFilter: (f: ChangeKind | "all") => void;
}) {
  const c = cs.counts;
  const rows = filter === "all" ? cs.items : cs.items.filter((i) => i.kind === filter);
  const kinds = (Object.keys(t.changeKind) as ChangeKind[]).filter((k) => cs.items.some((i) => i.kind === k));

  return (
    <Flex vertical gap={16}>
      {cs.problems.length > 0 && (
        <Alert
          type="error"
          showIcon
          title={t.overview.problemsTitle(cs.problems.length)}
          description={
            <ul style={{ margin: 0, paddingInlineStart: 20 }}>
              {cs.problems.slice(0, 20).map((p) => (
                <li key={p.path}>
                  <Typography.Text code>{p.path}</Typography.Text>　{p.message}
                </li>
              ))}
            </ul>
          }
        />
      )}
      {cs.over_scale && <Alert type="info" showIcon title={t.overScale} />}
      {cs.rules_changed && <Alert type="info" showIcon title={t.overview.rulesChanged} />}
      {!cs.baseline_version_id && <Alert type="info" showIcon title={t.overview.firstHint} />}

      <Flex gap={32} wrap>
        <Statistic title={t.counts.added} value={c.added} styles={{ content: { color: "#52c41a" } }} />
        <Statistic title={t.counts.modified} value={c.modified} styles={{ content: { color: "#1677ff" } }} />
        <Statistic title={t.counts.deleted} value={c.deleted} styles={{ content: { color: "#ff4d4f" } }} />
        {c.excluded_now > 0 && <Statistic title={t.counts.excluded} value={c.excluded_now} />}
      </Flex>
      {(c.dirs_added > 0 || c.dirs_deleted > 0) && (
        <Typography.Text type="secondary">{t.counts.folders(c.dirs_added, c.dirs_deleted)}</Typography.Text>
      )}

      {cs.items.length === 0 ? (
        <Empty description={cs.has_changes ? t.overview.firstHint : t.overview.noChanges} />
      ) : (
        <>
          <Segmented
            value={filter}
            onChange={(v) => onFilter(v as ChangeKind | "all")}
            options={[
              { value: "all", label: `${t.overview.filterAll}（${cs.items.length}）` },
              ...kinds.map((k) => ({
                value: k,
                label: `${t.changeKind[k]}（${cs.items.filter((i) => i.kind === k).length}）`,
              })),
            ]}
          />
          <ChangeTable rows={rows} />
        </>
      )}
    </Flex>
  );
}

export function ChangeTable({ rows, height = 420 }: { rows: ChangeItem[]; height?: number }) {
  const columns: TableColumnsType<ChangeItem> = [
    {
      title: t.overview.path,
      dataIndex: "path",
      ellipsis: true,
      render: (p: string, r) => (
        <Typography.Text>
          {p}
          {r.entry_type === "directory" && (
            <Typography.Text type="secondary">　（{t.overview.folder}）</Typography.Text>
          )}
        </Typography.Text>
      ),
    },
    {
      title: t.overview.status,
      dataIndex: "kind",
      width: 150,
      render: (k: ChangeKind) => <Tag color={KIND_COLOR[k]}>{t.changeKind[k]}</Tag>,
    },
    {
      title: t.overview.size,
      dataIndex: "size",
      width: 110,
      align: "right",
      render: (s: number | null) => (s === null ? "" : <span title={`${s} 字节`}>{formatBytes(s)}</span>),
    },
  ];
  return (
    <Table
      rowKey="path"
      size="small"
      columns={columns}
      dataSource={rows}
      pagination={false}
      virtual
      scroll={{ y: height }}
    />
  );
}
