import { ArrowLeftOutlined } from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { Link, useParams } from "@tanstack/react-router";
import { Alert, Button, Flex, Typography } from "antd";
import { useState } from "react";
import { api, type HistoryEntry, inDesktop, queryKeys } from "../api";
import { RestoreFileDialog } from "../components/RestoreFileDialog";
import { HistoryBrowser } from "../features/history/HistoryBrowser";
import { t } from "../locales/zh-CN";
import { formatTime, versionLabel } from "../utils/format";

/** 历史版本文件树（SRS 3.3.2.3、表 8-3“历史文件树”）。 */
export function FileTreePage() {
  const { projectId, versionId } = useParams({ from: "/projects/$projectId/versions/$versionId/files" });
  const map = useQuery({
    queryKey: queryKeys.timeMap(projectId),
    queryFn: () => api.timeMap(projectId),
    enabled: inDesktop,
  });
  const node = map.data?.nodes.find((n) => n.version_id === versionId);
  const [restoring, setRestoring] = useState<HistoryEntry | null>(null);

  return (
    <Flex vertical gap={12}>
      <Flex align="center" gap={12} wrap>
        <Link to="/projects/$projectId/map" params={{ projectId }}>
          <Button icon={<ArrowLeftOutlined />}>{t.nav.timeMap}</Button>
        </Link>
        <Typography.Title level={4} style={{ margin: 0 }}>
          {t.tree.title(node ? versionLabel(node.seq) : "…")}
        </Typography.Title>
        {node && (
          <Typography.Text type="secondary">
            {node.name} {formatTime(node.created_at)}
          </Typography.Text>
        )}
      </Flex>
      <Alert type="info" showIcon title={t.tree.readOnly} />
      <HistoryBrowser
        projectId={projectId}
        source={{ kind: "version", version_id: versionId }}
        actions={(entry) => (
          <Button type="primary" onClick={() => setRestoring(entry)}>
            {t.restoreFile.button}
          </Button>
        )}
      />
      {restoring && (
        <RestoreFileDialog
          open
          projectId={projectId}
          source={{ kind: "version", version_id: versionId }}
          path={restoring.path}
          onClose={() => setRestoring(null)}
        />
      )}
    </Flex>
  );
}
