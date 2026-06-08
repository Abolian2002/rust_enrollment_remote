import { describe, expect, it } from "vitest";

import { formatDisplayText } from "@/components/formatted-message";

describe("formatDisplayText", () => {
  it("removes common markdown markers while preserving readable text", () => {
    const text = [
      "**1. 官方网站**",
      "* **本科招生网**：http://zsb.hrbnu.edu.cn",
      "* **电话**：0451-88067377"
    ].join("\n");

    expect(formatDisplayText(text)).toBe(
      [
        "1. 官方网站",
        "• 本科招生网：http://zsb.hrbnu.edu.cn",
        "• 电话：0451-88067377"
      ].join("\n")
    );
  });
});
