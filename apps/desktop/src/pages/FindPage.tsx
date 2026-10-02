import { EyeOutlined, FileImageOutlined, NodeIndexOutlined, RollbackOutlined, SearchOutlined } from "@ant-design/icons";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import {
  Alert,
  App,
  Button,
  Card,
  Drawer,
  Empty,
  Flex,
  Form,
  Input,
  Select,
  Space,
  Spin,
  Table,
  type TableColumnsType,
  Tag,
  Tooltip,
  Typography,
  theme,
} from "antd";
import { useState } from "react";
import {
  api,
  errorMessage,
  inDesktop,
  queryKeys,
  type SearchHit,
  type SearchQuery,
  type SwitchTarget,
  sourceKey,
  thumbUrl,
} from "../api";
import { FilePreviewView } from "../components/FilePreviewView";
import { RestoreFileDialog } from "../components/RestoreFileDialog";
import { TrailView } from "../features/find/TrailView";
import { t } from "../locales/zh-CN";
import { formatBytes, formatTime } from "../utils/format";

interface FormValues {
  name?: string;
  ext?: string;
  path?: string;
  scope: string;
}

/** 找回文件（SRS 3.3.3.4、3.3.3.5，表 8-3“找回文件”）。 */
export function FindPage() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const { message } = App.useApp();
  const [form] = Form.useForm<FormValues>();
  const [query, setQuery] = useState<SearchQuery | null>(null);
  const [previewing, setPreviewing] = useState<SearchHit | null>(null);
  const [tracing, setTracing] = useState<SearchHit | null>(null);
  const [restoring, setRestoring] = useState<SearchHit | null>(null);
  const schemes = useQuery({
    queryKey: queryKeys.schemes(projectId),
    queryFn: () => api.listSchemes(projectId),
    enabled: inDesktop,
  });

  // 查询键包含全部条件：修改条件重新搜索时，旧的结果不会覆盖新结果
  const result = useQuery({
    queryKey: queryKeys.search(projectId, query),
    queryFn: () => api.searchFiles(projectId, query as SearchQuery),
    enabled: inDesktop && query !== null,
    placeholderData: keepPreviousData,
  });

  const toScope = (v: string): SwitchTarget | null =>
    v === "all" ? null : v === "default" ? { kind: "default" } : { kind: "scheme", scheme_id: v };

  const submit = (v: FormValues) => {
    if (!v.name?.trim() && !v.ext?.trim() && !v.path?.trim()) {
      message.warning(t.find.needOne);
      return;
    }
    setQuery({
      name: v.name?.trim() || null,
      ext: v.ext?.trim() || null,
      path: v.path?.trim() || null,
      scope: toScope(v.scope),
      page: 0,
    });
  };

  const columns: TableColumnsType<SearchHit> = [
    {
      title: "",
      key: "thumb",
      width: 76,
      render: (_, h) => (h.is_image ? <Thumb projectId={projectId} hash={h.hash} /> : null),
    },
    {
      title: t.find.name,
      key: "name",
      render: (_, h) => (
        <Flex vertical>
          <Typography.Text strong>{h.name}</Typography.Text>
          <Typography.Text type="secondary" ellipsis={{ tooltip: h.path }}>
            {h.path}
          </Typography.Text>
        </Flex>
      ),
    },
    {
      title: t.find.source,
      key: "source",
      width: 200,
      ellipsis: true,
      render: (_, h) =>
        h.source.kind === "version" ? (
          <Tag color="blue">{h.source_label}</Tag>
        ) : (
          <Tooltip title={h.source_label}>
            <Tag color="orange">{t.find.backupSource}</Tag>
          </Tooltip>
        ),
    },
    { title: t.find.time, dataIndex: "created_at", width: 160, render: (ms: number) => formatTime(ms) },
    {
      title: t.overview.size,
      dataIndex: "size",
      width: 90,
      align: "right",
      render: (s: number | null) => (s === null ? "" : formatBytes(s)),
    },
    {
      title: "",
      key: "actions",
      width: 250,
      align: "right",
      render: (_, h) => (
        <Space size={0}>
          <Button type="link" icon={<EyeOutlined />} onClick={() => setPreviewing(h)}>
            {t.find.preview}
          </Button>
          <Button type="link" icon={<NodeIndexOutlined />} onClick={() => setTracing(h)}>
            {t.find.trail}
          </Button>
          <Button type="link" icon={<RollbackOutlined />} onClick={() => setRestoring(h)}>
            {t.restoreFile.button}
          </Button>
        </Space>
      ),
    },
  ];

  return (
    <Flex vertical gap={16}>
      <Card title={t.find.title}>
        <Typography.Paragraph type="secondary">{t.find.desc}</Typography.Paragraph>
        <Form form={form} layout="inline" initialValues={{ scope: "all" }} onFinish={submit} style={{ rowGap: 12 }}>
          <Form.Item name="name" label={t.find.name}>
            <Input allowClear placeholder={t.find.namePlaceholder} style={{ width: 200 }} />
          </Form.Item>
          <Form.Item name="ext" label={t.find.ext}>
            <Input allowClear placeholder={t.find.extPlaceholder} style={{ width: 110 }} />
          </Form.Item>
          <Form.Item name="path" label={t.find.path}>
            <Input allowClear placeholder={t.find.pathPlaceholder} style={{ width: 180 }} />
          </Form.Item>
          <Form.Item name="scope" label={t.find.scope}>
            <Select
              style={{ width: 170 }}
              options={[
                { value: "all", label: t.find.scopeAll },
                { value: "default", label: t.schemes.defaultHistory },
                ...(schemes.data?.schemes ?? []).map((s) => ({
                  value: s.scheme_id,
                  label: t.map.filterScheme(s.name),
                })),
              ]}
            />
          </Form.Item>
          <Form.Item>
            <Button type="primary" htmlType="submit" icon={<SearchOutlined />}>
              {t.find.search}
            </Button>
          </Form.Item>
        </Form>
      </Card>

      {query && (
        <Card size="small" title={result.data ? t.find.total(result.data.total) : undefined}>
          {result.error ? (
            <Alert type="error" showIcon title={errorMessage(result.error)} />
          ) : (
            <Table
              rowKey={(h) => `${sourceKey(h.source)}:${h.path}`}
              columns={columns}
              dataSource={result.data?.hits ?? []}
              loading={result.isFetching}
              locale={{ emptyText: <Empty description={t.find.noResult} /> }}
              pagination={{
                current: (query.page ?? 0) + 1,
                pageSize: result.data?.page_size ?? 50,
                total: result.data?.total ?? 0,
                showSizeChanger: false,
                hideOnSinglePage: true,
                onChange: (p) => setQuery({ ...query, page: p - 1 }),
              }}
            />
          )}
        </Card>
      )}

      <Drawer
        title={previewing?.path}
        size={760}
        open={!!previewing}
        onClose={() => setPreviewing(null)}
        destroyOnHidden
      >
        {previewing && <HitPreview projectId={projectId} hit={previewing} />}
      </Drawer>
      <Drawer
        title={tracing ? t.trail.title(tracing.name) : ""}
        size={760}
        open={!!tracing}
        onClose={() => setTracing(null)}
        destroyOnHidden
      >
        {tracing && <TrailView projectId={projectId} path={tracing.path} />}
      </Drawer>
      {restoring && (
        <RestoreFileDialog
          open
          projectId={projectId}
          source={restoring.source}
          path={restoring.path}
          onClose={() => setRestoring(null)}
        />
      )}
    </Flex>
  );
}

