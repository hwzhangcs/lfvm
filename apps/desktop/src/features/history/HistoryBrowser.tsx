import { FileOutlined, FolderOutlined, WarningOutlined } from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import {
  Alert,
  Card,
  Col,
  Descriptions,
  Empty,
  Flex,
  Input,
  Row,
  Spin,
  Tag,
  Tree,
  type TreeDataNode,
  Typography,
} from "antd";
import { type ReactNode, useMemo, useState } from "react";
import { api, errorMessage, type HistoryEntry, inDesktop, queryKeys, type SourceRef, sourceKey } from "../../api";
import { FilePreviewView } from "../../components/FilePreviewView";
import { t } from "../../locales/zh-CN";
import { formatBytes } from "../../utils/format";
import { buildTree, type HNode } from "./buildTree";

/**
 * 只读浏览一个版本或安全备份中的文件（SRS 3.3.2.3、3.3.3.3）：左侧目录树，右侧文件信息与预览。
 * `actions` 用于放置“恢复”等针对所选文件的操作。
 */
export function HistoryBrowser({
  projectId,
  source,
  actions,
}: {
  projectId: string;
  source: SourceRef;
  actions?: (entry: HistoryEntry) => ReactNode;
}) {
  const files = useQuery({
    queryKey: queryKeys.sourceFiles(projectId, sourceKey(source)),
    queryFn: () => api.sourceFiles(projectId, source),
    enabled: inDesktop,
  });
  const [selected, setSelected] = useState<HistoryEntry | null>(null);
  const [filter, setFilter] = useState("");

  const tree = useMemo(() => (files.data ? buildTree(files.data) : []), [files.data]);
  const treeData = useMemo(() => {
    const kw = filter.trim().toLowerCase();
    if (!kw) return toTreeData(tree);
    // 筛选时平铺显示匹配的文件
    return (files.data ?? [])
      .filter((e) => e.entry_type === "file" && e.path.toLowerCase().includes(kw))
      .map((e) => toTreeData([{ key: e.path, name: e.path, isDir: false, entry: e, children: [] }])[0] as TreeDataNode);
  }, [tree, files.data, filter]);

  if (files.error) return <Alert type="error" showIcon title={errorMessage(files.error)} />;
  if (!files.data) return <Spin style={{ display: "block", marginTop: 60 }} />;

  return (
    <Row gutter={12}>
      <Col xs={24} lg={9}>
        <Card size="small" styles={{ body: { padding: 8 } }}>
          <Input.Search
            allowClear
            placeholder={t.tree.filter}
            value={filter}
            onChange={(e) => setFilter(e.target.value)}
            style={{ marginBottom: 8 }}
          />
          <div style={{ height: 520, overflow: "auto" }}>
            <Tree
              showIcon
              blockNode
              height={510}
              defaultExpandAll={(files.data?.length ?? 0) < 200}
              treeData={treeData}
              onSelect={(_, info) => {
                const e = (info.node as TreeDataNode & { entry?: HistoryEntry | null }).entry ?? null;
                setSelected(e);
              }}
            />
          </div>
        </Card>
      </Col>
      <Col xs={24} lg={15}>
        <Card size="small">
          {selected && selected.entry_type === "file" ? (
            <FilePanel projectId={projectId} source={source} entry={selected} actions={actions} />
          ) : selected ? (
            <Empty description={t.tree.emptyFolder} />
          ) : (
            <Empty description={t.tree.pick} />
          )}
        </Card>
      </Col>
    </Row>
  );
}

function toTreeData(nodes: HNode[]): TreeDataNode[] {
  return nodes.map((n) => ({
    key: n.key,
    title:
      n.entry && !n.entry.available ? (
        <Typography.Text type="danger">
          {n.name} <WarningOutlined />
        </Typography.Text>
      ) : (
        n.name
      ),
    icon: n.isDir ? <FolderOutlined /> : <FileOutlined />,
    isLeaf: !n.isDir,
    children: n.isDir ? toTreeData(n.children) : undefined,
    entry:
      n.entry ?? (n.isDir ? { path: n.key, entry_type: "directory", size: null, hash: null, available: true } : null),
  })) as TreeDataNode[];
}

function FilePanel({
  projectId,
  source,
  entry,
  actions,
}: {
  projectId: string;
  source: SourceRef;
  entry: HistoryEntry;
  actions?: (entry: HistoryEntry) => ReactNode;
}) {
  const preview = useQuery({
    queryKey: queryKeys.preview(projectId, sourceKey(source), entry.path),
    queryFn: () => api.previewFile(projectId, source, entry.path),
    enabled: inDesktop && entry.available,
  });
  return (
    <Flex vertical gap={12}>
      <Flex justify="space-between" align="flex-start" gap={12} wrap>
        <Descriptions column={1} size="small" style={{ flex: 1, minWidth: 240 }}>
          <Descriptions.Item label={t.overview.path}>
            <Typography.Text copyable>{entry.path}</Typography.Text>
          </Descriptions.Item>
          <Descriptions.Item label={t.tree.size}>
            {entry.size !== null && <span title={`${entry.size} 字节`}>{formatBytes(entry.size)}</span>}
          </Descriptions.Item>
          <Descriptions.Item label={t.tree.hash}>
            <Typography.Text type="secondary" style={{ fontSize: 12 }} ellipsis={{ tooltip: entry.hash }}>
              {entry.hash}
            </Typography.Text>
          </Descriptions.Item>
        </Descriptions>
        {actions?.(entry)}
      </Flex>
      {!entry.available ? (
        <Tag color="error" icon={<WarningOutlined />}>
          {t.tree.corrupted}
        </Tag>
      ) : preview.error ? (
        <Alert type="error" showIcon title={errorMessage(preview.error)} />
      ) : preview.data ? (
        <FilePreviewView projectId={projectId} preview={preview.data} height={400} />
      ) : (
        <Spin />
      )}
    </Flex>
  );
}
