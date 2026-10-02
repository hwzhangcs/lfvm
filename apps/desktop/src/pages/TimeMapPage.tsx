import {
  AimOutlined,
  BranchesOutlined,
  CompressOutlined,
  DiffOutlined,
  ExportOutlined,
  FolderOpenOutlined,
  RollbackOutlined,
  SearchOutlined,
} from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { useNavigate, useParams } from "@tanstack/react-router";
import {
  Alert,
  Button,
  Card,
  DatePicker,
  Descriptions,
  Empty,
  Flex,
  Input,
  Select,
  Space,
  Spin,
  Typography,
  theme,
} from "antd";
import type { Dayjs } from "dayjs";
import { useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { api, errorMessage, inDesktop, type MapNode, queryKeys, type TimeMap } from "../api";
import { ImpactDialog } from "../components/ImpactDialog";
import { OutputDialog, type OutputTarget } from "../features/output/OutputDialog";
import { CreateSchemeModal, SwitchFlow } from "../features/schemes/SchemeDialogs";
import {
  COL,
  layoutMap,
  type MapFilter,
  type MapLayout,
  matchNodes,
  type PlacedNode,
} from "../features/timemap/layout";
import { t } from "../locales/zh-CN";
import { formatTime, versionLabel } from "../utils/format";

interface View {
  x: number;
  y: number;
  k: number;
}

const K_MIN = 0.2;
const K_MAX = 3;

/** 时间地图（SRS 3.3.2.1、表 8-3）：版本节点、方案走向及起点、节点详情；缩放、平移、按日期/说明定位。 */
export function TimeMapPage() {
  const { projectId } = useParams({ from: "/projects/$projectId" });
  const navigate = useNavigate();
  const map = useQuery({
    queryKey: queryKeys.timeMap(projectId),
    queryFn: () => api.timeMap(projectId),
    enabled: inDesktop,
  });

  const [filter, setFilter] = useState<MapFilter>({ kind: "all" });
  const [selected, setSelected] = useState<string | null>(null);
  const [pair, setPair] = useState<{ a: string | null; b: string | null }>({ a: null, b: null });
  const [range, setRange] = useState<[Dayjs | null, Dayjs | null] | null>(null);
  const [keyword, setKeyword] = useState("");
  const [cursor, setCursor] = useState(0);
  const [restoring, setRestoring] = useState<MapNode | null>(null);
  const [output, setOutput] = useState<OutputTarget | null>(null);
  const [branching, setBranching] = useState<MapNode | null>(null);
  const [switching, setSwitching] = useState<{ id: string; label: string } | null>(null);

  const layout = useMemo(
    () =>
      map.data
        ? layoutMap(map.data, filter, t.map.defaultHistory, (s) => s.name + (s.cleared ? t.map.clearedScheme : ""))
        : null,
    [map.data, filter],
  );
  const matches = useMemo(() => {
    if (!layout || (!keyword.trim() && !range)) return [];
    const from = range?.[0]?.startOf("day").valueOf();
    const to = range?.[1]?.endOf("day").valueOf();
    const shown = layout.nodes.map((p) => p.node);
    return matchNodes(shown, { from, to, keyword });
  }, [layout, keyword, range]);
  const canvas = useRef<MapCanvasHandle>(null);

  const locate = (step: number) => {
    if (matches.length === 0) return;
    const i = (((cursor + step) % matches.length) + matches.length) % matches.length;
    setCursor(i);
    const id = matches[i] ?? null;
    setSelected(id);
    if (id) canvas.current?.center(id);
  };

  // 打开时定位到当前所在路线的最新版本
  useEffect(() => {
    if (!map.data || selected) return;
    const head = map.data.active_scheme_id
      ? map.data.schemes.find((s) => s.scheme_id === map.data?.active_scheme_id)?.head_version_id
      : map.data.default_head;
    if (head) {
      setSelected(head);
      requestAnimationFrame(() => canvas.current?.center(head));
    }
  }, [map.data, selected]);

  if (map.error) return <Alert type="error" showIcon title={errorMessage(map.error)} />;
  if (!map.data || !layout) return <Spin style={{ display: "block", marginTop: 80 }} />;
  if (map.data.nodes.length === 0) {
    return (
      <Card title={t.map.title}>
        <Empty description={t.map.empty} />
      </Card>
    );
  }

  const node = map.data.nodes.find((n) => n.version_id === selected) ?? null;
  const label = (id: string | null) => {
    const n = id ? map.data?.nodes.find((x) => x.version_id === id) : undefined;
    return n ? versionLabel(n.seq) : "—";
  };
  const schemeOptions = [
    { value: "all", label: t.map.filterAll },
    { value: "default", label: t.map.filterDefault },
    ...map.data.schemes
      .filter((s) => !s.cleared)
      .map((s) => ({ value: s.scheme_id, label: t.map.filterScheme(s.name) })),
  ];

  return (
    <Flex vertical gap={12} style={{ height: "100%" }}>
      <Card size="small">
        <Flex gap={12} wrap align="center">
          <Select
            style={{ width: 200 }}
            value={filter.kind === "scheme" ? filter.schemeId : filter.kind}
            options={schemeOptions}
            onChange={(v) => setFilter(v === "all" || v === "default" ? { kind: v } : { kind: "scheme", schemeId: v })}
          />
          <DatePicker.RangePicker
            value={range}
            onChange={(v) => {
              setRange(v);
              setCursor(0);
            }}
            placeholder={[t.map.dateRange, t.map.dateRange]}
            allowEmpty={[true, true]}
          />
          <Input
            allowClear
            prefix={<SearchOutlined />}
            placeholder={t.map.keyword}
            style={{ width: 220 }}
            value={keyword}
            onChange={(e) => {
              setKeyword(e.target.value);
              setCursor(0);
            }}
            onPressEnter={() => locate(0)}
          />
          <Space.Compact>
            <Button icon={<AimOutlined />} disabled={matches.length === 0} onClick={() => locate(0)}>
              {t.map.locate}
            </Button>
            <Button disabled={matches.length < 2} onClick={() => locate(1)}>
              ›
            </Button>
          </Space.Compact>
          {(keyword.trim() || range) && (
            <Typography.Text type={matches.length ? "secondary" : "danger"}>
              {matches.length ? t.map.matches(cursor + 1, matches.length) : t.map.noMatch}
            </Typography.Text>
          )}
          <Button icon={<CompressOutlined />} onClick={() => canvas.current?.fit()}>
            {t.map.fit}
          </Button>
          <Typography.Text type="secondary">{t.map.zoomHint}</Typography.Text>
        </Flex>
      </Card>

      <Flex gap={12} style={{ flex: 1, minHeight: 360 }}>
        <MapCanvas
          ref={canvas}
          layout={layout}
          selected={selected}
          matches={matches}
          pair={pair}
          onSelect={setSelected}
        />
        <Card
          size="small"
          style={{ width: 320, flex: "none", overflow: "auto" }}
          title={node ? versionLabel(node.seq) : t.map.title}
        >
          {node ? (
            <NodeDetails
              map={map.data}
              node={node}
              onFiles={() =>
                navigate({
                  to: "/projects/$projectId/versions/$versionId/files",
                  params: { projectId, versionId: node.version_id },
                })
              }
              onSetA={() => setPair((p) => ({ ...p, a: node.version_id }))}
              onSetB={() => setPair((p) => ({ ...p, b: node.version_id }))}
              onRestore={() => setRestoring(node)}
              onBranch={() => setBranching(node)}
              onOutput={(kind) =>
                setOutput({
                  mode: "single",
                  kind,
                  source: { kind: "version", version_id: node.version_id },
                  label: versionLabel(node.seq),
                })
              }
            />
          ) : (
            <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} />
          )}
          <Flex vertical gap={8} style={{ marginTop: 16 }}>
            <Typography.Text type="secondary">
              {pair.a && pair.b ? t.node.compareBar(label(pair.a), label(pair.b)) : t.node.pickTwo}
            </Typography.Text>
            <Button
              type="primary"
              icon={<DiffOutlined />}
              disabled={!pair.a || !pair.b}
              onClick={() =>
                pair.a &&
                pair.b &&
                navigate({
                  to: "/projects/$projectId/compare",
                  params: { projectId },
                  search: { a: pair.a, b: pair.b },
                })
              }
            >
              {t.node.compare}
            </Button>
          </Flex>
        </Card>
      </Flex>
      <OutputDialog projectId={projectId} target={output} onClose={() => setOutput(null)} />
      <CreateSchemeModal
        open={!!branching}
        projectId={projectId}
        versionId={branching?.version_id ?? null}
        onClose={() => setBranching(null)}
        onSwitch={(s) => setSwitching({ id: s.scheme_id, label: t.map.filterScheme(s.name) })}
      />
      <SwitchFlow
        projectId={projectId}
        target={switching ? { kind: "scheme", scheme_id: switching.id } : null}
        label={switching?.label ?? ""}
        onClose={() => setSwitching(null)}
      />
      {restoring && (
        <ImpactDialog
          open
          projectId={projectId}
          title={t.restoreVersion.title(versionLabel(restoring.seq))}
          plan={(task, ch) => api.planRestore(projectId, restoring.version_id, task, ch)}
          run={(fingerprint, requestId, task, ch) =>
            api.restoreVersion(
              { project_id: projectId, request_id: requestId, version_id: restoring.version_id, fingerprint },
              task,
              ch,
            )
          }
          onClose={() => setRestoring(null)}
        />
      )}
    </Flex>
  );
}

