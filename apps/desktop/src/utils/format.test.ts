import { describe, expect, it } from "vitest";
import { formatBytes, versionLabel } from "./format";

describe("formatBytes", () => {
  it("小于 1 KB 显示字节", () => {
    expect(formatBytes(0)).toBe("0 字节");
    expect(formatBytes(1023)).toBe("1023 字节");
  });
  it("换算单位", () => {
    expect(formatBytes(1536)).toBe("1.5 KB");
    expect(formatBytes(100 * 1024 * 1024)).toBe("100 MB");
    expect(formatBytes(1024 ** 3)).toBe("1.0 GB");
  });
});

describe("versionLabel", () => {
  it("以 V 加序号显示", () => {
    expect(versionLabel(12)).toBe("V12");
  });
});
