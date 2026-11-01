import {
  AppstoreOutlined,
  ClockCircleOutlined,
  DatabaseOutlined,
  FileSearchOutlined,
  FolderOutlined,
  HistoryOutlined,
  HomeOutlined,
  NodeIndexOutlined,
  SafetyOutlined,
  SwapOutlined,
} from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { Link, Outlet, useLocation, useNavigate, useParams } from "@tanstack/react-router";
import { Button, Dropdown, Menu, Result, Skeleton } from "antd";
import { useEffect, useSyncExternalStore } from "react";
import { api, errorMessage, inDesktop, queryKeys } from "../api";
import { IncompleteBanner } from "../components/IncompleteBanner";
import { t } from "../locales/zh-CN";
import { markOpened } from "../utils/recent";

/** 应用标识（左上角）。点击回到项目列表。 */
export function Brand() {
  return (
    <Link to="/" className="lf-brand" style={{ textDecoration: "none" }}>
      <span className="lf-brand-mark">
        <HistoryOutlined />
      </span>
      <span className="lf-brand-text">{t.app.title}</span>
    </Link>
  );
}

const narrowQuery = window.matchMedia("(max-width: 860px)");
function useNarrow(): boolean {
  return useSyncExternalStore(
    (cb) => {
      narrowQuery.addEventListener("change", cb);
      return () => narrowQuery.removeEventListener("change", cb);
    },
    () => narrowQuery.matches,
  );
}

/** 打开项目后的整体布局：左侧功能导航 + 右侧内容。进入时调用“打开项目”（SRS 3.3.1.1）。 */
export function ProjectLayout() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const { pathname } = useLocation();
  const navigate = useNavigate();
  // 窄窗口（如最小窗口 + 放大显示）时侧栏只显示图标
  const narrow = useNarrow();
  const overview = useQuery({
    queryKey: queryKeys.overview(projectId),
    queryFn: () => api.openProject(projectId),
    enabled: inDesktop,
  });
  const projects = useQuery({ queryKey: queryKeys.projects, queryFn: api.listProjects, enabled: inDesktop });
  const opened = overview.isSuccess;
  useEffect(() => {
    if (opened) markOpened(projectId);
  }, [opened, projectId]);

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
  const others = (projects.data ?? []).filter((p) => p.project_id !== projectId && p.accessible);
  const switchMenu = {
    items: [
      ...others.map((p) => ({ key: p.project_id, icon: <FolderOutlined />, label: p.name })),
      ...(others.length ? [{ type: "divider" as const }] : []),
      { key: "__all", icon: <AppstoreOutlined />, label: t.nav.backToProjects },
    ],
    onClick: ({ key }: { key: string }) =>
      key === "__all" ? navigate({ to: "/" }) : navigate({ to: "/projects/$projectId", params: { projectId: key } }),
  };

  return (
    <div className="lf-shell">
      <aside className="lf-sider">
        <Brand />
        <Dropdown menu={switchMenu} trigger={["click"]}>
          <button
            type="button"
            className="lf-project-switch"
            title={project?.root_path}
            aria-label={t.nav.switchProject}
          >
            <span className="lf-project-icon" style={{ width: 30, height: 30, fontSize: 15, borderRadius: 7 }}>
              <FolderOutlined />
            </span>
            <span className="lf-project-switch-text">
              <strong>{project?.name ?? "…"}</strong>
              <span>{project?.root_path ?? ""}</span>
            </span>
            <SwapOutlined style={{ color: "var(--ant-color-text-tertiary)", fontSize: 12 }} />
          </button>
        </Dropdown>
        <Menu
          mode="inline"
          inlineCollapsed={narrow}
          selectedKeys={[selected]}
          items={items}
          style={{ borderInlineEnd: 0 }}
        />
      </aside>
      <main className="lf-main">
        {project?.incomplete && (
          <div style={{ padding: "16px 28px 0" }}>
            <IncompleteBanner projectId={projectId} incomplete={project.incomplete} />
          </div>
        )}
        {overview.isLoading ? (
          <div style={{ padding: 28 }}>
            <Skeleton active paragraph={{ rows: 6 }} />
          </div>
        ) : (
          <Outlet />
        )}
      </main>
    </div>
  );
}
