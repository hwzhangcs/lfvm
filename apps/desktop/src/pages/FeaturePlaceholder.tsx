import { Card, Result, Tag, Typography } from "antd";
import { t } from "../locales/zh-CN";

type PageKey = keyof typeof t.pages;

const titles: Record<PageKey, string> = {
  overview: t.nav.overview,
  timeMap: t.nav.timeMap,
  compare: t.nav.compare,
  fileTree: t.nav.fileTree,
  find: t.nav.find,
  schemes: t.nav.schemes,
  backups: t.nav.backups,
  storage: t.nav.storage,
};

/** 尚未实现的页面：说明用途、对应需求编号和计划完成的阶段。 */
export function FeaturePlaceholder({ page }: { page: PageKey }) {
  const info = t.pages[page];
  return (
    <Card title={titles[page]} extra={<Tag>{info.srs}</Tag>}>
      <Typography.Paragraph type="secondary">{info.desc}</Typography.Paragraph>
      <Result status="info" title={t.placeholder.comingSoon} subTitle={t.placeholder.milestone(info.milestone)} />
    </Card>
  );
}
