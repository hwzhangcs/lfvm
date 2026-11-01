import { SwapOutlined } from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { useNavigate, useParams, useSearch } from "@tanstack/react-router";
import {
  Alert,
  Button,
  Card,
  Col,
  Empty,
  Flex,
  Result,
  Row,
  Segmented,
  Select,
  Skeleton,
  Spin,
  Table,
  type TableColumnsType,
  Tag,
  Typography,
} from "antd";
import { useMemo, useState } from "react";
import { api, type CompareItem, type CompareKind, errorMessage, inDesktop, queryKeys } from "../api";
import { DiffView } from "../components/DiffView";
import { BackLink, KindLabel, Page, PageHeader, PathText } from "../components/ui";
import { t } from "../locales/zh-CN";
import { formatBytes, formatTime, versionLabel } from "../utils/format";

/** 版本比较（SRS 3.3.2.2、表 8-3“版本比较”）。方向为 A→B。 */
export function ComparePage() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const search = useSearch({ from: "/projects/$projectId/compare" });
  const navigate = useNavigate();
  const { a, b } = search;
  const scope = search.scope ?? null;
  const [filter, setFilter] = useState<CompareKind | "all">("all");
  const [selected, setSelected] = useState<CompareItem | null>(null);

  const result = useQuery({
    queryKey: queryKeys.compare(projectId, a, b, scope),
    queryFn: () => api.compareVersions(projectId, a, b, scope),
    enabled: inDesktop && !!a && !!b,
  });
  // 可选的比较范围：两个版本中出现过的全部文件夹
  const dirs = useQuery({
    queryKey: ["compare-dirs", projectId, a, b],
    queryFn: async () => {
      const [fa, fb] = await Promise.all([
        api.sourceFiles(projectId, { kind: "version", version_id: a }),
        api.sourceFiles(projectId, { kind: "version", version_id: b }),
      ]);
      return [...new Set([...fa, ...fb].filter((e) => e.entry_type === "directory").map((e) => e.path))].sort();
    },
    enabled: inDesktop && !!a && !!b,
  });

  const rows = useMemo(
    () =>
      result.data ? (filter === "all" ? result.data.items : result.data.items.filter((i) => i.kind === filter)) : [],
    [result.data, filter],
  );

  const go = (next: { a: string; b: string; scope?: string }) => {
    setSelected(null);
    navigate({ to: "/projects/$projectId/compare", params: { projectId }, search: next });
  };

  if (result.error) {
    return (
      <Page>
        <BackLink projectId={projectId} />
        <Alert type="error" showIcon title={errorMessage(result.error)} />
      </Page>
    );
  }
  const r = result.data;
  if (!r) {
    return (
      <Page>
        <Skeleton active paragraph={{ rows: 8 }} />
      </Page>
    );
  }

  const kinds = (Object.keys(t.compare.kinds) as CompareKind[]).filter((k) => r.items.some((i) => i.kind === k));
  const columns: TableColumnsType<CompareItem> = [
    {
      title: t.overview.path,
      dataIndex: "path",
      ellipsis: true,
      render: (p: string, i) => <PathText path={p} isDir={i.entry_type === "directory"} />,
    },
    {
      title: t.overview.status,
      dataIndex: "kind",
      width: 120,
      render: (k: CompareKind) => <KindLabel kind={k}>{t.compare.kinds[k]}</KindLabel>,
    },
    {
      title: t.overview.size,
      key: "size",
      width: 90,
      align: "right",
      render: (_, i) => {
        const s = i.size_b ?? i.size_a;
        return s === null ? "" : formatBytes(s);
      },
    },
  ];

  return (
    <Page>
      <BackLink projectId={projectId} />
      <PageHeader
        title={t.compare.title}
        sub={
          <>
            <span className="lf-meta">
              <Tag color="processing" variant="filled" style={{ marginInlineEnd: 0 }}>
                A
              </Tag>
              {versionLabel(r.a.seq)} {r.a.name} · <span className="lf-num">{formatTime(r.a.created_at)}</span>
            </span>
            <span style={{ color: "var(--ant-color-text-tertiary)" }}>→</span>
            <span className="lf-meta">
              <Tag color="purple" variant="filled" style={{ marginInlineEnd: 0 }}>
                B
              </Tag>
              {versionLabel(r.b.seq)} {r.b.name} · <span className="lf-num">{formatTime(r.b.created_at)}</span>
            </span>
          </>
        }
        extra={
          <Button icon={<SwapOutlined />} onClick={() => go({ a: b, b: a, scope: scope ?? undefined })}>
            {t.compare.swap}
          </Button>
        }
      />

      <Card size="small">
        <Flex gap={12} wrap align="center">
          <Typography.Text>{t.compare.scope}</Typography.Text>
          <Select
            showSearch
            style={{ minWidth: 260 }}
            value={scope ?? ""}
            options={[
              { value: "", label: t.compare.scopeAll },
              ...(dirs.data ?? []).map((d) => ({ value: d, label: d })),
            ]}
            onChange={(v) => go({ a, b, scope: v || undefined })}
          />
          {r.items.length > 0 && (
            <Segmented
              value={filter}
              onChange={(v) => setFilter(v as CompareKind | "all")}
              options={[
                { value: "all", label: `${t.overview.filterAll}（${r.items.length}）` },
                ...kinds.map((k) => ({
                  value: k,
                  label: `${t.compare.kinds[k]}（${r.items.filter((i) => i.kind === k).length}）`,
                })),
              ]}
            />
          )}
        </Flex>
      </Card>

      {r.items.length === 0 ? (
        <Result status="success" title={t.compare.identical} />
      ) : (
        <Row gutter={12}>
          <Col xs={24} xl={10}>
            <Table
              rowKey="path"
              size="small"
              columns={columns}
              dataSource={rows}
              pagination={false}
              virtual
              scroll={{ y: 520 }}
              rowClassName={(i) => (i.path === selected?.path ? "ant-table-row-selected" : "")}
              onRow={(i) => ({ onClick: () => setSelected(i), style: { cursor: "pointer" } })}
            />
          </Col>
          <Col xs={24} xl={14}>
            <Card size="small" title={selected?.path}>
              {!selected ? (
                <Empty description={t.compare.selectFile} />
              ) : selected.entry_type === "directory" ? (
                <Empty description={t.compare.folderOnly} />
              ) : (
                <FileDiffPanel projectId={projectId} a={a} b={b} path={selected.path} />
              )}
            </Card>
          </Col>
        </Row>
      )}
    </Page>
  );
}

function FileDiffPanel({ projectId, a, b, path }: { projectId: string; a: string; b: string; path: string }) {
  const diff = useQuery({
    queryKey: queryKeys.diff(projectId, a, b, path),
    queryFn: () => api.diffFile(projectId, a, b, path),
    enabled: inDesktop,
  });
  if (diff.error) return <Alert type="error" showIcon title={errorMessage(diff.error)} />;
  if (!diff.data) return <Spin />;
  return <DiffView projectId={projectId} diff={diff.data} height={460} />;
}
