import {
  ClockCircleOutlined,
  DeleteOutlined,
  FileSearchOutlined,
  FolderAddOutlined,
  FolderOutlined,
  MoreOutlined,
  NodeIndexOutlined,
  RightOutlined,
  WarningOutlined,
} from "@ant-design/icons";
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { Alert, App, Button, Dropdown, Skeleton, Tag, Tooltip } from "antd";
import dayjs from "dayjs";
import { useEffect, useMemo } from "react";
import { api, appEvents, errorMessage, inDesktop, type ProjectOverview, type ProjectSummary, queryKeys } from "../api";
import { useAddProject } from "../hooks/useAddProject";
import { Brand } from "../layouts/ProjectLayout";
import { t } from "../locales/zh-CN";
import { formatAgo, versionLabel } from "../utils/format";
import { recentOpened } from "../utils/recent";

/**
 * 首页（SRS 3.3.1.1，表 8-3“项目列表”）：继续上次的项目、全部项目；选择或拖入文件夹加入；打开、移除项目。
 * 没有项目时显示三步引导。
 */
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

  // 可访问的在前；其中最近打开的排在前面，其余按加入时间从新到旧
  const recent = useMemo(() => recentOpened(), []);
  const list = useMemo(
    () =>
      [...(projects.data ?? [])].sort(
        (a, b) =>
          Number(b.accessible) - Number(a.accessible) ||
          (recent[b.project_id] ?? 0) - (recent[a.project_id] ?? 0) ||
          b.created_at - a.created_at,
      ),
    [projects.data, recent],
  );

  // 每个项目的版本数和上次保存的版本（只读数据库，不扫描文件夹）
  const briefs = useQueries({
    queries: list.map((p) => ({
      queryKey: queryKeys.projectBrief(p.project_id),
      queryFn: () => api.projectOverview(p.project_id),
      enabled: inDesktop && p.accessible,
    })),
  });
  const briefOf = (i: number) => briefs[i]?.data;

  const last = list[0] && recent[list[0].project_id] && list[0].accessible ? list[0] : null;
  const go = (p: ProjectSummary, to?: "map" | "find") => {
    const params = { projectId: p.project_id };
    if (to === "map") navigate({ to: "/projects/$projectId/map", params });
    else if (to === "find") navigate({ to: "/projects/$projectId/find", params });
    else navigate({ to: "/projects/$projectId", params });
  };

  const addButton = (
    <Button type="primary" size="large" icon={<FolderAddOutlined />} onClick={pick} disabled={!inDesktop}>
      {t.projects.add}
    </Button>
  );

  return (
    <div className="lf-home">
      <div className="lf-home-inner lf-fade">
        <div className="lf-home-top">
          <Brand />
        </div>
        <div className="lf-hero">
          <div>
            <h1>{t.home.greeting(new Date().getHours())}</h1>
            <p>{t.home.tagline}</p>
          </div>
          {list.length > 0 && addButton}
        </div>
        {!inDesktop && <Alert type="info" showIcon title={t.app.browserOnly} />}
        {projects.error && <Alert type="error" showIcon title={errorMessage(projects.error)} />}

        {projects.isLoading ? (
          <Skeleton active />
        ) : list.length === 0 ? (
          <Onboarding addButton={addButton} />
        ) : (
          <>
            {last && (
              <section className="lf-home-section">
                <div className="lf-section-title">{t.home.continueTitle}</div>
                <ContinueCard
                  p={last}
                  brief={briefOf(0)}
                  openedAt={recent[last.project_id] ?? 0}
                  onOpen={(to) => go(last, to)}
                />
              </section>
            )}
            <section className="lf-home-section">
              <div className="lf-section-title">{t.home.allProjects(list.length)}</div>
              <div className="lf-project-grid">
                {list.map((p, i) => (
                  <ProjectCard
                    key={p.project_id}
                    p={p}
                    brief={briefOf(i)}
                    onOpen={() => go(p)}
                    onRemove={() => confirmRemove(p)}
                  />
                ))}
                <div className="lf-dropzone">
                  <FolderAddOutlined className="lf-dropzone-icon" />
                  {t.projects.dropTile}
                </div>
              </div>
            </section>
          </>
        )}
      </div>
    </div>
  );
}

function savedLine(brief: ProjectOverview | undefined): string {
  if (!brief) return "";
  return brief.baseline
    ? t.home.lastSaved(versionLabel(brief.baseline.seq), formatAgo(brief.baseline.created_at))
    : t.home.neverSaved;
}

