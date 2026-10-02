import { DeleteOutlined, FileOutlined, FolderOutlined, PlusOutlined } from "@ant-design/icons";
import { keepPreviousData, useQuery } from "@tanstack/react-query";
import {
  Alert,
  App,
  Button,
  Drawer,
  Flex,
  Modal,
  Space,
  Switch,
  Table,
  type TableColumnsType,
  Tag,
  Tooltip,
  Tree,
  type TreeDataNode,
  Typography,
} from "antd";
import { useEffect, useMemo, useState } from "react";
import { api, type DirChild, type ExclusionRule, errorMessage, inDesktop, queryKeys } from "../api";
import { t } from "../locales/zh-CN";

interface Props {
  open: boolean;
  projectId: string;
  onClose: () => void;
  onSaved: () => void;
}

const ruleKey = (r: ExclusionRule) => `${r.entry_type}:${r.relative_path}`;

/**
 * 排除项设置（SRS 3.3.1.2）：规则先在编辑状态中修改，点击“保存规则”后才生效；取消时放弃本次编辑。
 */
export function ExclusionsDrawer({ open, projectId, onClose, onSaved }: Props) {
  const { message, modal } = App.useApp();
  const [rules, setRules] = useState<ExclusionRule[] | null>(null);
  const [selected, setSelected] = useState<string[]>([]);
  const [picking, setPicking] = useState(false);
  const [saving, setSaving] = useState(false);

  const saved = useQuery({
    queryKey: queryKeys.exclusions(projectId),
    queryFn: () => api.exclusionRules(projectId),
    enabled: inDesktop && open,
  });

  // 打开时以已保存的规则作为编辑起点
  useEffect(() => {
    if (open && saved.data) setRules(saved.data.map((v) => v.rule));
    if (!open) {
      setRules(null);
      setSelected([]);
    }
  }, [open, saved.data]);

  const counts = useQuery({
    queryKey: ["rule-counts", projectId, rules],
    queryFn: () => api.countRuleMatches(projectId, rules ?? []),
    enabled: inDesktop && open && rules !== null,
    placeholderData: keepPreviousData,
  });

  const dirty = useMemo(() => {
    const before = saved.data?.map((v) => v.rule) ?? [];
    return rules !== null && JSON.stringify(before) !== JSON.stringify(rules);
  }, [saved.data, rules]);

  const close = () => {
    if (!dirty) return onClose();
    modal.confirm({
      title: t.exclusions.discardTitle,
      okText: t.common.ok,
      cancelText: t.common.cancel,
      onOk: onClose,
    });
  };

  const addRule = (child: DirChild) => {
    const rule: ExclusionRule = {
      relative_path: child.path,
      entry_type: child.is_dir ? "directory" : "file",
      is_system_default: false,
      enabled: true,
    };
    if (rules?.some((r) => ruleKey(r) === ruleKey(rule))) {
      message.warning(t.exclusions.already);
      return;
    }
    setRules([...(rules ?? []), rule]);
  };

  const save = async () => {
    if (!rules) return;
    setSaving(true);
    try {
      await api.saveExclusionRules(projectId, rules);
      await saved.refetch();
      message.success(t.exclusions.saved);
      onSaved();
    } catch (e) {
      message.error(errorMessage(e));
    } finally {
      setSaving(false);
    }
  };

  const columns: TableColumnsType<ExclusionRule> = [
    {
      title: t.exclusions.path,
      dataIndex: "relative_path",
      ellipsis: true,
      render: (p: string, r) => (
        <Space>
          {r.entry_type === "directory" ? <FolderOutlined /> : r.entry_type === "file" ? <FileOutlined /> : null}
          <Typography.Text>{p}</Typography.Text>
        </Space>
      ),
    },
    {
      title: t.exclusions.type,
      dataIndex: "entry_type",
      width: 170,
      render: (_, r) =>
        r.is_system_default ? (
          <Tooltip title={t.exclusions.systemTip}>
            <Tag>{t.exclusions.typeSystem}</Tag>
          </Tooltip>
        ) : r.entry_type === "directory" ? (
          t.exclusions.typeDirectory
        ) : (
          t.exclusions.typeFile
        ),
    },
    {
      title: t.exclusions.matches,
      key: "count",
      width: 110,
      align: "right",
      render: (_, _r, i) => counts.data?.[i] ?? "…",
    },
    {
      title: t.exclusions.enabled,
      key: "enabled",
      width: 70,
      render: (_, r, i) =>
        r.is_system_default ? (
          <Switch
            size="small"
            checked={r.enabled}
            onChange={(v) => setRules((rules ?? []).map((x, j) => (j === i ? { ...x, enabled: v } : x)))}
          />
        ) : null,
    },
  ];

  return (
    <Drawer
      title={t.exclusions.title}
      open={open}
      onClose={close}
      size={720}
      maskClosable={false}
      extra={
        <Space>
          <Button onClick={close}>{t.common.cancel}</Button>
          <Button type="primary" loading={saving} disabled={!dirty} onClick={save}>
            {t.exclusions.save}
          </Button>
        </Space>
      }
    >
      <Flex vertical gap={12}>
        <Alert type="info" showIcon title={t.exclusions.desc} />
        <Space>
          <Button icon={<PlusOutlined />} onClick={() => setPicking(true)}>
            {t.exclusions.add}
          </Button>
          <Button
            icon={<DeleteOutlined />}
            disabled={selected.length === 0}
            onClick={() => {
              setRules((rules ?? []).filter((r) => !selected.includes(ruleKey(r))));
              setSelected([]);
            }}
          >
            {t.exclusions.removeSelected}
          </Button>
        </Space>
        <Table
          rowKey={ruleKey}
          size="small"
          loading={saved.isLoading}
          columns={columns}
          dataSource={rules ?? []}
          pagination={false}
          rowSelection={{
            selectedRowKeys: selected,
            onChange: (keys) => setSelected(keys as string[]),
            getCheckboxProps: (r) => ({ disabled: r.is_system_default }),
          }}
        />
      </Flex>
      <PickModal open={picking} projectId={projectId} onClose={() => setPicking(false)} onPick={addRule} />
    </Drawer>
  );
}

