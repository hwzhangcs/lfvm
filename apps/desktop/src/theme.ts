import { type ThemeConfig, theme } from "antd";

const base: ThemeConfig = {
  token: {
    colorPrimary: "#1677ff",
    borderRadius: 8,
    fontSize: 14,
    fontFamily:
      '-apple-system, BlinkMacSystemFont, "Segoe UI", "PingFang SC", "Microsoft YaHei", "Noto Sans CJK SC", "Helvetica Neue", Arial, sans-serif',
  },
  components: {
    Layout: { siderBg: "transparent", headerHeight: 56 },
  },
};

export function appTheme(dark: boolean): ThemeConfig {
  return { ...base, algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm };
}