/** 缩略图：先请后端生成（可能失败并给出原因），成功后经预览协议加载。 */
function Thumb({ projectId, hash }: { projectId: string; hash: string }) {
  const { token } = theme.useToken();
  const ready = useQuery({
    queryKey: queryKeys.thumb(projectId, hash),
    queryFn: () => api.ensureThumbnail(projectId, hash).then(() => true),
    enabled: inDesktop,
    staleTime: Number.POSITIVE_INFINITY,
  });
  const box = {
    width: 64,
    height: 64,
    display: "flex",
    alignItems: "center",
    justifyContent: "center",
    background: token.colorFillTertiary,
    borderRadius: token.borderRadiusSM,
    overflow: "hidden",
  } as const;
  if (ready.isLoading)
    return (
      <div style={box}>
        <Spin size="small" />
      </div>
    );
  if (ready.error) {
    return (
      <Tooltip title={`${t.find.thumbFailed}：${errorMessage(ready.error)}`}>
        <div style={box}>
          <FileImageOutlined style={{ fontSize: 24, color: token.colorTextQuaternary }} />
        </div>
      </Tooltip>
    );
  }
  return (
    <div style={box}>
      <img src={thumbUrl(projectId, hash)} alt="" style={{ maxWidth: 64, maxHeight: 64 }} draggable={false} />
    </div>
  );
}

function HitPreview({ projectId, hit }: { projectId: string; hit: SearchHit }) {
  const preview = useQuery({
    queryKey: queryKeys.preview(projectId, sourceKey(hit.source), hit.path),
    queryFn: () => api.previewFile(projectId, hit.source, hit.path),
    enabled: inDesktop,
  });
  if (preview.error) return <Alert type="error" showIcon title={errorMessage(preview.error)} />;
  if (!preview.data) return <Spin />;
  return (
    <Flex vertical gap={12}>
      <Typography.Text type="secondary">
        {hit.source.kind === "version" ? hit.source_label : `${t.find.backupSource}：${hit.source_label}`} ·{" "}
        {formatTime(hit.created_at)}
      </Typography.Text>
      <FilePreviewView projectId={projectId} preview={preview.data} height={560} />
    </Flex>
  );
}