function NodeDetails({
  map,
  node,
  onFiles,
  onSetA,
  onSetB,
  onRestore,
  onBranch,
  onOutput,
}: {
  map: TimeMap;
  node: MapNode;
  onFiles: () => void;
  onSetA: () => void;
  onSetB: () => void;
  onRestore: () => void;
  onBranch: () => void;
  onOutput: (kind: "expand" | "export") => void;
}) {
  const route = node.origin_scheme_name
    ? `${t.map.filterScheme(node.origin_scheme_name)}${map.schemes.find((s) => s.scheme_id === node.origin_scheme_id)?.cleared ? t.map.clearedScheme : ""}`
    : t.map.defaultHistory;
  return (
    <Flex vertical gap={12}>
      {node.cleared && <Alert type="warning" showIcon title={t.map.clearedHint} />}
      <Descriptions column={1} size="small">
        <Descriptions.Item label={t.node.savedAt}>{formatTime(node.created_at)}</Descriptions.Item>
        <Descriptions.Item label={t.node.name}>{node.name || t.node.unnamed}</Descriptions.Item>
        {node.note && <Descriptions.Item label={t.node.note}>{node.note}</Descriptions.Item>}
        <Descriptions.Item label={t.node.route}>{route}</Descriptions.Item>
        <Descriptions.Item label={t.node.files}>
          {node.file_count} / {node.directory_count}
        </Descriptions.Item>
        <Descriptions.Item label={t.node.changes}>
          {t.node.changesValue(node.added, node.modified, node.deleted)}
        </Descriptions.Item>
      </Descriptions>
      <Flex vertical gap={8}>
        <Button icon={<FolderOpenOutlined />} disabled={node.cleared} onClick={onFiles}>
          {t.node.viewFiles}
        </Button>
        <Button icon={<BranchesOutlined />} disabled={node.cleared} onClick={onBranch}>
          {t.schemes.createFromNode}
        </Button>
        <Button icon={<RollbackOutlined />} disabled={node.cleared} onClick={onRestore}>
          {t.restoreVersion.button}
        </Button>
        <Space.Compact block>
          <Button
            style={{ width: "50%" }}
            icon={<FolderOpenOutlined />}
            disabled={node.cleared}
            onClick={() => onOutput("expand")}
          >
            {t.output.expand}
          </Button>
          <Button
            style={{ width: "50%" }}
            icon={<ExportOutlined />}
            disabled={node.cleared}
            onClick={() => onOutput("export")}
          >
            {t.output.export}
          </Button>
        </Space.Compact>
        <Space.Compact block>
          <Button style={{ width: "50%" }} disabled={node.cleared} onClick={onSetA}>
            {t.node.setA}
          </Button>
          <Button style={{ width: "50%" }} disabled={node.cleared} onClick={onSetB}>
            {t.node.setB}
          </Button>
        </Space.Compact>
      </Flex>
    </Flex>
  );
}

