/**
 * 路由。页面与 SRS 表 8-3 的主要界面一一对应。
 * 桌面程序从本地协议加载页面，使用 hash 路由避免刷新时找不到页面。
 */
import { createHashHistory, createRootRoute, createRoute, createRouter, Outlet } from "@tanstack/react-router";
import { ProjectLayout } from "./layouts/ProjectLayout";
import { FeaturePlaceholder } from "./pages/FeaturePlaceholder";
import { OverviewPage } from "./pages/OverviewPage";
import { ProjectListPage } from "./pages/ProjectListPage";

const rootRoute = createRootRoute({ component: Outlet });

const projectListRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/",
  component: ProjectListPage,
});

export const projectRoute = createRoute({
  getParentRoute: () => rootRoute,
  path: "/projects/$projectId",
  component: ProjectLayout,
});

const overviewRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "/",
  component: OverviewPage,
});
const timeMapRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "map",
  component: () => <FeaturePlaceholder page="timeMap" />,
});
const compareRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "compare",
  component: () => <FeaturePlaceholder page="compare" />,
});
const fileTreeRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "versions/$versionId/files",
  component: () => <FeaturePlaceholder page="fileTree" />,
});
const findRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "find",
  component: () => <FeaturePlaceholder page="find" />,
});
const schemesRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "schemes",
  component: () => <FeaturePlaceholder page="schemes" />,
});
const backupsRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "backups",
  component: () => <FeaturePlaceholder page="backups" />,
});
const storageRoute = createRoute({
  getParentRoute: () => projectRoute,
  path: "storage",
  component: () => <FeaturePlaceholder page="storage" />,
});

const routeTree = rootRoute.addChildren([
  projectListRoute,
  projectRoute.addChildren([
    overviewRoute,
    timeMapRoute,
    compareRoute,
    fileTreeRoute,
    findRoute,
    schemesRoute,
    backupsRoute,
    storageRoute,
  ]),
]);

export const router = createRouter({ routeTree, history: createHashHistory() });

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