interface Node extends TreeDataNode {
  child: DirChild;
}

function toNodes(children: DirChild[]): Node[] {
  return children.map((c) => ({
    key: c.path,
    title: c.excluded ? <Typography.Text type="secondary">{c.name}</Typography.Text> : c.name,
    icon: c.is_dir ? <FolderOutlined /> : <FileOutlined />,
    isLeaf: !c.is_dir,
    child: c,
  }));
}

/** 在项目目录树中选择要排除的文件或文件夹（按需逐级加载）。 */
function PickModal({
  open,
  projectId,
  onClose,
  onPick,
}: {
  open: boolean;
  projectId: string;
  onClose: () => void;
  onPick: (c: DirChild) => void;
}) {
  const { message } = App.useApp();
  const [tree, setTree] = useState<Node[]>([]);

  useEffect(() => {
    if (!open) return;
    api
      .listWorkspaceDir(projectId, null)
      .then((c) => setTree(toNodes(c)))
      .catch((e) => message.error(errorMessage(e)));
  }, [open, projectId, message]);

  const attach = (nodes: Node[], key: React.Key, kids: Node[]): Node[] =>
    nodes.map((n) =>
      n.key === key
        ? { ...n, children: kids }
        : n.children
          ? { ...n, children: attach(n.children as Node[], key, kids) }
          : n,
    );

  return (
    <Modal title={t.exclusions.pickTitle} open={open} onCancel={onClose} footer={null} width={560} destroyOnHidden>
      <Typography.Paragraph type="secondary">{t.exclusions.pickHint}</Typography.Paragraph>
      <div style={{ maxHeight: 420, overflow: "auto" }}>
        <Tree<Node>
          showIcon
          blockNode
          treeData={tree}
          loadData={async (node) => {
            if (node.children || !node.child.is_dir) return;
            const kids = await api.listWorkspaceDir(projectId, node.child.path);
            setTree((cur) => attach(cur, node.key, toNodes(kids)));
          }}
          onSelect={(_, { node }) => {
            onPick(node.child);
            onClose();
          }}
        />
      </div>
    </Modal>
  );
}
