-- 单文件“另存”覆盖项目外的已有文件时，也要先做安全备份（SRS 3.3.2.4 第 4 步）。
-- 这类备份中的路径相对于 external_root（用户选择的另存文件夹），而不是项目文件夹；
-- 为 NULL 时表示路径相对于项目文件夹。
ALTER TABLE safety_backups ADD COLUMN external_root TEXT;
