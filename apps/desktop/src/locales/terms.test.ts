import { describe, expect, it } from "vitest";
import { zhCN } from "./zh-CN";

/** 表 8-2 “不使用的术语”。按整词、不区分大小写匹配。 */
const FORBIDDEN = [
  "commit",
  "snapshot",
  "branch",
  "HEAD",
  "checkout",
  "reset",
  "revert",
  "working tree",
  "log",
  "grep",
];

function collectStrings(value: unknown, path: string, out: Array<[string, string]>): void {
  if (typeof value === "string") {
    out.push([path, value]);
  } else if (typeof value === "function") {
    // 带参数的文案：用示例参数展开后检查
    out.push([path, String((value as (...a: string[]) => unknown)("x", "x", "x"))]);
  } else if (value && typeof value === "object") {
    for (const [k, v] of Object.entries(value)) collectStrings(v, `${path}.${k}`, out);
  }
}

describe("界面用语（LFVM-USR-0032、表 8-2）", () => {
  const strings: Array<[string, string]> = [];
  collectStrings(zhCN, "zhCN", strings);

  it("收集到界面文字", () => {
    expect(strings.length).toBeGreaterThan(10);
  });

  it.each(FORBIDDEN)("不出现“%s”", (term) => {
    const re = new RegExp(`\\b${term.replace(" ", "\\s+")}\\b`, "i");
    const hits = strings.filter(([, s]) => re.test(s)).map(([p]) => p);
    expect(hits).toEqual([]);
  });
});
