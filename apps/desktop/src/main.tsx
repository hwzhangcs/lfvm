import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { RouterProvider } from "@tanstack/react-router";
import { App as AntApp, ConfigProvider } from "antd";
import zhCNLocale from "antd/locale/zh_CN";
import dayjs from "dayjs";
import "dayjs/locale/zh-cn";
import relativeTime from "dayjs/plugin/relativeTime";
import { StrictMode, useLayoutEffect } from "react";
import { createRoot } from "react-dom/client";
import { router } from "./router";
import "./styles.css";
import { appTheme, useSystemDark } from "./theme";

dayjs.extend(relativeTime);
dayjs.locale("zh-cn");

const queryClient = new QueryClient({
  defaultOptions: {
    // 本地数据只在操作后变化，由操作完成时主动刷新，不需要切回窗口时自动重新获取。
    queries: { refetchOnWindowFocus: false, retry: false },
  },
});

function Root() {
  const dark = useSystemDark();
  // styles.css 中的自定义颜色（--lf-*）按这个属性切换亮色/暗色；在绘制前设置，避免闪烁
  useLayoutEffect(() => {
    document.documentElement.dataset.theme = dark ? "dark" : "light";
  }, [dark]);
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
