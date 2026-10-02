import { useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { App, Button, Flex } from "antd";
import dayjs from "dayjs";
import { useCallback } from "react";
import { type AddCheck, api, errorMessage, type PickedFolder, queryKeys } from "../api";
import { t } from "../locales/zh-CN";

/**
 * 加入项目的完整流程（SRS 3.3.1.1）：校验所选文件夹 → 若该路径曾被移除，让用户选择关联原历史或作为新项目
 * （系统不自动关联，LFVM-Q-01）→ 加入并进入项目概览。
 */
export function useAddProject() {
  const { message, modal } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();

  const askAssociate = useCallback(
    (check: AddCheck) =>
      new Promise<string | null | undefined>((resolve) => {
        const prev = check.previous;
        if (!prev) {
          resolve(null);
          return;
        }
        // 返回值：原项目 ID = 关联；null = 作为新项目；undefined = 用户关闭对话框
        let settled = false;
        const done = (v: string | null) => {
          settled = true;
          resolve(v);
          instance.destroy();
        };
        const instance = modal.confirm({
          title: t.projects.previousTitle,
          content: t.projects.previousContent(
            prev.name,
            prev.version_count,
            dayjs(prev.created_at).format("YYYY-MM-DD"),
          ),
          closable: true,
          width: 520,
          footer: (
            <Flex justify="end" gap={8} style={{ marginTop: 16 }}>
              <Button onClick={() => done(null)}>{t.projects.startFresh}</Button>
              <Button type="primary" onClick={() => done(prev.project_id)}>
                {t.projects.associate}
              </Button>
            </Flex>
          ),
          afterClose: () => {
            if (!settled) resolve(undefined);
          },
        });
      }),
    [modal],
  );

  return useCallback(
    async (folder: PickedFolder) => {
      try {
        const check = await api.checkAddProject(folder.token);
        const associate = await askAssociate(check);
        if (associate === undefined) return;
        const project = await api.addProject(folder.token, associate);
        await queryClient.invalidateQueries({ queryKey: queryKeys.projects });
        message.success(t.projects.added(project.name));
        navigate({ to: "/projects/$projectId", params: { projectId: project.project_id } });
      } catch (e) {
        message.error(errorMessage(e));
      }
    },
    [askAssociate, message, navigate, queryClient],
  );
}
