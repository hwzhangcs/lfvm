import { CompressOutlined } from "@ant-design/icons";
import { Button, Flex, InputNumber, Slider, Typography, theme } from "antd";
import { useState } from "react";
import { t } from "../locales/zh-CN";

export const ZOOM_MIN = 25;
export const ZOOM_MAX = 400;

/** 缩放控件：适应窗口，或 25%～400%（SRS 3.3.2.2 第 4 步）。zoom 为 null 表示适应窗口。 */
export function ZoomControl({ zoom, onChange }: { zoom: number | null; onChange: (z: number | null) => void }) {
  return (
    <Flex gap={12} align="center" wrap>
      <Button
        size="small"
        icon={<CompressOutlined />}
        type={zoom === null ? "primary" : "default"}
        onClick={() => onChange(null)}
      >
        {t.compare.fit}
      </Button>
      <Typography.Text type="secondary">{t.compare.zoom}</Typography.Text>
      <Slider
        style={{ width: 160 }}
        min={ZOOM_MIN}
        max={ZOOM_MAX}
        value={zoom ?? 100}
        onChange={(v) => onChange(v)}
        tooltip={{ formatter: (v) => `${v}%` }}
      />
      <InputNumber
        size="small"
        min={ZOOM_MIN}
        max={ZOOM_MAX}
        value={zoom ?? undefined}
        placeholder={t.compare.fit}
        suffix="%"
        style={{ width: 96 }}
        onChange={(v) => onChange(typeof v === "number" ? v : null)}
      />
    </Flex>
  );
}

/**
 * 按原始宽高比显示图片。图片来自预览协议，浏览器只把它当作图片加载，不执行任何内容。
 */
export function ImageBox({
  src,
  width,
  height,
  zoom,
  boxHeight = 420,
}: {
  src: string;
  width: number;
  height: number;
  zoom: number | null;
  boxHeight?: number;
}) {
  const { token } = theme.useToken();
  const [failed, setFailed] = useState(false);
  const style =
    zoom === null
      ? { maxWidth: "100%", maxHeight: boxHeight - 16, objectFit: "contain" as const }
      : { width: (width * zoom) / 100, height: (height * zoom) / 100, maxWidth: "none" };
  return (
    <div
      style={{
        height: boxHeight,
        overflow: "auto",
        display: "flex",
        alignItems: zoom === null ? "center" : "flex-start",
        justifyContent: zoom === null ? "center" : "flex-start",
        background: `repeating-conic-gradient(${token.colorFillSecondary} 0% 25%, transparent 0% 50%) 50% / 16px 16px`,
        border: `1px solid ${token.colorBorderSecondary}`,
        borderRadius: token.borderRadius,
      }}
    >
      {failed ? (
        <Typography.Text type="secondary">{t.compare.imageFailed}</Typography.Text>
      ) : (
        <img src={src} alt="" draggable={false} style={style} onError={() => setFailed(true)} />
      )}
    </div>
  );
}
