import {
  AimOutlined,
  BranchesOutlined,
  CloseOutlined,
  CompressOutlined,
  DiffOutlined,
  ExportOutlined,
  FolderOpenOutlined,
  MinusOutlined,
  PlusOutlined,
  RollbackOutlined,
  SearchOutlined,
} from "@ant-design/icons";
import { useQuery } from "@tanstack/react-query";
import { useNavigate, useParams } from "@tanstack/react-router";
import { Alert, Button, DatePicker, Empty, Input, Select, Skeleton, Space, Tooltip, Typography, theme } from "antd";
import dayjs, { type Dayjs } from "dayjs";
import { useCallback, useEffect, useImperativeHandle, useMemo, useRef, useState } from "react";
import { api, errorMessage, inDesktop, type MapNode, queryKeys, type TimeMap } from "../api";
import { ImpactDialog } from "../components/ImpactDialog";
import { Delta, Page, PageHeader } from "../components/ui";
import { OutputDialog, type OutputTarget } from "../features/output/OutputDialog";
import { CreateSchemeModal, SwitchFlow } from "../features/schemes/SchemeDialogs";
import {
  COL,
  laneColor,
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
/** 适应窗口时最多放大到的比例，避免版本很少时节点被放得过大。 */
const K_FIT_MAX = 1.25;
/** 左侧泳道标签占用的宽度，适应窗口时让出这块区域。 */
const GUTTER = 168;
const GRID = 24;

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

  // 打开时选中当前所在路线的最新版本：整张图放得下就整体显示，否则以它为中心
  useEffect(() => {
    if (!map.data || selected) return;
    const head = map.data.active_scheme_id
      ? map.data.schemes.find((s) => s.scheme_id === map.data?.active_scheme_id)?.head_version_id
      : map.data.default_head;
    if (head) {
      setSelected(head);
      requestAnimationFrame(() => canvas.current?.initial(head));
    }
  }, [map.data, selected]);

  if (map.error) {
    return (
      <Page>
        <Alert type="error" showIcon title={errorMessage(map.error)} />
      </Page>
    );
  }
  if (!map.data || !layout) {
    return (
      <Page>
        <Skeleton active paragraph={{ rows: 8 }} />
      </Page>
    );
  }
  if (map.data.nodes.length === 0) {
    return (
      <Page>
        <PageHeader title={t.map.title} />
        <div className="lf-panel" style={{ padding: "64px 0" }}>
          <Empty description={t.map.empty} />
        </div>
      </Page>
    );
  }

  const data = map.data;
  const node = data.nodes.find((n) => n.version_id === selected) ?? null;
  const label = (id: string | null) => {
    const n = id ? data.nodes.find((x) => x.version_id === id) : undefined;
    return n ? versionLabel(n.seq) : "—";
  };
  const liveSchemes = data.schemes.filter((s) => !s.cleared);
  const schemeOptions = [
    { value: "all", label: t.map.filterAll },
    { value: "default", label: t.map.filterDefault },
    ...liveSchemes.map((s) => ({ value: s.scheme_id, label: t.map.filterScheme(s.name) })),
  ];

  return (
    <Page full>
      <PageHeader title={t.map.title} sub={t.map.summary(data.nodes.length, liveSchemes.length)} />
      {data.over_scale && <Alert type="info" showIcon title={t.overScale} />}
      <div className="lf-map-toolbar">
        <Select
          style={{ width: 180 }}
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
          style={{ width: 260 }}
        />
        <Input
          allowClear
          prefix={<SearchOutlined style={{ color: "var(--ant-color-text-tertiary)" }} />}
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
      </div>

      <div className="lf-map-body">
        <MapCanvas
          ref={canvas}
          map={data}
          layout={layout}
          selected={selected}
          matches={matches}
          pair={pair}
          onSelect={setSelected}
          compareBar={
            <div className="lf-map-overlay lf-map-compare">
              {pair.a || pair.b ? (
                <>
                  <Slot name="A" value={pair.a && label(pair.a)} onClear={() => setPair((p) => ({ ...p, a: null }))} />
                  <span style={{ color: "var(--ant-color-text-tertiary)" }}>→</span>
                  <Slot name="B" value={pair.b && label(pair.b)} onClear={() => setPair((p) => ({ ...p, b: null }))} />
                </>
              ) : (
                <span style={{ color: "var(--ant-color-text-tertiary)", paddingInlineEnd: 4 }}>{t.node.pickTwo}</span>
              )}
              <Button
                type="primary"
                size="small"
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
            </div>
          }
        />
        <div className="lf-panel lf-detail">
          {node ? (
            <NodeDetails
              map={data}
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
            <Empty image={Empty.PRESENTED_IMAGE_SIMPLE} style={{ marginTop: 64 }} />
          )}
        </div>
      </div>
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
    </Page>
  );
}

