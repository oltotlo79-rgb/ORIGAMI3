// @vitest-environment jsdom
// 操作案内の開閉部品が、狭い区画でも縦書き状に潰れたり親から出たりしないための契約。

import { afterEach, describe, expect, it } from "vitest";
import { cleanup, render } from "@testing-library/react";
import { useAppStore } from "../store/appStore";
import { CpOperationHint } from "./CpEditor/CpOperationHint";
import { OperationSteps } from "./OperationSteps";
import { PaperAppearance } from "./PaperAppearance";
import { PaperActionTip } from "./Viewer3D/PaperActionTip";
import { ViewerOperationHint } from "./Viewer3D/ViewerOperationHint";

const initialStoreState = useAppStore.getState();

afterEach(() => {
  cleanup();
  useAppStore.setState(initialStoreState, true);
});

function renderAllOperationHelp() {
  useAppStore.setState({
    activeTool: "select",
    viewerHintExpanded: false,
    cpHelpExpanded: false,
    contextHelpExpanded: false,
    paperHelpExpanded: false,
    paperActionTipVisible: true,
    paperActionTipExpanded: false,
  });
  return render(
    <>
      <ViewerOperationHint hint="紙をつかんで折れます" blocked={false} />
      <CpOperationHint />
      <OperationSteps />
      <PaperAppearance />
      <PaperActionTip />
    </>,
  );
}

describe("操作案内の狭幅レイアウト", () => {
  it("4つの開閉ボタンを縮ませず、文言を途中で折り返さない", () => {
    const { container } = renderAllOperationHelp();
    const toggles = Array.from(
      container.querySelectorAll<HTMLButtonElement>(".operation-detail-toggle"),
    );

    expect(toggles).toHaveLength(4);
    for (const toggle of toggles) expect(toggle.getAttribute("data-tooltip")).toBeTruthy();
  });

  it("畳んだ後に残る4つの要点を必ず1行に保ち、省略時は吹き出しで全文を示す", () => {
    const { container } = renderAllOperationHelp();
    const summaries = Array.from(
      container.querySelectorAll<HTMLElement>(".operation-summary-line"),
    );

    expect(summaries).toHaveLength(4);
    for (const summary of summaries)
      expect(summary.getAttribute("data-tooltip")).toBe(summary.textContent);
  });

  it("紙を選んだときの小さな吹き出しも、3D区画からはみ出さず1行を保つ", () => {
    const { container } = renderAllOperationHelp();
    const compactTip = container.querySelector<HTMLElement>(".paper-action-tip.compact");

    expect(compactTip).not.toBeNull();
    expect(compactTip!.getAttribute("data-tooltip")).toBe(
      "この紙を動かす・ふくらます案内を開きます",
    );
  });

  it("最狭3Dで紙の案内を開いたときは、現在操作を残す省スペース配置へ切り替える", () => {
    useAppStore.setState({
      activeTool: "select",
      viewerHintExpanded: true,
      paperActionTipVisible: true,
      paperActionTipExpanded: true,
    });
    const { container } = render(
      <>
        <ViewerOperationHint hint="紙をつかんで折れます" blocked={false} />
        <PaperActionTip />
      </>,
    );

    expect(container.querySelector(".paper-action-tip.expanded")).not.toBeNull();
    expect(
      container.querySelector(".viewer-operation-hint.paper-action-tip-open"),
    ).not.toBeNull();
    expect(container.querySelector(".viewer-current-action")?.textContent).toBe(
      "紙をつかんで折れます",
    );
  });
});
