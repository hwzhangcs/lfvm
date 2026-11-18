import {
  FilterOutlined,
  FolderOpenOutlined,
  HistoryOutlined,
  NodeIndexOutlined,
  ReloadOutlined,
  SaveOutlined,
} from "@ant-design/icons";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import { Alert, App, Button, Empty, Flex, Space, Table, type TableColumnsType, Tag, Tooltip, Typography } from "antd";
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
import { KindLabel, Page, PageHeader, PathText } from "../components/ui";
import { t } from "../locales/zh-CN";
import { formatBytes, formatTime, versionLabel } from "../utils/format";

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

  const sep = <span style={{ color: "var(--ant-color-border)" }}>·</span>;
  return (
    <Page>
      <PageHeader
        title={t.nav.overview}
        sub={
          <>
            <span className="lf-meta">
              <NodeIndexOutlined />
              {ov?.active_scheme ? t.overview.scheme(ov.active_scheme.name) : t.overview.defaultRoute}
            </span>
            <span className="lf-meta">
              <HistoryOutlined />
              {ov?.baseline ? (
                <>
                  {t.overview.baseline}
                  <Tag color="processing" variant="filled" style={{ marginInlineEnd: 0 }}>
                    {versionLabel(ov.baseline.seq)}
                  </Tag>
                  {ov.baseline.name && (
                    <>
                      <Typography.Text>{ov.baseline.name}</Typography.Text>
                      {sep}
                    </>
                  )}
                  <span className="lf-num">{formatTime(ov.baseline.created_at)}</span>
                </>
              ) : (
                t.overview.noVersion
              )}
            </span>
            {ov && <span className="lf-meta">{t.overview.versionCountValue(ov.version_count)}</span>}
          </>
        }
        extra={
          <Tooltip title={ov?.project.root_path}>
            <Button
              icon={<FolderOpenOutlined />}
              onClick={() => api.revealProjectFolder(projectId).catch((e) => message.error(errorMessage(e)))}
            >
              {t.overview.reveal}
            </Button>
          </Tooltip>
        }
      />

      <Flex vertical gap={14}>
        <div className="lf-section-title">
          {t.overview.changesTitle}
          <Space wrap>
            <Tooltip title={t.common.refresh}>
              <Button
                icon={<ReloadOutlined />}
                onClick={refresh}
                disabled={changes.isFetching}
                aria-label={t.common.refresh}
              />
            </Tooltip>
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
        </div>
        {changes.isFetching ? (
          <div className="lf-panel" style={{ padding: 16 }}>
            <TaskProgress progress={scanProgress} onCancel={stopScan} />
          </div>
        ) : changes.error ? (
          <Alert type="error" showIcon title={errorMessage(changes.error)} />
        ) : cs ? (
          <ChangesBody cs={cs} filter={filter} onFilter={setFilter} />
        ) : null}
      </Flex>

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
    </Page>
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
  const count = (k: ChangeKind) => cs.items.filter((i) => i.kind === k).length;
  // 统计块同时是筛选：新增、修改、删除总是显示（数字为文件数），其余类型有才显示
  const tiles: Array<{ key: ChangeKind | "all"; label: string; value: number }> = [
    { key: "all", label: t.overview.filterAll, value: cs.items.length },
    { key: "added", label: t.counts.added, value: c.added },
    { key: "modified", label: t.counts.modified, value: c.modified },
    { key: "deleted", label: t.counts.deleted, value: c.deleted },
    ...(["type_changed", "excluded_now"] as const)
      .filter((k) => count(k) > 0)
      .map((k) => ({ key: k, label: t.changeKind[k], value: count(k) })),
  ];

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

      {cs.items.length === 0 ? (
        <div className="lf-panel" style={{ padding: "36px 0" }}>
          <Empty
            image={Empty.PRESENTED_IMAGE_SIMPLE}
            description={cs.has_changes ? t.overview.firstHint : t.overview.noChanges}
          />
        </div>
      ) : (
        <>
          <div className="lf-tiles" role="tablist">
            {tiles.map((tile) => (
              <button
                key={tile.key}
                type="button"
                role="tab"
                aria-selected={filter === tile.key}
                className={`lf-tile${filter === tile.key ? " active" : ""}`}
                onClick={() => onFilter(tile.key)}
              >
                <span className="lf-tile-label">
                  {tile.key === "all" ? tile.label : <KindLabel kind={tile.key}>{tile.label}</KindLabel>}
                </span>
                <div className="lf-tile-value">{tile.value}</div>
              </button>
            ))}
          </div>
          {(c.dirs_added > 0 || c.dirs_deleted > 0) && (
            <Typography.Text type="secondary">{t.counts.folders(c.dirs_added, c.dirs_deleted)}</Typography.Text>
          )}
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
        <PathText
          path={p}
          isDir={r.entry_type === "directory"}
          suffix={
            r.entry_type === "directory" && (
              <Typography.Text type="secondary">　（{t.overview.folder}）</Typography.Text>
            )
          }
        />
      ),
    },
    {
      title: t.overview.status,
      dataIndex: "kind",
      width: 150,
      render: (k: ChangeKind) => <KindLabel kind={k}>{t.changeKind[k]}</KindLabel>,
    },
    {
      title: t.overview.size,
      dataIndex: "size",
      width: 110,
      align: "right",
      render: (s: number | null) =>
        s === null ? (
          ""
        ) : (
          <span className="lf-num" title={`${s} 字节`}>
            {formatBytes(s)}
          </span>
        ),
    },
  ];
  return (
    <div className="lf-panel" style={{ overflow: "hidden" }}>
      <Table
        rowKey="path"
        size="small"
        columns={columns}
        dataSource={rows}
        pagination={false}
        virtual
        scroll={{ y: height }}
      />
    </div>
  );
}
