import { FolderAddOutlined, FolderOpenOutlined, InboxOutlined, WarningOutlined } from "@ant-design/icons";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Alert, App, Button, Card, Empty, Flex, Space, Table, type TableColumnsType, Tag, Typography } from "antd";
import dayjs from "dayjs";
import { useEffect } from "react";
import { api, appEvents, errorMessage, inDesktop, type ProjectSummary, queryKeys } from "../api";
import { useAddProject } from "../hooks/useAddProject";
import { t } from "../locales/zh-CN";

/** 项目列表（SRS 3.3.1.1，表 8-3“项目列表”）：选择或拖入文件夹加入；查看、打开、移除项目。 */
export function ProjectListPage() {
  const { message, modal } = App.useApp();
  const navigate = useNavigate();
  const queryClient = useQueryClient();
  const addProject = useAddProject();
  const projects = useQuery({ queryKey: queryKeys.projects, queryFn: api.listProjects, enabled: inDesktop });

  // 拖入文件夹（Rust 端接收拖放，前端只拿到令牌）
  useEffect(() => {
    if (!inDesktop) return;
    const unlisten = appEvents.folderDropped.listen(({ payload }) => {
      if (payload.folder) void addProject(payload.folder);
      else message.warning(t.projects.dropNotFolder);
    });
    return () => void unlisten.then((f) => f());
  }, [addProject, message]);

  const pick = async () => {
    try {
      const folder = await api.pickFolder("add_project");
      if (folder) await addProject(folder);
    } catch (e) {
      message.error(errorMessage(e));
    }
  };

  const remove = useMutation({
    mutationFn: (id: string) => api.removeProject(id),
    onSuccess: () => {
      message.success(t.projects.removed);
      return queryClient.invalidateQueries({ queryKey: queryKeys.projects });
    },
    onError: (e) => message.error(errorMessage(e)),
  });

  const confirmRemove = (p: ProjectSummary) =>
    modal.confirm({
      title: t.projects.removeConfirmTitle(p.name),
      content: t.projects.removeConfirmContent,
      okText: t.projects.remove,
      cancelText: t.common.cancel,
      onOk: () => remove.mutateAsync(p.project_id),
    });

  const columns: TableColumnsType<ProjectSummary> = [
    {
      title: t.projects.name,
      key: "name",
      render: (_, p) => (
        <Flex vertical gap={2}>
          <Space wrap>
            <Typography.Text strong>{p.name}</Typography.Text>
            {!p.accessible && (
              <Tag icon={<WarningOutlined />} color="warning">
                {t.projects.unavailable}
              </Tag>
            )}
            {p.incomplete && <Tag color="error">{t.projects.incomplete}</Tag>}
          </Space>
          <Typography.Text type="secondary" ellipsis={{ tooltip: p.root_path }}>
            {p.root_path}
          </Typography.Text>
        </Flex>
      ),
    },
    {
      title: t.projects.addedAt,
      dataIndex: "created_at",
      width: 160,
      render: (ms: number) => dayjs(ms).format("YYYY-MM-DD HH:mm"),
    },
    {
      title: "",
      key: "actions",
      width: 180,
      align: "right",
      render: (_, p) => (
        <Space size={0}>
          <Button
            type="link"
            icon={<FolderOpenOutlined />}
            disabled={!p.accessible}
            onClick={() => navigate({ to: "/projects/$projectId", params: { projectId: p.project_id } })}
          >
            {t.projects.open}
          </Button>
          <Button type="link" danger onClick={() => confirmRemove(p)}>
            {t.projects.remove}
          </Button>
        </Space>
      ),
    },
  ];

  return (
    <Flex vertical gap={16} style={{ maxWidth: 960, margin: "0 auto", padding: 24, height: "100%", overflow: "auto" }}>
      <Typography.Title level={3} style={{ margin: 0 }}>
        {t.app.title}
      </Typography.Title>
      {!inDesktop && <Alert type="info" showIcon title={t.app.browserOnly} />}
      {projects.error && <Alert type="error" showIcon title={errorMessage(projects.error)} />}
      <Card
        title={t.nav.projects}
        extra={
          <Button type="primary" icon={<FolderAddOutlined />} onClick={pick} disabled={!inDesktop}>
            {t.projects.add}
          </Button>
        }
      >
        <Table
          rowKey="project_id"
          columns={columns}
          dataSource={projects.data ?? []}
          loading={projects.isLoading}
          pagination={false}
          showHeader={(projects.data?.length ?? 0) > 0}
          locale={{
            emptyText: (
              <Empty
                image={<InboxOutlined style={{ fontSize: 48, opacity: 0.45 }} />}
                styles={{ image: { height: 56 } }}
                description={t.projects.dropHint}
              />
            ),
          }}
        />
      </Card>
    </Flex>
  );
}