interface MapCanvasHandle {
  center: (versionId: string) => void;
  fit: () => void;
}

function MapCanvas({
  ref,
  layout,
  selected,
  matches,
  pair,
  onSelect,
}: {
  ref: React.Ref<MapCanvasHandle>;
  layout: MapLayout;
  selected: string | null;
  matches: string[];
  pair: { a: string | null; b: string | null };
  onSelect: (id: string) => void;
}) {
  const { token } = theme.useToken();
  const box = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View>({ x: 0, y: 0, k: 1 });
  const drag = useRef<{ x: number; y: number; vx: number; vy: number; moved: boolean; hit: string | null } | null>(
    null,
  );
  const matchSet = useMemo(() => new Set(matches), [matches]);
  const byId = useMemo(() => new Map(layout.nodes.map((p) => [p.node.version_id, p])), [layout]);

  const fit = useCallback(() => {
    const el = box.current;
    if (!el) return;
    const k = Math.min(
      K_MAX,
      Math.max(K_MIN, Math.min(el.clientWidth / layout.width, el.clientHeight / layout.height)),
    );
    setView({ x: (el.clientWidth - layout.width * k) / 2, y: (el.clientHeight - layout.height * k) / 2, k });
  }, [layout]);

  const center = useCallback(
    (id: string) => {
      const el = box.current;
      const p = byId.get(id);
      if (!el || !p) return;
      setView((v) => ({ ...v, x: el.clientWidth / 2 - p.x * v.k, y: el.clientHeight / 2 - p.y * v.k }));
    },
    [byId],
  );

  useImperativeHandle(ref, () => ({ center, fit }), [center, fit]);

  // 滚轮缩放（以鼠标位置为中心）。需要非被动监听才能阻止页面滚动。
  useEffect(() => {
    const el = box.current;
    if (!el) return;
    const onWheel = (e: WheelEvent) => {
      e.preventDefault();
      const rect = el.getBoundingClientRect();
      const mx = e.clientX - rect.left;
      const my = e.clientY - rect.top;
      setView((v) => {
        const k = Math.min(K_MAX, Math.max(K_MIN, v.k * Math.exp(-e.deltaY * 0.0015)));
        return { k, x: mx - ((mx - v.x) * k) / v.k, y: my - ((my - v.y) * k) / v.k };
      });
    };
    el.addEventListener("wheel", onWheel, { passive: false });
    return () => el.removeEventListener("wheel", onWheel);
  }, []);

  const markerOffsets = new Map<string, number>();

  return (
    <div
      ref={box}
      role="application"
      style={{
        flex: 1,
        position: "relative",
        overflow: "hidden",
        cursor: drag.current ? "grabbing" : "grab",
        background: token.colorBgContainer,
        border: `1px solid ${token.colorBorderSecondary}`,
        borderRadius: token.borderRadiusLG,
        touchAction: "none",
        userSelect: "none",
      }}
      onPointerDown={(e) => {
        // 指针被捕获后 pointerup 的目标是容器本身，因此在按下时记录点中的节点
        const hit = (e.target as Element).closest("[data-version]")?.getAttribute("data-version") ?? null;
        drag.current = { x: e.clientX, y: e.clientY, vx: view.x, vy: view.y, moved: false, hit };
        e.currentTarget.setPointerCapture(e.pointerId);
      }}
      onPointerMove={(e) => {
        const d = drag.current;
        if (!d) return;
        const dx = e.clientX - d.x;
        const dy = e.clientY - d.y;
        if (Math.abs(dx) + Math.abs(dy) > 3) d.moved = true;
        if (d.moved) setView((v) => ({ ...v, x: d.vx + dx, y: d.vy + dy }));
      }}
      onPointerUp={() => {
        const d = drag.current;
        drag.current = null;
        if (d && !d.moved && d.hit) onSelect(d.hit);
      }}
    >
      <svg width="100%" height="100%" aria-label={t.map.title}>
        <title>{t.map.title}</title>
        <g transform={`translate(${view.x},${view.y}) scale(${view.k})`}>
          {layout.lanes.map((l) => (
            <g key={l.lane}>
              <line
                x1={0}
                x2={layout.width}
                y1={l.y}
                y2={l.y}
                stroke={l.color}
                strokeOpacity={0.12}
                strokeWidth={28}
                strokeLinecap="round"
              />
              <text x={8} y={l.y - 20} fontSize={12} fill={l.color} opacity={l.cleared ? 0.5 : 0.9}>
                {l.label}
              </text>
            </g>
          ))}
          {layout.edges.map((e) => (
            <path
              key={`${e.from.node.version_id}-${e.to.node.version_id}`}
              d={edgePath(e.from, e.to)}
              fill="none"
              stroke={e.color}
              strokeWidth={2.5}
              opacity={0.8}
            />
          ))}
          {layout.markers.map((m) => {
            const p = byId.get(m.versionId);
            if (!p) return null;
            const off = markerOffsets.get(m.versionId) ?? 0;
            markerOffsets.set(m.versionId, off + 1);
            const text = m.kind === "base" ? t.map.base(m.label) : t.map.head(m.label);
            const y = p.y + 26 + off * 16;
            return (
              <text
                key={`${m.kind}-${m.label}-${m.versionId}`}
                x={p.x}
                y={y + 16}
                fontSize={11}
                textAnchor="middle"
                fill={m.color}
              >
                {m.kind === "base" ? "⚑ " : "★ "}
                {text}
              </text>
            );
          })}
          {layout.nodes.map((p) => (
            <NodeMark
              key={p.node.version_id}
              p={p}
              selected={p.node.version_id === selected}
              matched={matchSet.has(p.node.version_id)}
              tag={pair.a === p.node.version_id ? "A" : pair.b === p.node.version_id ? "B" : null}
              text={token.colorText}
              bg={token.colorBgContainer}
            />
          ))}
        </g>
      </svg>
    </div>
  );
}

