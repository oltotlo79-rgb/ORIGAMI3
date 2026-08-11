import { describe, expect, it } from "vitest";
import { hexToHsv, hsvToHex, hsvToRgb, rgbToHsv, type HsvColor } from "./colorPicker";

describe("RGBとHSVの変換", () => {
  it.each([
    ["赤", [255, 0, 0], { h: 0, s: 100, v: 100 }],
    ["黄", [255, 255, 0], { h: 60, s: 100, v: 100 }],
    ["緑", [0, 255, 0], { h: 120, s: 100, v: 100 }],
    ["水色", [0, 255, 255], { h: 180, s: 100, v: 100 }],
    ["青", [0, 0, 255], { h: 240, s: 100, v: 100 }],
    ["紫", [255, 0, 255], { h: 300, s: 100, v: 100 }],
  ] as const)("代表色の%sを相互変換できる", (_name, rgb, hsv) => {
    expect(rgbToHsv([...rgb])).toEqual(hsv);
    expect(hsvToRgb(hsv)).toEqual(rgb);
  });

  it.each([
    [[0, 0, 0], { h: 0, s: 0, v: 0 }],
    [[255, 255, 255], { h: 0, s: 0, v: 100 }],
    [[128, 128, 128], { h: 0, s: 0, v: 50 }],
  ] as const)("無彩色では色相を0として扱う", (rgb, hsv) => {
    expect(rgbToHsv([...rgb])).toEqual(hsv);
    expect(hsvToRgb(hsv)).toEqual(rgb);
  });

  it("HSVは整数へ丸めても元のRGBに戻せる精度を保つ", () => {
    const rgb: [number, number, number] = [12, 34, 56];
    const hsv = rgbToHsv(rgb);

    expect(hsv).toEqual({ h: 210, s: 79, v: 22 });
    expect(hsvToRgb(hsv)).toEqual(rgb);
  });

  it("範囲外のRGBを0〜255へ収める", () => {
    expect(rgbToHsv([-10, 300, 0])).toEqual({ h: 120, s: 100, v: 100 });
  });
});

describe("HSV入力の正規化", () => {
  it.each([
    [{ h: 360, s: 100, v: 100 }, [255, 0, 0]],
    [{ h: -120, s: 100, v: 100 }, [0, 0, 255]],
    [{ h: 720, s: 100, v: 100 }, [255, 0, 0]],
  ] as const)("色相を0〜359度へ循環させる", (hsv, rgb) => {
    expect(hsvToRgb(hsv)).toEqual(rgb);
  });

  it("彩度と明度を0〜100へ収める", () => {
    expect(hsvToRgb({ h: 0, s: 120, v: 120 })).toEqual([255, 0, 0]);
    expect(hsvToRgb({ h: 0, s: -10, v: 50 })).toEqual([128, 128, 128]);
    expect(hsvToRgb({ h: 0, s: 100, v: -10 })).toEqual([0, 0, 0]);
  });
});

describe("16進色とHSVの変換", () => {
  it.each([
    ["#ff0000", { h: 0, s: 100, v: 100 }],
    ["00ff00", { h: 120, s: 100, v: 100 }],
    ["  #0000FF  ", { h: 240, s: 100, v: 100 }],
    ["#808080", { h: 0, s: 0, v: 50 }],
  ] as const)("%sを読める", (hex, hsv) => {
    expect(hexToHsv(hex)).toEqual(hsv);
  });

  it.each(["", "#fff", "#gg0000", "#1234567"])("不正な%sはnullにする", (hex) => {
    expect(hexToHsv(hex)).toBeNull();
  });

  it("HSVを小文字の6桁へ丸めて書き出す", () => {
    const hsv: HsvColor = { h: 210, s: 79, v: 22 };
    expect(hsvToHex(hsv)).toBe("#0c2238");
    expect(hsvToHex({ h: 0, s: 100, v: 100 })).toBe("#ff0000");
  });
});
