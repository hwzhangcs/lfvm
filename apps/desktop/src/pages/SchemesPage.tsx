import { BranchesOutlined, EditOutlined, PlusOutlined, SwapOutlined } from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import { Alert, Button, Card, Flex, Space, Table, type TableColumnsType, Tag, Typography } from "antd";
import { useState } from "react";
import { api, errorMessage, inDesktop, queryKeys, type SchemeInfo, type SwitchTarget } from "../api";
import { CreateSchemeModal, RenameSchemeModal, SwitchFlow } from "../features/schemes/SchemeDialogs";
import { laneColor } from "../features/timemap/layout";
import { t } from "../locales/zh-CN";
import { formatTime, versionLabel } from "../utils/format";

interface Row {
  key: string;
  scheme: SchemeInfo | null;
  name: string;
  active: boolean;
  base: string;
  head: string;
  versions: number | null;
  createdAt: number | null;
  color: string;
}

/** 方案管理（表 8-3）：方案列表；创建、改名、切换方案；切回默认历史。 */
export function SchemesPage() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const list = useQuery({
    queryKey: queryKeys.schemes(projectId),
    queryFn: () => api.listSchemes(projectId),
    enabled: inDesktop,
  });
  const map = useQuery({
    queryKey: queryKeys.timeMap(projectId),
    queryFn: () => api.timeMap(projectId),
    enabled: inDesktop,
  });
  const [creating, setCreating] = useState(false);
  const [renaming, setRenaming] = useState<SchemeInfo | null>(null);
  const [switching, setSwitching] = useState<{ target: SwitchTarget; label: string } | null>(null);

  const l = list.data;
  // 与时间地图使用相同的方案配色
  const colorOf = (schemeId: string) => {
    const i = map.data?.schemes.findIndex((s) => s.scheme_id === schemeId) ?? -1;
    return laneColor(i + 1);
  };
  const rows: Row[] = l
    ? [
        {
          key: "default",
          scheme: null,
          name: t.schemes.defaultHistory,
          active: l.default_active,
          base: "—",
          head: l.default_head ? `${versionLabel(l.default_head.seq)} ${l.default_head.name}` : "—",
          versions: null,
          createdAt: null,
          color: laneColor(0),
        },
        ...l.schemes.map((s) => ({
          key: s.scheme_id,
          scheme: s,
          name: s.name,
          active: s.active,
          base: versionLabel(s.base.seq),
          head: `${versionLabel(s.head.seq)} ${s.head.name}`,
          versions: s.version_count,
          createdAt: s.created_at,
          color: colorOf(s.scheme_id),
        })),
      ]
    : [];

  const columns: TableColumnsType<Row> = [
    {
      title: t.schemes.name,
      key: "name",
      render: (_, r) => (
        <Space>
          <BranchesOutlined style={{ color: r.color }} />
          <Typography.Text strong>{r.name}</Typography.Text>
          {r.active && <Tag color="blue">{t.schemes.current}</Tag>}
        </Space>
      ),
    },
    { title: t.schemes.base, dataIndex: "base", width: 90 },
    { title: t.schemes.head, dataIndex: "head", ellipsis: true },
    { title: t.schemes.versions, dataIndex: "versions", width: 80, render: (v: number | null) => v ?? "" },
    {
      title: t.schemes.created_at,
      dataIndex: "createdAt",
      width: 170,
      render: (v: number | null) => (v === null ? "" : formatTime(v)),
    },
    {
      title: "",
      key: "actions",
      width: 200,
      align: "right",
      render: (_, r) => (
        <Space size={0}>
          <Button
            type="link"
            icon={<SwapOutlined />}
            disabled={r.active || (!r.scheme && !l?.default_head)}
            onClick={() =>
              setSwitching({
                target: r.scheme ? { kind: "scheme", scheme_id: r.scheme.scheme_id } : { kind: "default" },
                label: r.scheme ? t.map.filterScheme(r.name) : t.schemes.defaultHistory,
              })
            }
          >
            {t.schemes.switch}
          </Button>
          {r.scheme && (
            <Button type="link" icon={<EditOutlined />} onClick={() => setRenaming(r.scheme)}>
              {t.schemes.rename}
            </Button>
          )}
        </Space>
      ),
    },
  ];

  return (
    <Card
      title={t.schemes.title}
      extra={
        <Button
          type="primary"
          icon={<PlusOutlined />}
          disabled={!l?.default_head && !l?.schemes.length}
          onClick={() => setCreating(true)}
        >
          {t.schemes.create}
        </Button>
      }
    >
      <Flex vertical gap={12}>
        <Alert type="info" showIcon title={t.schemes.desc} />
        {list.error ? (
          <Alert type="error" showIcon title={errorMessage(list.error)} />
        ) : (
          <Table
            rowKey="key"
            size="middle"
            loading={list.isLoading}
            columns={columns}
            dataSource={rows}
            pagination={false}
          />
        )}
      </Flex>
      <CreateSchemeModal
        open={creating}
        projectId={projectId}
        versionId={null}
        onClose={() => setCreating(false)}
        onSwitch={(s) =>
          setSwitching({ target: { kind: "scheme", scheme_id: s.scheme_id }, label: t.map.filterScheme(s.name) })
        }
      />
      <RenameSchemeModal scheme={renaming} projectId={projectId} onClose={() => setRenaming(null)} />
      <SwitchFlow
        projectId={projectId}
        target={switching?.target ?? null}
        label={switching?.label ?? ""}
        onClose={() => setSwitching(null)}
      />
    </Card>
  );
}