function edgePath(a: PlacedNode, b: PlacedNode): string {
  if (a.y === b.y) return `M${a.x},${a.y} L${b.x},${b.y}`;
  const mx = a.x + COL / 2;
  return `M${a.x},${a.y} C${mx},${a.y} ${mx},${b.y} ${Math.max(mx + 8, b.x - COL / 2)},${b.y} L${b.x},${b.y}`;
}

function NodeMark({
  p,
  selected,
  matched,
  tag,
  text,
  bg,
}: {
  p: PlacedNode;
  selected: boolean;
  matched: boolean;
  tag: "A" | "B" | null;
  text: string;
  bg: string;
}) {
  const n = p.node;
  return (
    <g data-version={n.version_id} style={{ cursor: "pointer" }}>
      <title>
        {versionLabel(n.seq)} {n.name} {formatTime(n.created_at)}
        {n.cleared ? `（${t.map.clearedNode}）` : ""}
      </title>
      {matched && <circle cx={p.x} cy={p.y} r={20} fill="#fadb14" opacity={0.45} />}
      {selected && <circle cx={p.x} cy={p.y} r={17} fill="none" stroke={p.color} strokeWidth={2} />}
      <circle
        cx={p.x}
        cy={p.y}
        r={11}
        fill={n.cleared ? bg : p.color}
        stroke={p.color}
        strokeWidth={2.5}
        strokeDasharray={n.cleared ? "3 3" : undefined}
      />
      <text x={p.x} y={p.y - 18} fontSize={12} textAnchor="middle" fill={text} fontWeight={selected ? 600 : 400}>
        {versionLabel(n.seq)}
      </text>
      {tag && (
        <g>
          <circle cx={p.x + 13} cy={p.y - 12} r={8} fill="#000" opacity={0.75} />
          <text x={p.x + 13} y={p.y - 8} fontSize={10} textAnchor="middle" fill="#fff">
            {tag}
          </text>
        </g>
      )}
    </g>
  );
}
