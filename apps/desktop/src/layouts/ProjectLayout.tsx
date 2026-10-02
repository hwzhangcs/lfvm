import {
  ArrowLeftOutlined,
  ClockCircleOutlined,
  DatabaseOutlined,
  FileSearchOutlined,
  HomeOutlined,
  NodeIndexOutlined,
  SafetyOutlined,
} from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { Link, Outlet, useLocation, useNavigate, useParams } from "@tanstack/react-router";
import { Button, Layout, Menu, Result, Spin, Typography, theme } from "antd";
import { api, errorMessage, inDesktop, queryKeys } from "../api";
import { t } from "../locales/zh-CN";

const { Sider, Content } = Layout;

/** 打开项目后的整体布局：左侧功能导航 + 右侧内容。进入时调用“打开项目”（SRS 3.3.1.1）。 */
export function ProjectLayout() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const { pathname } = useLocation();
  const navigate = useNavigate();
  const { token } = theme.useToken();
  const overview = useQuery({
    queryKey: queryKeys.overview(projectId),
    queryFn: () => api.openProject(projectId),
    enabled: inDesktop,
  });

  if (overview.error) {
    return (
      <Result
        status="warning"
        title={t.errors.openFailed}
        subTitle={errorMessage(overview.error)}
        extra={
          <Button type="primary" onClick={() => navigate({ to: "/" })}>
            {t.nav.backToProjects}
          </Button>
        }
      />
    );
  }

  const base = `/projects/${projectId}`;
  const items = [
    { key: base, icon: <HomeOutlined />, label: t.nav.overview },
    { key: `${base}/map`, icon: <ClockCircleOutlined />, label: t.nav.timeMap },
    { key: `${base}/find`, icon: <FileSearchOutlined />, label: t.nav.find },
    { key: `${base}/schemes`, icon: <NodeIndexOutlined />, label: t.nav.schemes },
    { key: `${base}/backups`, icon: <SafetyOutlined />, label: t.nav.backups },
    { key: `${base}/storage`, icon: <DatabaseOutlined />, label: t.nav.storage },
  ].map((it) => ({ ...it, label: <Link to={it.key}>{it.label}</Link> }));

  // 比较、历史文件等子页面归属到“时间地图”
  const selected =
    items
      .map((i) => i.key)
      .filter((k) => k !== base && pathname.startsWith(k))
      .at(0) ?? (pathname.includes("/compare") || pathname.includes("/versions/") ? `${base}/map` : base);

  const project = overview.data?.project;
  return (
    <Layout style={{ height: "100%" }}>
      <Sider width={208} style={{ borderRight: `1px solid ${token.colorBorderSecondary}` }}>
        <div style={{ padding: "16px 16px 8px" }}>
          <Link to="/">
            <Button type="text" size="small" icon={<ArrowLeftOutlined />}>
              {t.nav.backToProjects}
            </Button>
          </Link>
          <Typography.Title level={5} ellipsis={{ tooltip: project?.root_path }} style={{ margin: "12px 0 0" }}>
            {project?.name ?? "…"}
          </Typography.Title>
        </div>
        <Menu mode="inline" selectedKeys={[selected]} items={items} style={{ borderInlineEnd: 0 }} />
      </Sider>
      <Content style={{ overflow: "auto", padding: 24 }}>
        {overview.isLoading ? <Spin style={{ display: "block", marginTop: 80 }} /> : <Outlet />}
      </Content>
    </Layout>
  );
}