function Slot({ name, value, onClear }: { name: string; value: string | null; onClear: () => void }) {
  return (
    <span className="lf-slot">
      <b>{name}</b>
      {value ?? "—"}
      {value && (
        <Button
          type="text"
          size="small"
          icon={<CloseOutlined style={{ fontSize: 10 }} />}
          aria-label={t.node.clearSlot}
          onClick={onClear}
          style={{ width: 20, height: 20, minWidth: 20 }}
        />
      )}
    </span>
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
  const schemeIndex = map.schemes.findIndex((s) => s.scheme_id === node.origin_scheme_id);
  const route = node.origin_scheme_name
    ? `${t.map.filterScheme(node.origin_scheme_name)}${map.schemes[schemeIndex]?.cleared ? t.map.clearedScheme : ""}`
    : t.map.defaultHistory;
  const off = node.cleared;
  return (
    <>
      <div className="lf-detail-head">
        <h2>
          {versionLabel(node.seq)}
          <small>{node.name || t.node.unnamed}</small>
        </h2>
        {node.note && <div className="lf-detail-note">{node.note}</div>}
      </div>
      <div className="lf-detail-body">
        {off && <Alert type="warning" showIcon title={t.map.clearedHint} />}
        <dl className="lf-kv" style={{ margin: 0 }}>
          <dt>{t.node.savedAt}</dt>
          <dd>{formatTime(node.created_at)}</dd>
          <dt>{t.node.route}</dt>
          <dd>
            <span className="lf-meta" style={{ color: "inherit" }}>
              <i
                style={{
                  width: 8,
                  height: 8,
                  borderRadius: "50%",
                  background: laneColor(schemeIndex + 1),
                  flex: "none",
                }}
              />
              {route}
            </span>
          </dd>
          <dt>{t.node.files}</dt>
          <dd>
            {node.file_count} / {node.directory_count}
          </dd>
          <dt>{t.node.changes}</dt>
          <dd title={t.node.changesValue(node.added, node.modified, node.deleted)}>
            <Delta added={node.added} modified={node.modified} deleted={node.deleted} />
          </dd>
        </dl>
        <div className="lf-actions">
          <Button className="wide" type="primary" icon={<FolderOpenOutlined />} disabled={off} onClick={onFiles}>
            {t.node.viewFiles}
          </Button>
          <Button className="wide" icon={<BranchesOutlined />} disabled={off} onClick={onBranch}>
            {t.schemes.createFromNode}
          </Button>
          <Button icon={<FolderOpenOutlined />} disabled={off} onClick={() => onOutput("expand")}>
            {t.output.expand}
          </Button>
          <Button icon={<ExportOutlined />} disabled={off} onClick={() => onOutput("export")}>
            {t.output.export}
          </Button>
          <Button disabled={off} onClick={onSetA}>
            {t.node.setA}
          </Button>
          <Button disabled={off} onClick={onSetB}>
            {t.node.setB}
          </Button>
        </div>
        <div className="lf-danger-zone">
          <Button danger icon={<RollbackOutlined />} disabled={off} onClick={onRestore}>
            {t.restoreVersion.button}
          </Button>
          <span>{t.node.restoreHint}</span>
        </div>
      </div>
    </>
  );
}

interface MapCanvasHandle {
  center: (versionId: string) => void;
  fit: () => void;
  initial: (versionId: string) => void;
}

function MapCanvas({
  ref,
  map,
  layout,
  selected,
  matches,
  pair,
  onSelect,
  compareBar,
}: {
  ref: React.Ref<MapCanvasHandle>;
  map: TimeMap;
  layout: MapLayout;
  selected: string | null;
  matches: string[];
  pair: { a: string | null; b: string | null };
  onSelect: (id: string) => void;
  compareBar: React.ReactNode;
}) {
  const { token } = theme.useToken();
  const box = useRef<HTMLDivElement>(null);
  const [view, setView] = useState<View>({ x: 0, y: 0, k: 1 });
  const [dragging, setDragging] = useState(false);
  const drag = useRef<{ x: number; y: number; vx: number; vy: number; moved: boolean; hit: string | null } | null>(
    null,
  );
  const matchSet = useMemo(() => new Set(matches), [matches]);
  const byId = useMemo(() => new Map(layout.nodes.map((p) => [p.node.version_id, p])), [layout]);

  const fitView = useCallback((): View | null => {
    const el = box.current;
    if (!el) return null;
    const w = el.clientWidth - GUTTER;
    const k = Math.min(K_FIT_MAX, Math.max(K_MIN, Math.min(w / layout.width, el.clientHeight / layout.height)));
    return { x: GUTTER + (w - layout.width * k) / 2, y: (el.clientHeight - layout.height * k) / 2, k };
  }, [layout]);

  const fit = useCallback(() => {
    const v = fitView();
    if (v) setView(v);
  }, [fitView]);

  const center = useCallback(
    (id: string) => {
      const el = box.current;
      const p = byId.get(id);
      if (!el || !p) return;
      setView((v) => ({ ...v, x: el.clientWidth / 2 - p.x * v.k, y: el.clientHeight / 2 - p.y * v.k }));
    },
    [byId],
  );

  const initial = useCallback(
    (id: string) => {
      const v = fitView();
      if (v && v.k >= 0.75) setView(v);
      else center(id);
    },
    [fitView, center],
  );

  useImperativeHandle(ref, () => ({ center, fit, initial }), [center, fit, initial]);

  const zoomBy = (factor: number) => {
    const el = box.current;
    if (!el) return;
    const mx = el.clientWidth / 2;
    const my = el.clientHeight / 2;
    setView((v) => {
      const k = Math.min(K_MAX, Math.max(K_MIN, v.k * factor));
      return { k, x: mx - ((mx - v.x) * k) / v.k, y: my - ((my - v.y) * k) / v.k };
    });
  };

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

  const heads = new Set(layout.markers.filter((m) => m.kind === "head").map((m) => m.versionId));
  const activeLane = map.active_scheme_id ? map.schemes.findIndex((s) => s.scheme_id === map.active_scheme_id) + 1 : 0;
  const colors = { text: token.colorTextSecondary, faint: token.colorTextTertiary, bg: token.colorBgContainer };

  return (
    <div
      ref={box}
      role="application"
      className="lf-map-canvas"
      style={{
        cursor: dragging ? "grabbing" : "grab",
        backgroundSize: `${GRID * view.k}px ${GRID * view.k}px`,
        backgroundPosition: `${view.x}px ${view.y}px`,
      }}
      onPointerDown={(e) => {
        if ((e.target as Element).closest(".lf-map-overlay")) return;
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
        if (!d.moved && Math.abs(dx) + Math.abs(dy) > 3) {
          d.moved = true;
          setDragging(true);
        }
        if (d.moved) setView((v) => ({ ...v, x: d.vx + dx, y: d.vy + dy }));
      }}
      onPointerUp={() => {
        const d = drag.current;
        drag.current = null;
        setDragging(false);
        if (d && !d.moved && d.hit) onSelect(d.hit);
      }}
    >
      <svg width="100%" height="100%" aria-label={t.map.title} style={{ display: "block" }}>
        <title>{t.map.title}</title>
        <g transform={`translate(${view.x},${view.y}) scale(${view.k})`}>
          {layout.lanes.map((l) => (
            <line
              key={l.lane}
              x1={-4000}
              x2={layout.width + 4000}
              y1={l.y}
              y2={l.y}
              stroke={l.color}
              strokeOpacity={l.cleared ? 0.04 : 0.08}
              strokeWidth={30}
            />
          ))}
          {layout.edges.map((e) => (
            <path
              key={`${e.from.node.version_id}-${e.to.node.version_id}`}
              d={edgePath(e.from, e.to)}
              fill="none"
              stroke={e.color}
              strokeWidth={2}
              strokeLinecap="round"
            />
          ))}
          {layout.nodes.map((p) => (
            <NodeMark
              key={p.node.version_id}
              p={p}
              selected={p.node.version_id === selected}
              matched={matchSet.has(p.node.version_id)}
              head={heads.has(p.node.version_id)}
              tag={pair.a === p.node.version_id ? "A" : pair.b === p.node.version_id ? "B" : null}
              colors={colors}
              primary={token.colorPrimary}
            />
          ))}
        </g>
      </svg>

      <div className="lf-map-fade" />
      {layout.lanes.map((l) => {
        const s = l.lane > 0 ? map.schemes[l.lane - 1] : undefined;
        const base = s && map.nodes.find((n) => n.version_id === s.base_version_id);
        return (
          <div
            key={l.lane}
            className="lf-map-lane-label"
            style={{ top: view.y + l.y * view.k, opacity: l.cleared ? 0.6 : 1 }}
          >
            <i style={{ background: l.color }} />
            <strong>{l.label}</strong>
            {base && <span>{t.map.startsAt(versionLabel(base.seq))}</span>}
            {l.lane === activeLane && <em>{t.map.current}</em>}
          </div>
        );
      })}

      {compareBar}

      <div className="lf-map-overlay lf-map-zoom" title={t.map.zoomHint}>
        <Tooltip title={t.map.zoomOut}>
          <Button
            type="text"
            size="small"
            icon={<MinusOutlined />}
            aria-label={t.map.zoomOut}
            onClick={() => zoomBy(1 / 1.25)}
          />
        </Tooltip>
        <span className="lf-zoom-value lf-num">{Math.round(view.k * 100)}%</span>
        <Tooltip title={t.map.zoomIn}>
          <Button
            type="text"
            size="small"
            icon={<PlusOutlined />}
            aria-label={t.map.zoomIn}
            onClick={() => zoomBy(1.25)}
          />
        </Tooltip>
        <Tooltip title={t.map.fit}>
          <Button type="text" size="small" icon={<CompressOutlined />} aria-label={t.map.fit} onClick={fit} />
        </Tooltip>
      </div>
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
  head,
  tag,
  colors,
  primary,
}: {
  p: PlacedNode;
  selected: boolean;
  matched: boolean;
  head: boolean;
  tag: "A" | "B" | null;
  colors: { text: string; faint: string; bg: string };
  primary: string;
}) {
  const n = p.node;
  const r = n.name ? 8.5 : 7;
  return (
    <g data-version={n.version_id} style={{ cursor: "pointer" }}>
      <title>
        {versionLabel(n.seq)} {n.name} {formatTime(n.created_at)}
        {n.note ? `\n${n.note}` : ""}
        {n.cleared ? `（${t.map.clearedNode}）` : ""}
      </title>
      {/* 扩大点击范围 */}
      <circle cx={p.x} cy={p.y} r={20} fill="transparent" />
      {matched && (
        <circle cx={p.x} cy={p.y} r={16} fill="none" stroke="#d4880f" strokeWidth={2} strokeDasharray="4 3" />
      )}
      {selected && (
        <circle cx={p.x} cy={p.y} r={14} fill={p.color} fillOpacity={0.16} stroke={p.color} strokeWidth={1.5} />
      )}
      <circle
        cx={p.x}
        cy={p.y}
        r={r}
        fill={n.cleared ? colors.bg : p.color}
        stroke={n.cleared ? p.color : colors.bg}
        strokeWidth={n.cleared ? 2 : 2.5}
        strokeDasharray={n.cleared ? "3 2" : undefined}
      />
      <text
        x={p.x}
        y={p.y - 20}
        fontSize={12}
        textAnchor="middle"
        fill={selected ? p.color : colors.text}
        fontWeight={selected ? 700 : 500}
      >
        {versionLabel(n.seq)}
      </text>
      <text x={p.x} y={p.y + 27} fontSize={10.5} textAnchor="middle" fill={colors.faint}>
        {dayjs(n.created_at).format("MM-DD")}
      </text>
      {head && (
        <g>
          <rect x={p.x + 18} y={p.y - 9} width={32} height={18} rx={9} fill={p.color} fillOpacity={0.14} />
          <text x={p.x + 34} y={p.y + 4} fontSize={10.5} textAnchor="middle" fill={p.color} fontWeight={600}>
            {t.map.latest}
          </text>
        </g>
      )}
      {tag && (
        <g>
          <rect x={p.x + 13} y={p.y - 33} width={16} height={16} rx={4} fill={primary} />
          <text x={p.x + 21} y={p.y - 21} fontSize={11} textAnchor="middle" fill="#fff" fontWeight={700}>
            {tag}
          </text>
        </g>
      )}
    </g>
  );
}
