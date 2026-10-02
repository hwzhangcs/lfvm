import { Alert, Flex, Tag, Typography, theme } from "antd";
import { useState } from "react";
import { type FilePreview, imageUrl } from "../api";
import { ImageBox, ZoomControl } from "./ImageViewer";
import { VirtualRows } from "./VirtualRows";

/** 只读预览：文本按纯文本显示，PNG/JPEG 按图片显示，其他类型只显示原因（SRS 3.3.2.3）。 */
export function FilePreviewView({
  projectId,
  preview,
  height = 460,
}: {
  projectId: string;
  preview: FilePreview;
  height?: number;
}) {
  const c = preview.content;
  const [zoom, setZoom] = useState<number | null>(null);
  if (c.kind === "text") {
    const lines = c.text.split(/\r?\n/);
    return (
      <Flex vertical gap={8}>
        <Tag style={{ alignSelf: "flex-start" }}>{c.encoding}</Tag>
        <CodeLines lines={lines} height={height} />
      </Flex>
    );
  }
  if (c.kind === "image") {
    return (
      <Flex vertical gap={8}>
        <Flex justify="space-between" wrap gap={8}>
          <Tag>
            {c.info.format.toUpperCase()} · {c.info.width}×{c.info.height}
          </Tag>
          <ZoomControl zoom={zoom} onChange={setZoom} />
        </Flex>
        <ImageBox
          src={imageUrl(projectId, c.hash)}
          width={c.info.width}
          height={c.info.height}
          zoom={zoom}
          boxHeight={height}
        />
      </Flex>
    );
  }
  return <Alert type="info" showIcon title={c.reason} />;
}

function CodeLines({ lines, height }: { lines: string[]; height: number }) {
  const { token } = theme.useToken();
  const gutter = String(lines.length).length * 9 + 16;
  return (
    <div
      style={{
        border: `1px solid ${token.colorBorderSecondary}`,
        borderRadius: token.borderRadius,
        fontFamily: token.fontFamilyCode,
        fontSize: 13,
      }}
    >
      <VirtualRows
        items={lines}
        height={height}
        render={(line, i) => (
          <div style={{ display: "flex", whiteSpace: "pre", lineHeight: "20px" }}>
            <span
              style={{
                width: gutter,
                flex: "none",
                textAlign: "right",
                paddingRight: 8,
                color: token.colorTextQuaternary,
                userSelect: "none",
              }}
            >
              {i + 1}
            </span>
            <Typography.Text style={{ fontFamily: "inherit", fontSize: "inherit" }}>{line}</Typography.Text>
          </div>
        )}
      />
    </div>
  );
}
