-- LFVM 初始数据库结构，对应 SRS V2.1 第 4 章表 4-1～4-10。
-- 约定：ID 为 ULID 文本；时间为 UTC 毫秒整数（LFVM-P-03）；
--       relative_path 保留原始拼写，path_key 为比较用的规范化键（见 paths.rs）。
-- 相对数据字典新增的实现字段已用【实现】标注。

-- 表 4-1 项目信息
CREATE TABLE projects (
    project_id              TEXT PRIMARY KEY,
    name                    TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    root_path               TEXT NOT NULL,
    root_key                TEXT NOT NULL,                 -- 【实现】root_path 的比较键
    case_policy             TEXT NOT NULL CHECK (case_policy IN ('sensitive', 'insensitive')), -- 【实现】加入时探测
    created_at              INTEGER NOT NULL,
    active_scheme_id        TEXT,
    default_head_version_id TEXT,
    is_registered           INTEGER NOT NULL DEFAULT 1 CHECK (is_registered IN (0, 1)),
    workspace_state         TEXT NOT NULL DEFAULT 'normal' CHECK (workspace_state IN ('normal', 'incomplete')),
    next_seq                INTEGER NOT NULL DEFAULT 1     -- 【实现】版本序号分配，排序不依赖系统时钟
);
-- 同一路径只能有一个已登记项目；已移除的项目保留，供“关联原历史”。
CREATE UNIQUE INDEX ux_projects_registered_root ON projects(root_key) WHERE is_registered = 1;
CREATE INDEX ix_projects_root ON projects(root_key);

-- 表 4-2 排除规则
CREATE TABLE exclusion_rules (
    rule_id           TEXT PRIMARY KEY,
    project_id        TEXT NOT NULL REFERENCES projects(project_id),
    relative_path     TEXT NOT NULL,
    path_key          TEXT NOT NULL,                       -- 【实现】
    entry_type        TEXT NOT NULL CHECK (entry_type IN ('file', 'directory', 'name_pattern')),
    is_system_default INTEGER NOT NULL DEFAULT 0 CHECK (is_system_default IN (0, 1)),
    enabled           INTEGER NOT NULL DEFAULT 1 CHECK (enabled IN (0, 1)),
    UNIQUE (project_id, entry_type, path_key)
);

-- 表 4-5 方案
CREATE TABLE schemes (
    scheme_id       TEXT PRIMARY KEY,
    project_id      TEXT NOT NULL REFERENCES projects(project_id),
    name            TEXT NOT NULL CHECK (length(name) BETWEEN 1 AND 100),
    name_key        TEXT NOT NULL,                         -- 【实现】小写名称，用于不区分大小写判重
    base_version_id TEXT NOT NULL,
    head_version_id TEXT NOT NULL,
    created_at      INTEGER NOT NULL,
    state           TEXT NOT NULL DEFAULT 'active' CHECK (state IN ('active', 'cleared'))
);
CREATE UNIQUE INDEX ux_schemes_active_name ON schemes(project_id, name_key) WHERE state = 'active';

-- 表 4-3 版本
CREATE TABLE versions (
    version_id               TEXT PRIMARY KEY,
    project_id               TEXT NOT NULL REFERENCES projects(project_id),
    seq                      INTEGER NOT NULL,             -- 【实现】项目内单调递增
    parent_version_id        TEXT REFERENCES versions(version_id),
    origin_scheme_id         TEXT REFERENCES schemes(scheme_id),
    name                     TEXT NOT NULL DEFAULT '' CHECK (length(name) <= 100),
    note                     TEXT NOT NULL DEFAULT '' CHECK (length(note) <= 500),
    created_at               INTEGER NOT NULL,
    file_count               INTEGER NOT NULL DEFAULT 0,
    directory_count          INTEGER NOT NULL DEFAULT 0,
    added_count              INTEGER NOT NULL DEFAULT 0,
    modified_count           INTEGER NOT NULL DEFAULT 0,
    deleted_count            INTEGER NOT NULL DEFAULT 0,
    exclusion_rules_snapshot TEXT NOT NULL,                -- JSON
    excluded_paths_observed  TEXT NOT NULL DEFAULT '[]',   -- JSON
    payload_state            TEXT NOT NULL DEFAULT 'available' CHECK (payload_state IN ('available', 'cleared')),
    cleared_at               INTEGER,
    UNIQUE (project_id, seq)
);
CREATE INDEX ix_versions_parent ON versions(parent_version_id);

-- 表 4-4 版本文件
CREATE TABLE version_files (
    version_id    TEXT NOT NULL REFERENCES versions(version_id),
    path_key      TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    name_key      TEXT NOT NULL,                           -- 【实现】小写文件名，供搜索
    ext_key       TEXT NOT NULL DEFAULT '',                -- 【实现】小写扩展名，供搜索
    entry_type    TEXT NOT NULL CHECK (entry_type IN ('file', 'directory')),
    size          INTEGER,
    content_hash  TEXT,
    PRIMARY KEY (version_id, path_key)
) WITHOUT ROWID;
CREATE INDEX ix_version_files_hash ON version_files(content_hash);
CREATE INDEX ix_version_files_path ON version_files(path_key);

