/**
 * 通用的界面小部件：页面标题、变化类型标记、路径显示。样式见 styles.css。
 */
import {
  ArrowLeftOutlined,
  FileExcelOutlined,
  FileImageOutlined,
  FileMarkdownOutlined,
  FileOutlined,
  FilePdfOutlined,
  FilePptOutlined,
  FileTextOutlined,
  FileWordOutlined,
  FileZipOutlined,
  FolderOutlined,
} from "@ant-design/icons";
import { Link } from "@tanstack/react-router";
import { Button } from "antd";
import type { ReactNode } from "react";
import { t } from "../locales/zh-CN";

export function PageHeader({ title, sub, extra }: { title: ReactNode; sub?: ReactNode; extra?: ReactNode }) {
  return (
    <div className="lf-page-header">
      <div style={{ minWidth: 0 }}>
        <h1>{title}</h1>
        {sub && <div className="lf-sub">{sub}</div>}
      </div>
      {extra && <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>{extra}</div>}
    </div>
  );
}

/** 子页面（比较、历史文件）左上角返回时间地图的链接。 */
export function BackLink({ projectId }: { projectId: string }) {
  return (
    <div style={{ marginBottom: -12 }}>
      <Link to="/projects/$projectId/map" params={{ projectId }}>
        <Button type="text" size="small" icon={<ArrowLeftOutlined />} style={{ marginInlineStart: -8 }}>
          {t.nav.timeMap}
        </Button>
      </Link>
    </div>
  );
}

/** 页面内容容器。`full` 用于需要占满高度的页面（如时间地图）。 */
export function Page({ children, full }: { children: ReactNode; full?: boolean }) {
  return <div className={`lf-page lf-fade${full ? " full" : ""}`}>{children}</div>;
}

export type KindTone = "added" | "modified" | "deleted" | "other";

/** 把变化类型归为四种颜色：新增、修改、删除，其余为中性色。 */
export function kindTone(kind: string): KindTone {
  return kind === "added" || kind === "modified" || kind === "deleted" ? kind : "other";
}

export function KindLabel({ kind, children }: { kind: string; children: ReactNode }) {
  return <span className={`lf-kind ${kindTone(kind)}`}>{children}</span>;
}

/** 相对上一版本的增减，三种颜色区分。 */
export function Delta({ added, modified, deleted }: { added: number; modified: number; deleted: number }) {
  return (
    <span className="lf-delta">
      <span className="added">
        {t.changeKind.added} {added}
      </span>
      <span className="modified">
        {t.changeKind.modified} {modified}
      </span>
      <span className="deleted">
        {t.changeKind.deleted} {deleted}
      </span>
    </span>
  );
}

const ICONS: Array<[RegExp, ReactNode]> = [
  [/\.(png|jpe?g|gif|bmp|webp|svg|heic|tiff?)$/i, <FileImageOutlined key="img" />],
  [/\.(docx?|odt|rtf)$/i, <FileWordOutlined key="doc" />],
  [/\.(xlsx?|csv|ods)$/i, <FileExcelOutlined key="xls" />],
  [/\.(pptx?|odp|key)$/i, <FilePptOutlined key="ppt" />],
  [/\.pdf$/i, <FilePdfOutlined key="pdf" />],
  [/\.(md|markdown)$/i, <FileMarkdownOutlined key="md" />],
  [/\.(zip|rar|7z|tar|gz|bz2|xz)$/i, <FileZipOutlined key="zip" />],
  [
    /\.(txt|log|json|ya?ml|toml|ini|xml|html?|css|js|ts|tsx|py|rs|c|cpp|h|java|bib|tex)$/i,
    <FileTextOutlined key="txt" />,
  ],
];

export function fileIcon(path: string, isDir: boolean): ReactNode {
  if (isDir) return <FolderOutlined />;
  return ICONS.find(([re]) => re.test(path))?.[1] ?? <FileOutlined />;
}

/** 路径：图标 + 淡色的上级目录 + 文件名。 */
export function PathText({ path, isDir = false, suffix }: { path: string; isDir?: boolean; suffix?: ReactNode }) {
  const i = path.lastIndexOf("/");
  const dir = i >= 0 ? path.slice(0, i + 1) : "";
  const name = i >= 0 ? path.slice(i + 1) : path;
  return (
    <span className="lf-path" title={path}>
      {fileIcon(path, isDir)}
      <span className="lf-path-text">
        {dir && <span className="lf-path-dir">{dir}</span>}
        {name}
        {suffix}
      </span>
    </span>
  );
}
