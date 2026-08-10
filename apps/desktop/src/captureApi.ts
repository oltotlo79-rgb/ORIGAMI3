import type { RefObject } from "react";
import { TECHNIQUE_LABEL } from "./lib/techniques";
import { useAppStore } from "./store/appStore";

export type CaptureView = "3d" | "cp" | "both" | "normal";

export interface CaptureStepInfo {
  number: number;
  name: string;
}

export interface CaptureDocumentInfo {
  version: 1;
  stepCount: number;
  steps: CaptureStepInfo[];
}

export interface Origami3CaptureApi {
  readonly version: 1;
  openDocument(path: string): Promise<CaptureDocumentInfo>;
  getDocumentInfo(): CaptureDocumentInfo;
  goToStep(step: number): Promise<CaptureStepInfo>;
  setView(view: CaptureView): Promise<void>;
  waitForStable(): Promise<void>;
}

declare global {
  interface Window {
    /** WebView2/CDPからだけ使う、解説画像の自動撮影口。 */
    __origami3Capture?: Origami3CaptureApi;
  }
}

interface FitRefs {
  fit2d: RefObject<(() => void) | null>;
  fit3d: RefObject<(() => void) | null>;
}

const CAPTURE_VIEW_ATTRIBUTE = "data-origami3-capture-view";

function nextPaint(): Promise<void> {
  return new Promise((resolve) => {
    let settled = false;
    const fallback = window.setTimeout(() => {
      if (settled) return;
      settled = true;
      resolve();
    }, 100);
    window.requestAnimationFrame(() => {
      if (settled) return;
      settled = true;
      window.clearTimeout(fallback);
      resolve();
    });
  });
}

async function waitForStable(): Promise<void> {
  await document.fonts?.ready;
  // Zustand更新 -> React effect -> Three.js/canvas描画 -> compositor の順を待つ。
  await nextPaint();
  await nextPaint();
  await nextPaint();
}

function documentInfo(): CaptureDocumentInfo {
  const doc = useAppStore.getState().doc;
  if (!doc) throw new Error("作品が開かれていません");
  const steps: CaptureStepInfo[] = [
    { number: 0, name: "折る前" },
    ...doc.sequence.map((step, index) => ({
      number: index + 1,
      name: `${index + 1} ${TECHNIQUE_LABEL[step.kind]}`,
    })),
  ];
  return { version: 1, stepCount: doc.sequence.length, steps };
}

function setCaptureView(view: CaptureView): void {
  if (view === "normal") {
    document.documentElement.removeAttribute(CAPTURE_VIEW_ATTRIBUTE);
  } else {
    document.documentElement.setAttribute(CAPTURE_VIEW_ATTRIBUTE, view);
  }
}

/**
 * 通常UIには見た目も操作も足さず、CDPのRuntime.evaluateから呼ぶ口だけを公開する。
 * React StrictModeの再マウントでも、cleanupが自分のAPIだけを取り外す。
 */
export function installCaptureApi({ fit2d, fit3d }: FitRefs): () => void {
  const api: Origami3CaptureApi = {
    version: 1,

    async openDocument(path) {
      setCaptureView("normal");
      await useAppStore.getState().openDocument(path);
      const state = useAppStore.getState();
      if (state.errorMessage) throw new Error(state.errorMessage);
      await waitForStable();
      return documentInfo();
    },

    getDocumentInfo: documentInfo,

    async goToStep(step) {
      const info = documentInfo();
      if (!Number.isInteger(step) || step < 0 || step > info.stepCount) {
        throw new Error(`手順番号は0〜${info.stepCount}で指定してください: ${step}`);
      }
      await useAppStore.getState().selectStepForCapture(step);
      const state = useAppStore.getState();
      if (state.errorMessage) throw new Error(state.errorMessage);
      await waitForStable();
      return info.steps[step];
    },

    async setView(view) {
      setCaptureView(view);
      await nextPaint();
      if (view === "3d" || view === "both" || view === "normal") {
        fit3d.current?.();
      }
      if (view === "cp" || view === "both" || view === "normal") {
        fit2d.current?.();
      }
      await waitForStable();
    },

    waitForStable,
  };

  window.__origami3Capture = api;
  return () => {
    if (window.__origami3Capture === api) delete window.__origami3Capture;
    setCaptureView("normal");
  };
}