-- 表 4-8 操作记录
CREATE TABLE operations (
    operation_id        TEXT PRIMARY KEY,
    project_id          TEXT NOT NULL REFERENCES projects(project_id),
    request_id          TEXT NOT NULL,
    type                TEXT NOT NULL CHECK (type IN ('save', 'restore', 'switch', 'expand', 'export', 'clear')),
    target_ref          TEXT NOT NULL,
    resolved_version_id TEXT,
    status              TEXT NOT NULL DEFAULT 'running' CHECK (status IN ('running', 'succeeded', 'cancelled', 'failed', 'incomplete')),
    phase               TEXT NOT NULL DEFAULT 'planned' CHECK (phase IN ('planned', 'protecting', 'executing', 'committing')),
    retry_of            TEXT REFERENCES operations(operation_id),
    resolution          TEXT NOT NULL DEFAULT 'open' CHECK (resolution IN ('open', 'resolved')),
    created_at          INTEGER NOT NULL,
    finished_at         INTEGER,
    result_message      TEXT NOT NULL DEFAULT '' CHECK (length(result_message) <= 500),
    UNIQUE (project_id, request_id)                        -- 重复提交不重复执行（LFVM-Q-05）
);
CREATE INDEX ix_operations_project ON operations(project_id, created_at);

-- 表 4-6 安全备份
CREATE TABLE safety_backups (
    backup_id           TEXT PRIMARY KEY,
    project_id          TEXT NOT NULL REFERENCES projects(project_id),
    operation_id        TEXT NOT NULL REFERENCES operations(operation_id),
    reason              TEXT NOT NULL CHECK (length(reason) BETWEEN 1 AND 200),
    created_at          INTEGER NOT NULL,
    status              TEXT NOT NULL DEFAULT 'creating' CHECK (status IN ('creating', 'ready', 'failed', 'cleared')),
    expected_file_count INTEGER NOT NULL DEFAULT 0,
    cleared_at          INTEGER
);
CREATE INDEX ix_backups_project ON safety_backups(project_id, created_at);

-- 表 4-7 备份文件
CREATE TABLE backup_files (
    backup_id     TEXT NOT NULL REFERENCES safety_backups(backup_id),
    path_key      TEXT NOT NULL,
    relative_path TEXT NOT NULL,
    name_key      TEXT NOT NULL,                           -- 【实现】
    ext_key       TEXT NOT NULL DEFAULT '',                -- 【实现】
    entry_type    TEXT NOT NULL CHECK (entry_type IN ('file', 'directory')),
    size          INTEGER,
    content_hash  TEXT,
    PRIMARY KEY (backup_id, path_key)
) WITHOUT ROWID;
CREATE INDEX ix_backup_files_hash ON backup_files(content_hash);

-- 表 4-9 操作明细
CREATE TABLE operation_items (
    operation_id    TEXT NOT NULL REFERENCES operations(operation_id),
    path_key        TEXT NOT NULL,
    relative_path   TEXT NOT NULL,
    action          TEXT NOT NULL CHECK (action IN ('create', 'replace', 'delete', 'keep')),
    entry_type      TEXT NOT NULL DEFAULT 'file' CHECK (entry_type IN ('file', 'directory')), -- 【实现】
    before_hash     TEXT,
    after_hash      TEXT,
    item_state      TEXT NOT NULL DEFAULT 'pending' CHECK (item_state IN ('pending', 'applying', 'done', 'failed', 'skipped')),
    backup_file_ref TEXT,
    error_code      TEXT,
    error_message   TEXT,
    PRIMARY KEY (operation_id, path_key)
) WITHOUT ROWID;

-- 表 4-10 内容对象（按项目分目录存放；ref_count 可由清单重新计算）
CREATE TABLE content_objects (
    project_id   TEXT NOT NULL REFERENCES projects(project_id),
    content_hash TEXT NOT NULL,
    byte_size    INTEGER NOT NULL,
    state        TEXT NOT NULL DEFAULT 'ready' CHECK (state IN ('ready', 'pending_gc')),
    ref_count    INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (project_id, content_hash)
) WITHOUT ROWID;

-- 【实现】扫描缓存：大小、修改时间、文件标识都未变的文件不重新计算哈希（LFVM-P-05、P-12）
CREATE TABLE scan_cache (
    project_id   TEXT NOT NULL REFERENCES projects(project_id),
    path_key     TEXT NOT NULL,
    size         INTEGER NOT NULL,
    mtime_ns     INTEGER NOT NULL,
    file_id      TEXT,
    content_hash TEXT NOT NULL,
    PRIMARY KEY (project_id, path_key)
) WITHOUT ROWID;
