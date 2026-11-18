import { type ThemeConfig, theme } from "antd";
import { useSyncExternalStore } from "react";

/**
 * 视觉规范：中性灰 + 一个主色（靛蓝）。
 * 颜色只在这里定义；自定义样式通过 antd 的 CSS 变量（--ant-*）和 styles.css 中的 --lf-* 引用，
 * 亮色、暗色自动跟随系统。
 */
const PRIMARY = "#5e6ad2";

const neutral = {
  light: {
    bgLayout: "#f6f6f7",
    bgContainer: "#ffffff",
    bgElevated: "#ffffff",
    border: "#e3e3e7",
    borderSecondary: "#ececef",
    fill: "#f1f1f3",
    text: "#1c1c21",
    textSecondary: "#62626d",
    textTertiary: "#93939d",
  },
  dark: {
    bgLayout: "#111113",
    bgContainer: "#19191c",
    bgElevated: "#222226",
    border: "#303036",
    borderSecondary: "#26262b",
    fill: "#232327",
    text: "#ececf0",
    textSecondary: "#a3a3ad",
    textTertiary: "#6f6f79",
  },
};

export function appTheme(dark: boolean): ThemeConfig {
  const n = dark ? neutral.dark : neutral.light;
  return {
    algorithm: dark ? theme.darkAlgorithm : theme.defaultAlgorithm,
    token: {
      colorPrimary: PRIMARY,
      colorInfo: PRIMARY,
      colorSuccess: "#2e9d6a",
      colorWarning: "#d4880f",
      colorError: "#d64545",
      colorLink: PRIMARY,
      colorBgLayout: n.bgLayout,
      colorBgContainer: n.bgContainer,
      colorBgElevated: n.bgElevated,
      colorBorder: n.border,
      colorBorderSecondary: n.borderSecondary,
      colorFillTertiary: n.fill,
      colorText: n.text,
      colorTextSecondary: n.textSecondary,
      colorTextTertiary: n.textTertiary,
      colorTextDescription: n.textSecondary,
      colorTextHeading: n.text,
      borderRadius: 6,
      borderRadiusLG: 10,
      borderRadiusSM: 4,
      fontSize: 14,
      fontFamily:
        '-apple-system, BlinkMacSystemFont, "Segoe UI Variable Text", "Segoe UI", "PingFang SC", "Microsoft YaHei UI", "Microsoft YaHei", "Noto Sans CJK SC", "Noto Sans SC", "Helvetica Neue", Arial, sans-serif',
      boxShadow: dark
        ? "0 8px 24px rgba(0,0,0,0.5), 0 0 0 1px rgba(255,255,255,0.06)"
        : "0 8px 24px rgba(20,20,30,0.10), 0 0 0 1px rgba(20,20,30,0.05)",
      boxShadowSecondary: dark
        ? "0 8px 24px rgba(0,0,0,0.5), 0 0 0 1px rgba(255,255,255,0.06)"
        : "0 8px 24px rgba(20,20,30,0.10), 0 0 0 1px rgba(20,20,30,0.05)",
      motionDurationMid: "0.15s",
    },
    components: {
      Layout: { siderBg: "transparent", bodyBg: n.bgLayout },
      Menu: {
        itemBg: "transparent",
        itemHeight: 34,
        itemMarginInline: 8,
        itemMarginBlock: 2,
        itemBorderRadius: 6,
        itemSelectedBg: dark ? "rgba(94,106,210,0.18)" : "rgba(94,106,210,0.10)",
        itemHoverBg: dark ? "rgba(255,255,255,0.05)" : "rgba(0,0,0,0.04)",
        iconSize: 15,
        activeBarBorderWidth: 0,
        collapsedWidth: 64,
      },
      Card: { headerFontSize: 15, headerHeight: 52, paddingLG: 20 },
      Table: {
        headerBg: "transparent",
        headerColor: n.textSecondary,
        headerSplitColor: "transparent",
        headerBorderRadius: 0,
        rowHoverBg: dark ? "rgba(255,255,255,0.03)" : "rgba(0,0,0,0.02)",
        cellPaddingBlockSM: 7,
      },
      Button: { primaryShadow: "none", defaultShadow: "none", dangerShadow: "none", fontWeight: 500 },
      Segmented: { itemSelectedBg: n.bgContainer, trackBg: n.fill },
      Tag: { defaultBg: n.fill },
      Descriptions: { labelColor: n.textSecondary, itemPaddingBottom: 10 },
      Modal: { titleFontSize: 16 },
      Alert: { withDescriptionPadding: "14px 16px" },
    },
  };
}

const darkQuery = window.matchMedia("(prefers-color-scheme: dark)");

/** 跟随系统的亮色/暗色设置。 */
export function useSystemDark(): boolean {
  return useSyncExternalStore(
    (cb) => {
      darkQuery.addEventListener("change", cb);
      return () => darkQuery.removeEventListener("change", cb);
    },
    () => darkQuery.matches,
  );
}
