import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { App as AntApp, ConfigProvider } from "antd";
import zhCNLocale from "antd/locale/zh_CN";
import dayjs from "dayjs";
import "dayjs/locale/zh-cn";
import { StrictMode, useSyncExternalStore } from "react";
import { createRoot } from "react-dom/client";
import { router } from "./router";
import "./styles.css";
import { appTheme } from "./theme";

dayjs.locale("zh-cn");

const queryClient = new QueryClient({
  defaultOptions: {
    // 本地数据只在操作后变化，由操作完成时主动刷新，不需要切回窗口时自动重新获取。
    queries: { refetchOnWindowFocus: false, retry: false },
  },
});

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");
function useSystemDark(): boolean {
  return useSyncExternalStore(
    (cb) => {
      darkQuery.addEventListener("change", cb);
      return () => darkQuery.removeEventListener("change", cb);
    },
    () => darkQuery.matches,
  );
}

function Root() {
  const dark = useSystemDark();
  return (
    <ConfigProvider locale={zhCNLocale} theme={appTheme(dark)}>
      <AntApp>
        <QueryClientProvider client={queryClient}>
          <RouterProvider router={router} />
        </QueryClientProvider>
      </AntApp>
    </ConfigProvider>
  );
}

const rootEl = document.getElementById("root");
if (!rootEl) throw new Error("找不到 #root 元素");
createRoot(rootEl).render(
  <StrictMode>
    <Root />
  </StrictMode>,
);
