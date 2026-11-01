import { useQuery } from "@tanstack/react-query";
import { useParams } from "@tanstack/react-router";
import { Alert, Button } from "antd";
import { useState } from "react";
import { api, type HistoryEntry, inDesktop, queryKeys } from "../api";
import { RestoreFileDialog } from "../components/RestoreFileDialog";
import { BackLink, Page, PageHeader } from "../components/ui";
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
    <Page>
      <BackLink projectId={projectId} />
      <PageHeader
        title={t.tree.title(node ? versionLabel(node.seq) : "…")}
        sub={node && `${node.name ? `${node.name} · ` : ""}${formatTime(node.created_at)}`}
      />
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
    </Page>
  );
}
