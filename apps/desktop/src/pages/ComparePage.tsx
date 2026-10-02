import { ArrowLeftOutlined, SwapOutlined } from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
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
  Space,
  Spin,
  Table,
  type TableColumnsType,
  Tag,
  Typography,
} from "antd";
import { useMemo, useState } from "react";
import { api, type CompareItem, type CompareKind, errorMessage, inDesktop, queryKeys } from "../api";
import { DiffView } from "../components/DiffView";
import { t } from "../locales/zh-CN";
import { formatBytes, formatTime, versionLabel } from "../utils/format";

const KIND_COLOR: Record<CompareKind, string> = {
  added: "success",
  deleted: "error",
  modified: "processing",
  type_changed: "warning",
  excluded_in_b: "default",
};

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

  const back = (
    <Link to="/projects/$projectId/map" params={{ projectId }}>
      <Button icon={<ArrowLeftOutlined />}>{t.nav.timeMap}</Button>
    </Link>
  );

  if (result.error) {
    return (
      <Flex vertical gap={12}>
        {back}
        <Alert type="error" showIcon title={errorMessage(result.error)} />
      </Flex>
    );
  }
  const r = result.data;
  if (!r) return <Spin style={{ display: "block", marginTop: 80 }} />;

  const kinds = (Object.keys(t.compare.kinds) as CompareKind[]).filter((k) => r.items.some((i) => i.kind === k));
  const columns: TableColumnsType<CompareItem> = [
    { title: t.overview.path, dataIndex: "path", ellipsis: true },
    {
      title: t.overview.status,
      dataIndex: "kind",
      width: 120,
      render: (k: CompareKind) => <Tag color={KIND_COLOR[k]}>{t.compare.kinds[k]}</Tag>,
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
    <Flex vertical gap={12}>
      <Flex align="center" gap={12} wrap>
        {back}
        <Typography.Title level={4} style={{ margin: 0 }}>
          {t.compare.title}
        </Typography.Title>
        <Space wrap>
          <Tag color="blue">
            A：{versionLabel(r.a.seq)} {r.a.name} · {formatTime(r.a.created_at)}
          </Tag>
          →
          <Tag color="purple">
            B：{versionLabel(r.b.seq)} {r.b.name} · {formatTime(r.b.created_at)}
          </Tag>
          <Button icon={<SwapOutlined />} onClick={() => go({ a: b, b: a, scope: scope ?? undefined })}>
            {t.compare.swap}
          </Button>
        </Space>
      </Flex>

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
    </Flex>
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
