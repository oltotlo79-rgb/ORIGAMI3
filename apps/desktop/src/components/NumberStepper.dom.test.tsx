// @vitest-environment jsdom

import { createRef, useState } from "react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { NumberStepper } from "./NumberStepper";

afterEach(cleanup);

function inputNamed(name: string) {
  return screen.getByRole("spinbutton", { name }) as HTMLInputElement;
}

describe("テーマ共通の数値上下ボタン", () => {
  it("上下ボタンで通常のstepずつ増減し、小数の表示誤差を残さない", () => {
    const values: string[] = [];
    render(
      <NumberStepper
        aria-label="丸み"
        min={0}
        max={1}
        step={0.1}
        defaultValue={0.2}
        onChange={(event) => values.push(event.currentTarget.value)}
      />,
    );

    const input = inputNamed("丸み");
    fireEvent.click(screen.getByRole("button", { name: "丸みを増やす" }));
    expect(input.value).toBe("0.3");
    fireEvent.click(screen.getByRole("button", { name: "丸みを減らす" }));
    expect(input.value).toBe("0.2");
    expect(values).toEqual(["0.3", "0.2"]);
  });

  it("Shift併用ではlargeStepを使い、省略時は通常stepの10倍にする", () => {
    render(
      <>
        <NumberStepper
          aria-label="角度"
          step={1}
          largeStep={10}
          defaultValue={5}
        />
        <NumberStepper aria-label="硬さ" step={0.2} defaultValue={1} />
      </>,
    );

    fireEvent.click(screen.getByRole("button", { name: "角度を増やす" }), {
      shiftKey: true,
    });
    expect(inputNamed("角度").value).toBe("15");
    fireEvent.click(screen.getByRole("button", { name: "硬さを減らす" }), {
      shiftKey: true,
    });
    expect(inputNamed("硬さ").value).toBe("-1");
  });

  it("上下限を越えず、端で押しても余分なchangeを発火しない", () => {
    const onChange = vi.fn();
    const onStepComplete = vi.fn();
    render(
      <NumberStepper
        aria-label="折り角度"
        min={-180}
        max={180}
        step={1}
        defaultValue={179}
        onChange={onChange}
        onStepComplete={onStepComplete}
      />,
    );

    const input = inputNamed("折り角度");
    const increase = screen.getByRole("button", { name: "折り角度を増やす" });
    fireEvent.click(increase);
    fireEvent.click(increase);
    expect(input.value).toBe("180");
    expect(onChange).toHaveBeenCalledTimes(1);
    expect(onStepComplete).toHaveBeenCalledTimes(1);

    fireEvent.change(input, { target: { value: "-179" } });
    const decrease = screen.getByRole("button", { name: "折り角度を減らす" });
    fireEvent.click(decrease);
    fireEvent.click(decrease);
    expect(input.value).toBe("-180");
    expect(onChange).toHaveBeenCalledTimes(3);
    expect(onStepComplete).toHaveBeenCalledTimes(2);
  });

  it("入力欄の上下キーも同じ幅で動き、既存onKeyDownも維持する", () => {
    const onKeyDown = vi.fn();
    const onStepComplete = vi.fn();
    render(
      <NumberStepper
        aria-label="角度"
        min={-180}
        max={180}
        step={1}
        largeStep={10}
        defaultValue={20}
        onKeyDown={onKeyDown}
        onStepComplete={onStepComplete}
      />,
    );

    const input = inputNamed("角度");
    fireEvent.keyDown(input, { key: "ArrowUp" });
    expect(input.value).toBe("21");
    fireEvent.keyDown(input, { key: "ArrowDown", shiftKey: true });
    expect(input.value).toBe("11");
    fireEvent.keyDown(input, { key: "Enter" });
    expect(onKeyDown).toHaveBeenCalledTimes(3);
    expect(onStepComplete).toHaveBeenCalledTimes(2);
  });

  it("制御入力でもonChangeを通じて親の値を更新する", () => {
    function ControlledStepper() {
      const [value, setValue] = useState(3);
      return (
        <NumberStepper
          aria-label="等分数"
          value={value}
          min={2}
          max={8}
          step={1}
          onChange={(event) => setValue(Number(event.currentTarget.value))}
        />
      );
    }

    render(<ControlledStepper />);
    fireEvent.click(screen.getByRole("button", { name: "等分数を増やす" }));
    expect(inputNamed("等分数").value).toBe("4");
    fireEvent.keyDown(inputNamed("等分数"), { key: "ArrowDown" });
    expect(inputNamed("等分数").value).toBe("3");
  });

  it("ボタンはフォーカス可能で説明を持ち、inputのrefも転送する", () => {
    const ref = createRef<HTMLInputElement>();
    render(<NumberStepper ref={ref} aria-label="紙の幅" defaultValue={150} />);

    const input = inputNamed("紙の幅");
    const increase = screen.getByRole("button", { name: "紙の幅を増やす" });
    const decrease = screen.getByRole("button", { name: "紙の幅を減らす" });
    expect(ref.current).toBe(input);
    for (const button of [increase, decrease]) {
      expect(button.getAttribute("type")).toBe("button");
      expect(button.tabIndex).toBe(0);
      expect(button.getAttribute("data-tooltip")).toContain("Shift");
      expect(button.hasAttribute("title")).toBe(false);
      expect(button.getAttribute("aria-controls")).toBe(input.id);
    }
  });

  it("disabledとreadOnlyではボタンと矢印キーのどちらでも変更しない", () => {
    const onStepComplete = vi.fn();
    render(
      <>
        <NumberStepper
          aria-label="無効"
          defaultValue={4}
          disabled
          onStepComplete={onStepComplete}
        />
        <NumberStepper
          aria-label="読み取り専用"
          defaultValue={7}
          readOnly
          onStepComplete={onStepComplete}
        />
      </>,
    );

    const disabledInput = inputNamed("無効");
    const readOnlyInput = inputNamed("読み取り専用");
    const disabledButton = screen.getByRole("button", {
      name: "無効を増やす",
      hidden: true,
    }) as HTMLButtonElement;
    const readOnlyButton = screen.getByRole("button", {
      name: "読み取り専用を増やす",
      hidden: true,
    }) as HTMLButtonElement;
    expect(disabledButton.disabled).toBe(true);
    expect(readOnlyButton.disabled).toBe(true);
    fireEvent.click(disabledButton);
    fireEvent.click(readOnlyButton);
    fireEvent.keyDown(disabledInput, { key: "ArrowUp" });
    fireEvent.keyDown(readOnlyInput, { key: "ArrowUp" });
    expect(disabledInput.value).toBe("4");
    expect(readOnlyInput.value).toBe("7");
    expect(onStepComplete).not.toHaveBeenCalled();
  });
});
