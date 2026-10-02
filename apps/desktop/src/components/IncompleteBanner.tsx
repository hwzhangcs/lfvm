import { useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Alert, App, Button, Space } from "antd";
import { useState } from "react";
import { api, errorMessage, inDesktop, invalidateProject, queryKeys } from "../api";
import { t } from "../locales/zh-CN";
import { ImpactDialog } from "./ImpactDialog";

/** 存在未处置的未完成操作时的处置提示（SRS 6.1.1“用户处置”）。 */
export function IncompleteBanner({ projectId, incomplete }: { projectId: string; incomplete: boolean }) {
  const { modal, message } = App.useApp();
  const qc = useQueryClient();
  const navigate = useNavigate();
  const [retrying, setRetrying] = useState(false);
  const op = useQuery({
    queryKey: queryKeys.incomplete(projectId),
    queryFn: () => api.openIncomplete(projectId),
    enabled: inDesktop && incomplete,
  });
  const o = op.data;
  if (!incomplete || !o) return null;

  const keep = () =>
    modal.confirm({
      title: t.ops.keepConfirm,
      content: t.ops.keepConfirmDesc,
      okText: t.ops.keepCurrent,
      cancelText: t.common.cancel,
      onOk: async () => {
        try {
          await api.resolveIncomplete(projectId, o.operation_id);
          message.success(t.ops.resolved);
        } catch (e) {
          message.error(errorMessage(e));
        } finally {
          void invalidateProject(qc, projectId);
        }
      },
    });

  return (
    <>
      <Alert
        type="warning"
        showIcon
        style={{ marginBottom: 16 }}
        title={t.ops.incompleteTitle}
        description={t.ops.incompleteDesc(o.target_label)}
        action={
          <Space direction="vertical">
            <Button
              size="small"
              block
              onClick={() =>
                navigate({ to: "/projects/$projectId/backups", params: { projectId }, search: { op: o.operation_id } })
              }
            >
              {t.ops.viewDetail}
            </Button>
            <Button size="small" block type="primary" onClick={() => setRetrying(true)}>
              {t.ops.retry}
            </Button>
            <Button size="small" block onClick={keep}>
              {t.ops.keepCurrent}
            </Button>
          </Space>
        }
      />
      <ImpactDialog
        open={retrying}
        projectId={projectId}
        title={t.restoreVersion.retryTitle}
        plan={(task, ch) => api.planRetry(projectId, o.operation_id, task, ch)}
        run={(fp, rid, task, ch) => api.retryOperation(projectId, rid, o.operation_id, fp, task, ch)}
        onClose={() => setRetrying(false)}
      />
    </>
  );
}
