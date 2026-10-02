import { useQuery } from "@tanstack/react-query";
import { Alert, Button, Descriptions, Empty, Flex, Modal, Select, Space, Spin, Tag, Timeline, Typography } from "antd";
import { useState } from "react";
import {
  api,
  errorMessage,
  inDesktop,
  queryKeys,
  type SwitchTarget,
  type TrailEntry,
  type TrailState,
} from "../../api";
import { FilePreviewView } from "../../components/FilePreviewView";
import { RestoreFileDialog } from "../../components/RestoreFileDialog";
import { t } from "../../locales/zh-CN";
import { formatTime, versionLabel } from "../../utils/format";

const COLOR: Record<TrailState, string> = {
  present: "gray",
  changed: "blue",
  missing: "red",
  excluded: "orange",
  unknown: "gray",
};

/** 历史文件轨迹（SRS 3.3.3.5）：选择历史路线，按版本列出出现、变化与缺失，分段显示，可预览或恢复。 */
export function TrailView({ projectId, path }: { projectId: string; path: string }) {
  const overview = useQuery({
    queryKey: queryKeys.overview(projectId),
    queryFn: () => api.openProject(projectId),
    enabled: inDesktop,
  });
  const schemes = useQuery({
    queryKey: queryKeys.schemes(projectId),
    queryFn: () => api.listSchemes(projectId),
    enabled: inDesktop,
  });
  // 默认为当前活动方案；处于默认历史时为默认历史
  const active = overview.data?.active_scheme?.scheme_id;
  const [picked, setPicked] = useState<string | null>(null);
  const routeKey = picked ?? active ?? "default";
  const route: SwitchTarget = routeKey === "default" ? { kind: "default" } : { kind: "scheme", scheme_id: routeKey };
  const [restoring, setRestoring] = useState<TrailEntry | null>(null);
  const [previewing, setPreviewing] = useState<TrailEntry | null>(null);

  const trail = useQuery({
    queryKey: queryKeys.trail(projectId, path, route),
    queryFn: () => api.fileTrail(projectId, path, route),
    enabled: inDesktop && !overview.isLoading,
  });
  const tr = trail.data;

  return (
    <Flex vertical gap={16}>
      <Flex gap={12} align="center">
        <Typography.Text>{t.trail.route}</Typography.Text>
        <Select
          style={{ width: 220 }}
          value={routeKey}
          onChange={setPicked}
          options={[
            { value: "default", label: t.schemes.defaultHistory },
            ...(schemes.data?.schemes ?? []).map((s) => ({ value: s.scheme_id, label: t.map.filterScheme(s.name) })),
          ]}
        />
      </Flex>
      <Typography.Text type="secondary">{t.trail.note}</Typography.Text>
      {trail.error ? (
        <Alert type="error" showIcon title={errorMessage(trail.error)} />
      ) : !tr ? (
        <Spin />
      ) : (
        <>
          {tr.all_cleared && <Alert type="warning" showIcon title={t.trail.allCleared} />}
          {tr.incomplete && !tr.all_cleared && <Alert type="warning" showIcon title={t.trail.incomplete} />}
          {tr.segments.length === 0 ? (
            <Empty description={t.find.noResult} />
          ) : (
            <Descriptions column={1} size="small" bordered>
              {tr.segments.map((s, i) => (
                <Descriptions.Item
                  key={s.first.version_id}
                  label={tr.segments.length > 1 ? t.trail.segment(i + 1) : t.trail.lastSeen}
                >
                  <Flex vertical>
                    <span>
                      {versionLabel(s.first.seq)} → {versionLabel(s.last.seq)}（{t.trail.lastSeen}：
                      {formatTime(s.last.created_at)}）
                    </span>
                    {s.first_missing ? (
                      <span>
                        {t.trail.firstMissing}：{versionLabel(s.first_missing.seq)}，
                        {formatTime(s.first_missing.created_at)}
                        {s.missing_by_exclusion && <Tag style={{ marginInlineStart: 8 }}>{t.trail.byExclusion}</Tag>}
                      </span>
                    ) : (
                      <Typography.Text type="success">{t.trail.stillPresent}</Typography.Text>
                    )}
                  </Flex>
                </Descriptions.Item>
              ))}
            </Descriptions>
          )}
          {tr.same_content.length > 0 && (
            <Alert
              type="info"
              showIcon
              title={t.trail.sameContent}
              description={
                <ul style={{ margin: 0, paddingInlineStart: 20 }}>
                  {tr.same_content.map((c) => (
                    <li key={c.path}>
                      <Typography.Text code>{c.path}</Typography.Text>（{versionLabel(c.version.seq)}）
                    </li>
                  ))}
                </ul>
              }
            />
          )}
          <Timeline
            items={tr.entries
              .slice()
              .reverse()
              .map((e) => ({
                key: e.version.version_id,
                color: COLOR[e.state],
                children: (
                  <Flex justify="space-between" align="center" gap={8}>
                    <span>
                      <Typography.Text strong>{versionLabel(e.version.seq)}</Typography.Text> {e.version.name}{" "}
                      <Typography.Text type="secondary">{formatTime(e.version.created_at)}</Typography.Text>{" "}
                      <Tag color={COLOR[e.state] === "gray" ? undefined : COLOR[e.state]}>
                        {t.trail.states[e.state]}
                      </Tag>
                    </span>
                    {e.hash && (
                      <Space size={4}>
                        <Button size="small" onClick={() => setPreviewing(e)}>
                          {t.find.preview}
                        </Button>
                        <Button size="small" onClick={() => setRestoring(e)}>
                          {t.restoreFile.button}
                        </Button>
                      </Space>
                    )}
                  </Flex>
                ),
              }))}
          />
        </>
      )}
      <Modal
        open={!!previewing}
        title={previewing ? `${versionLabel(previewing.version.seq)} · ${path}` : ""}
        width={760}
        footer={null}
        onCancel={() => setPreviewing(null)}
        destroyOnHidden
      >
        {previewing && <EntryPreview projectId={projectId} versionId={previewing.version.version_id} path={path} />}
      </Modal>
      {restoring && (
        <RestoreFileDialog
          open
          projectId={projectId}
          source={{ kind: "version", version_id: restoring.version.version_id }}
          path={path}
          onClose={() => setRestoring(null)}
        />
      )}
    </Flex>
  );
}

function EntryPreview({ projectId, versionId, path }: { projectId: string; versionId: string; path: string }) {
  const preview = useQuery({
    queryKey: queryKeys.preview(projectId, `v:${versionId}`, path),
    queryFn: () => api.previewFile(projectId, { kind: "version", version_id: versionId }, path),
    enabled: inDesktop,
  });
  if (preview.error) return <Alert type="error" showIcon title={errorMessage(preview.error)} />;
  if (!preview.data) return <Spin />;
  return <FilePreviewView projectId={projectId} preview={preview.data} height={480} />;
}