function ContinueCard({
  p,
  brief,
  openedAt,
  onOpen,
}: {
  p: ProjectSummary;
  brief: ProjectOverview | undefined;
  openedAt: number;
  onOpen: (to?: "map" | "find") => void;
}) {
  return (
    <div className="lf-continue">
      <span className="lf-project-icon" style={{ width: 48, height: 48, fontSize: 22, borderRadius: 12 }}>
        <FolderOutlined />
      </span>
      <div className="lf-continue-main">
        <div className="lf-continue-name">{p.name}</div>
        <div className="lf-continue-path" title={p.root_path}>
          {p.root_path}
        </div>
        <div className="lf-continue-meta">
          {brief && (
            <span className="lf-meta">
              <NodeIndexOutlined />
              {brief.active_scheme ? t.overview.scheme(brief.active_scheme.name) : t.overview.defaultRoute}
            </span>
          )}
          {brief && (
            <span className="lf-meta">
              <ClockCircleOutlined />
              {savedLine(brief)}
              {brief.baseline?.name && <span style={{ color: "var(--ant-color-text)" }}>“{brief.baseline.name}”</span>}
            </span>
          )}
          {brief && <span className="lf-meta">{t.home.versions(brief.version_count)}</span>}
          {openedAt > 0 && <span className="lf-meta">{t.home.lastOpened(formatAgo(openedAt))}</span>}
        </div>
      </div>
      <div className="lf-continue-actions">
        <Button icon={<ClockCircleOutlined />} onClick={() => onOpen("map")}>
          {t.nav.timeMap}
        </Button>
        <Button icon={<FileSearchOutlined />} onClick={() => onOpen("find")}>
          {t.nav.find}
        </Button>
        <Button type="primary" onClick={() => onOpen()}>
          {t.home.openProject}
          <RightOutlined />
        </Button>
      </div>
    </div>
  );
}

function Onboarding({ addButton }: { addButton: React.ReactNode }) {
  return (
    <section className="lf-home-section">
      <div className="lf-section-title">{t.home.stepsTitle}</div>
      <div className="lf-steps">
        {t.home.steps.map((s, i) => (
          <div key={s.title} className="lf-step">
            <span className="lf-step-no">{i + 1}</span>
            <strong>{s.title}</strong>
            <p>{s.desc}</p>
          </div>
        ))}
      </div>
      <div className="lf-dropzone large">
        <FolderAddOutlined className="lf-dropzone-icon" />
        <strong className="lf-dropzone-title">{t.projects.emptyTitle}</strong>
        <span>{t.projects.dropHint}</span>
        {addButton}
      </div>
    </section>
  );
}

function ProjectCard({
  p,
  brief,
  onOpen,
  onRemove,
}: {
  p: ProjectSummary;
  brief: ProjectOverview | undefined;
  onOpen: () => void;
  onRemove: () => void;
}) {
  return (
    // biome-ignore lint/a11y/useSemanticElements: 卡片内还有“更多操作”按钮，不能整体用 <button>
    <div
      role="button"
      tabIndex={p.accessible ? 0 : -1}
      aria-disabled={!p.accessible}
      className={`lf-project-card${p.accessible ? "" : " disabled"}`}
      onClick={() => p.accessible && onOpen()}
      onKeyDown={(e) => {
        if (p.accessible && (e.key === "Enter" || e.key === " ")) {
          e.preventDefault();
          onOpen();
        }
      }}
    >
      <div className="lf-project-card-head">
        <span className="lf-project-icon">
          <FolderOutlined />
        </span>
        <div className="lf-project-card-title">
          <strong>{p.name}</strong>
          <span title={p.root_path}>{p.root_path}</span>
        </div>
        {/* 菜单渲染在 body 下，但其中的点击、按键仍会经 React 冒泡到卡片上，在这里拦下 */}
        {/* biome-ignore lint/a11y/noStaticElementInteractions: 只用于阻止事件冒泡 */}
        <span onClick={(e) => e.stopPropagation()} onKeyDown={(e) => e.stopPropagation()}>
          <Dropdown
            trigger={["click"]}
            menu={{
              items: [{ key: "remove", danger: true, icon: <DeleteOutlined />, label: t.projects.remove }],
              onClick: () => onRemove(),
            }}
          >
            <Button type="text" size="small" icon={<MoreOutlined />} aria-label={t.projects.more} />
          </Dropdown>
        </span>
      </div>
      <div className="lf-project-card-foot">
        <span className="lf-num">
          {brief
            ? `${t.home.versions(brief.version_count)} · ${savedLine(brief)}`
            : t.projects.addedOn(dayjs(p.created_at).format("YYYY-MM-DD"))}
        </span>
        <span style={{ display: "inline-flex", gap: 6 }}>
          {!p.accessible && (
            <Tooltip title={t.projects.unavailable}>
              <Tag icon={<WarningOutlined />} color="warning" style={{ marginInlineEnd: 0 }}>
                {t.projects.unavailableShort}
              </Tag>
            </Tooltip>
          )}
          {p.incomplete && (
            <Tag color="error" style={{ marginInlineEnd: 0 }}>
              {t.projects.incomplete}
            </Tag>
          )}
        </span>
      </div>
    </div>
  );
}
