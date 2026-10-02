import dayjs from "dayjs";

/** 字节数显示为 KB/MB/GB，同时保留精确字节数供提示（LFVM-P-01）。 */
export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} 字节`;
  const units = ["KB", "MB", "GB", "TB"];
  let v = bytes / 1024;
  let i = 0;
  while (v >= 1024 && i < units.length - 1) {
    v /= 1024;
    i++;
  }
  return `${v.toFixed(v >= 100 ? 0 : 1)} ${units[i]}`;
}

/** UTC 毫秒按本机时区显示（LFVM-P-03）。 */
export function formatTime(ms: number): string {
  return dayjs(ms).format("YYYY-MM-DD HH:mm:ss");
}

export function versionLabel(seq: number): string {
  return `V${seq}`;
}
