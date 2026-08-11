import {
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
  type CSSProperties,
  type KeyboardEvent as ReactKeyboardEvent,
  type PointerEvent as ReactPointerEvent,
} from "react";
import { createPortal } from "react-dom";
import {
  hexToHsv,
  hsvToHex,
  type HsvColor,
} from "../lib/colorPicker";
import { placeFloatingUi } from "../lib/floatingUi";
import { hexToRgb, rgbToHex } from "../lib/displayPrefs";

const VIEWPORT_PADDING = 8;
const TRIGGER_GAP = 8;

function normalizedHsv(color: HsvColor): HsvColor {
  const finiteHue = Number.isFinite(color.h) ? color.h : 0;
  return {
    h: ((Math.round(finiteHue) % 360) + 360) % 360,
    s: Math.round(clamp(Number.isFinite(color.s) ? color.s : 0)),
    v: Math.round(clamp(Number.isFinite(color.v) ? color.v : 0)),
  };
}

function clamp(value: number): number {
  return Math.max(0, Math.min(100, value));
}

export function ColorPickerPopover({
  label,
  value,
  onSelect,
}: {
  label: string;
  value: string;
  onSelect: (hex: string) => void;
}) {
  const dialogId = useId();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const pickerRef = useRef<HTMLDivElement>(null);
  const saturationValueRef = useRef<HTMLDivElement>(null);
  const [open, setOpen] = useState(false);
  const [portalRoot, setPortalRoot] = useState<Element | null>(null);
  const [draft, setDraft] = useState<HsvColor>(
    () => hexToHsv(value) ?? { h: 0, s: 0, v: 0 },
  );
  const [hexText, setHexText] = useState(value.toUpperCase());
  const parsedHex = hexToHsv(hexText);

  const updateDraft = (next: HsvColor) => {
    const normalized = normalizedHsv(next);
    setDraft(normalized);
    setHexText(hsvToHex(normalized).toUpperCase());
  };

  const close = (returnFocus: boolean) => {
    setOpen(false);
    if (returnFocus) {
      window.requestAnimationFrame(() => triggerRef.current?.focus());
    }
  };

  const confirm = () => {
    const parsed = hexToRgb(hexText);
    if (!parsed) return;
    const next = rgbToHex(parsed);
    onSelect(next);
    close(true);
  };

  const openPicker = () => {
    const next = hexToHsv(value) ?? { h: 0, s: 0, v: 0 };
    setDraft(next);
    const exact = hexToRgb(value);
    setHexText((exact ? rgbToHex(exact) : hsvToHex(next)).toUpperCase());
    setPortalRoot(triggerRef.current?.closest(".app") ?? document.body);
    setOpen(true);
  };

  useLayoutEffect(() => {
    if (!open) return;
    const trigger = triggerRef.current;
    const picker = pickerRef.current;
    if (!trigger || !picker) return;

    const updatePosition = () => {
      const bounds = picker.getBoundingClientRect();
      const next = placeFloatingUi(
        trigger.getBoundingClientRect(),
        { width: bounds.width, height: bounds.height },
        { width: window.innerWidth, height: window.innerHeight },
        { padding: VIEWPORT_PADDING, gap: TRIGGER_GAP },
      );
      picker.style.left = `${next.left}px`;
      picker.style.top = `${next.top}px`;
      picker.style.visibility = "visible";
    };

    updatePosition();
    const sizeObserver =
      typeof window.ResizeObserver === "function"
        ? new window.ResizeObserver(updatePosition)
        : null;
    sizeObserver?.observe(picker);
    window.addEventListener("resize", updatePosition);
    window.addEventListener("scroll", updatePosition, true);
    return () => {
      sizeObserver?.disconnect();
      window.removeEventListener("resize", updatePosition);
      window.removeEventListener("scroll", updatePosition, true);
    };
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const focusHandle = window.requestAnimationFrame(() => {
      saturationValueRef.current?.focus();
    });
    const handlePointerDown = (event: PointerEvent) => {
      const target = event.target;
      if (!(target instanceof Node)) return;
      if (pickerRef.current?.contains(target) || triggerRef.current?.contains(target)) {
        return;
      }
      close(false);
    };
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault();
      close(true);
    };
    document.addEventListener("pointerdown", handlePointerDown, true);
    window.addEventListener("keydown", handleKeyDown);
    return () => {
      window.cancelAnimationFrame(focusHandle);
      document.removeEventListener("pointerdown", handlePointerDown, true);
      window.removeEventListener("keydown", handleKeyDown);
    };
  }, [open]);

  const setSaturationValueFromPointer = (
    event: ReactPointerEvent<HTMLDivElement>,
  ) => {
    const bounds = event.currentTarget.getBoundingClientRect();
    if (bounds.width <= 0 || bounds.height <= 0) return;
    updateDraft({
      h: draft.h,
      s: clamp(((event.clientX - bounds.left) / bounds.width) * 100),
      v: clamp(((bounds.bottom - event.clientY) / bounds.height) * 100),
    });
  };

  const handleSaturationValueKeys = (event: ReactKeyboardEvent<HTMLDivElement>) => {
    const step = event.shiftKey ? 10 : 1;
    let next: HsvColor;
    if (event.key === "ArrowLeft") next = { ...draft, s: clamp(draft.s - step) };
    else if (event.key === "ArrowRight") next = { ...draft, s: clamp(draft.s + step) };
    else if (event.key === "ArrowUp") next = { ...draft, v: clamp(draft.v + step) };
    else if (event.key === "ArrowDown") next = { ...draft, v: clamp(draft.v - step) };
    else {
      return;
    }
    event.preventDefault();
    updateDraft(next);
  };

  return (
    <>
      <button
        ref={triggerRef}
        type="button"
        className="paper-custom-color"
        aria-label={`${label}のその他の色を開く`}
        aria-haspopup="dialog"
        aria-expanded={open}
        aria-controls={open ? dialogId : undefined}
        onClick={() => (open ? close(true) : openPicker())}
      >
        <span
          className="paper-custom-color-preview"
          style={{ backgroundColor: value }}
          aria-hidden="true"
        />
        その他の色
      </button>
      {open && portalRoot &&
        createPortal(
          <div
            id={dialogId}
            ref={pickerRef}
            className="color-picker-popover"
            role="dialog"
            aria-label={`${label}の色を選ぶ`}
            data-floating-ui="color-picker"
            onKeyDown={(event) => {
              if (
                event.key !== "Enter" ||
                event.target instanceof HTMLButtonElement
              ) {
                return;
              }
              event.preventDefault();
              confirm();
            }}
            style={{
              left: VIEWPORT_PADDING,
              top: VIEWPORT_PADDING,
              visibility: "hidden",
            }}
          >
            <div className="color-picker-heading">
              <strong>{label}の色</strong>
              <button
                type="button"
                className="color-picker-close"
                aria-label={`${label}の色選びを閉じる`}
                onClick={() => close(true)}
              >
                ×
              </button>
            </div>
            <div
              ref={saturationValueRef}
              className="color-saturation-value"
              role="slider"
              tabIndex={0}
              aria-label={`${label}の彩度と明度`}
              aria-valuemin={0}
              aria-valuemax={100}
              aria-valuenow={Math.round(draft.v)}
              aria-valuetext={`彩度${Math.round(draft.s)}%、明度${Math.round(draft.v)}%`}
              style={{
                "--picker-hue": `hsl(${draft.h} 100% 50%)`,
              } as CSSProperties}
              onPointerDown={(event) => {
                event.currentTarget.setPointerCapture?.(event.pointerId);
                setSaturationValueFromPointer(event);
              }}
              onPointerMove={(event) => {
                if ((event.buttons & 1) === 1) setSaturationValueFromPointer(event);
              }}
              onKeyDown={handleSaturationValueKeys}
            >
              <span
                className="color-saturation-value-thumb"
                style={{ left: `${draft.s}%`, top: `${100 - draft.v}%` }}
                aria-hidden="true"
              />
            </div>
            <label className="color-hue-row">
              <span>色相</span>
              <input
                type="range"
                aria-label={`${label}の色相`}
                min={0}
                max={359}
                step={1}
                value={draft.h}
                onChange={(event) =>
                  updateDraft({ ...draft, h: Number(event.target.value) })
                }
              />
              <output>{Math.round(draft.h)}°</output>
            </label>
            <label className="color-hex-row">
              <span>16進数</span>
              <span className="color-picker-result" aria-hidden="true">
                <span style={{ backgroundColor: hsvToHex(draft) }} />
              </span>
              <input
                type="text"
                aria-label={`${label}の16進数の色コード`}
                value={hexText}
                aria-invalid={parsedHex === null}
                spellCheck={false}
                autoComplete="off"
                onChange={(event) => {
                  const nextText = event.target.value;
                  setHexText(nextText);
                  const next = hexToHsv(nextText);
                  if (next) setDraft(next);
                }}
              />
            </label>
            {parsedHex === null && (
              <span className="color-picker-error" role="alert">
                #に続けて0〜9・A〜Fを6桁で入力してください
              </span>
            )}
            <div className="color-picker-actions">
              <button type="button" onClick={() => close(true)}>
                取り消し
              </button>
              <button
                type="button"
                className="button-primary"
                disabled={parsedHex === null}
                onClick={confirm}
              >
                この色にする
              </button>
            </div>
            <small className="color-picker-key-help">
              矢印キーで調整・Enterで確定・Escで閉じる
            </small>
          </div>,
          portalRoot,
        )}
    </>
  );
}
