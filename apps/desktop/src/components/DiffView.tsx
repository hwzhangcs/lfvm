import { Alert, Col, Empty, Flex, Row, Tag, Typography, theme } from "antd";
import { useMemo, useState } from "react";
import { type DiffLine, type FileDiff, type ImageSide, imageUrl } from "../api";
import { t } from "../locales/zh-CN";
import { ImageBox, ZoomControl } from "./ImageViewer";
import { VirtualRows } from "./VirtualRows";

/** 单个文件的内容差异：文本逐行、图片并排、其他类型说明原因（SRS 3.3.2.2 第 3～5 步）。 */
export function DiffView({ projectId, diff, height = 480 }: { projectId: string; diff: FileDiff; height?: number }) {
  if (diff.kind === "text") return <TextDiffView diff={diff} height={height} />;
  if (diff.kind === "image") return <ImageCompare projectId={projectId} a={diff.a} b={diff.b} height={height} />;
  return <Alert type="info" showIcon title={t.compare.degraded} description={diff.reason} />;
}

function TextDiffView({ diff, height }: { diff: Extract<FileDiff, { kind: "text" }>; height: number }) {
  const { token } = theme.useToken();
  const width = useMemo(() => String(diff.lines.length).length * 9 + 12, [diff.lines.length]);
  const bg = (l: DiffLine) =>
    l.tag === "insert" ? token.colorSuccessBg : l.tag === "delete" ? token.colorErrorBg : "transparent";
  const sign = (l: DiffLine) => (l.tag === "insert" ? "+" : l.tag === "delete" ? "−" : " ");
  const num = {
    width,
    flex: "none",
    textAlign: "right" as const,
    paddingRight: 6,
    color: token.colorTextQuaternary,
    userSelect: "none" as const,
  };

  return (
    <Flex vertical gap={8}>
      <Flex gap={8} wrap>
        {diff.encoding_a && <Tag>A：{diff.encoding_a}</Tag>}
        {diff.encoding_b && <Tag>B：{diff.encoding_b}</Tag>}
        {diff.notes.map((n) => (
          <Tag key={n} color="warning">
            {n}
          </Tag>
        ))}
      </Flex>
      {diff.lines.length === 0 ? (
        <Empty description={t.compare.identical} />
      ) : (
        <div
          style={{
            border: `1px solid ${token.colorBorderSecondary}`,
            borderRadius: token.borderRadius,
            fontFamily: token.fontFamilyCode,
            fontSize: 13,
          }}
        >
          <VirtualRows
            items={diff.lines}
            height={height}
            render={(l) => (
              <div style={{ display: "flex", whiteSpace: "pre", lineHeight: "20px", background: bg(l) }}>
                <span style={num}>{l.old_no ?? ""}</span>
                <span style={num}>{l.new_no ?? ""}</span>
                <span style={{ width: 16, flex: "none", userSelect: "none" }}>{sign(l)}</span>
                <Typography.Text style={{ fontFamily: "inherit", fontSize: "inherit" }}>{l.text}</Typography.Text>
              </div>
            )}
          />
        </div>
      )}
    </Flex>
  );
}

function ImageCompare({
  projectId,
  a,
  b,
  height,
}: {
  projectId: string;
  a: ImageSide | null;
  b: ImageSide | null;
  height: number;
}) {
  const [zoom, setZoom] = useState<number | null>(null);
  const side = (s: ImageSide | null, label: string, missing: string) => (
    <Flex vertical gap={8}>
      <Typography.Text strong>
        {label}
        {s?.info && (
          <Typography.Text type="secondary">
            　{s.info.width}×{s.info.height}
          </Typography.Text>
        )}
      </Typography.Text>
      {!s ? (
        <Empty description={missing} style={{ height: height / 2 }} />
      ) : s.info ? (
        <ImageBox
          src={imageUrl(projectId, s.hash)}
          width={s.info.width}
          height={s.info.height}
          zoom={zoom}
          boxHeight={height}
        />
      ) : (
        <Alert type="warning" showIcon title={s.reason} />
      )}
    </Flex>
  );
  return (
    <Flex vertical gap={12}>
      <ZoomControl zoom={zoom} onChange={setZoom} />
      <Row gutter={12}>
        <Col span={12}>{side(a, t.compare.sideA, t.compare.notInA)}</Col>
        <Col span={12}>{side(b, t.compare.sideB, t.compare.notInB)}</Col>
      </Row>
    </Flex>
  );
}
