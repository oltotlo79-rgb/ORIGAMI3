import {
  forwardRef,
  useCallback,
  useId,
  useRef,
  type ForwardedRef,
  type InputHTMLAttributes,
  type KeyboardEvent,
} from "react";

const DEFAULT_STEP = 1;
const LARGE_STEP_MULTIPLIER = 10;
/** toFixedで安全に丸められ、画面入力としても十分な小数桁数。 */
const MAX_DECIMAL_PLACES = 15;

export interface NumberStepperProps
  extends Omit<InputHTMLAttributes<HTMLInputElement>, "type"> {
  /** 外側の配置用クラス。classNameは従来どおりinput自体へ付く。 */
  containerClassName?: string;
  /** Shiftを押しながら操作したときの増減幅。省略時はstepの10倍。 */
  largeStep?: number;
  incrementLabel?: string;
  decrementLabel?: string;
  incrementTooltip?: string;
  decrementTooltip?: string;
  /** ボタンまたは矢印キーによる値変更を通知し終えた直後に呼ぶ。 */
  onStepComplete?: () => void;
}

function finiteNumber(value: number | string | undefined): number | null {
  if (value === undefined || value === "") return null;
  const parsed = typeof value === "number" ? value : Number(value);
  return Number.isFinite(parsed) ? parsed : null;
}

function positiveStep(value: number | string | undefined): number {
  const parsed = finiteNumber(value);
  return parsed !== null && parsed > 0 ? parsed : DEFAULT_STEP;
}

function decimalPlaces(value: number): number {
  if (!Number.isFinite(value)) return 0;
  const [coefficient, exponentText] = Math.abs(value)
    .toString()
    .toLowerCase()
    .split("e");
  const fractionLength = coefficient.split(".")[1]?.length ?? 0;
  const exponent = exponentText === undefined ? 0 : Number(exponentText);
  return Math.max(0, fractionLength - exponent);
}

/** 0.1 + 0.2のような二進浮動小数点由来の表示誤差を取り除く。 */
function addWithoutFloatingPointNoise(current: number, delta: number): number {
  const places = Math.min(
    MAX_DECIMAL_PLACES,
    Math.max(decimalPlaces(current), decimalPlaces(delta)),
  );
  const next = Number((current + delta).toFixed(places));
  return Object.is(next, -0) ? 0 : next;
}

function clamp(value: number, minimum: number | null, maximum: number | null) {
  let next = value;
  if (minimum !== null) next = Math.max(minimum, next);
  if (maximum !== null) next = Math.min(maximum, next);
  return Object.is(next, -0) ? 0 : next;
}

/** Reactの値追跡を通して、手入力と同じonInput/onChangeを発火させる。 */
function publishInputValue(input: HTMLInputElement, value: number) {
  const prototype = Object.getPrototypeOf(input) as HTMLInputElement;
  const nativeSetter = Object.getOwnPropertyDescriptor(prototype, "value")?.set;
  if (nativeSetter) {
    nativeSetter.call(input, String(value));
  } else {
    input.value = String(value);
  }
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

function assignRef<T>(ref: ForwardedRef<T>, value: T | null) {
  if (typeof ref === "function") {
    ref(value);
  } else if (ref) {
    ref.current = value;
  }
}

function NumberStepperComponent(
  {
    containerClassName,
    className,
    largeStep,
    incrementLabel,
    decrementLabel,
    incrementTooltip,
    decrementTooltip,
    onStepComplete,
    disabled = false,
    readOnly = false,
    min,
    max,
    step,
    id,
    onKeyDown,
    "aria-label": ariaLabel,
    ...inputProps
  }: NumberStepperProps,
  forwardedRef: ForwardedRef<HTMLInputElement>,
) {
  const generatedId = useId();
  const inputId = id ?? generatedId;
  const inputRef = useRef<HTMLInputElement | null>(null);
  const baseStep = positiveStep(step);
  const requestedLargeStep = finiteNumber(largeStep);
  const resolvedLargeStep =
    requestedLargeStep !== null && requestedLargeStep > 0
      ? requestedLargeStep
      : baseStep * LARGE_STEP_MULTIPLIER;
  const fieldLabel = ariaLabel?.trim() || "数値";
  const resolvedIncrementLabel = incrementLabel ?? `${fieldLabel}を増やす`;
  const resolvedDecrementLabel = decrementLabel ?? `${fieldLabel}を減らす`;
  const resolvedIncrementTooltip =
    incrementTooltip ??
    `${fieldLabel}を${baseStep}ずつ増やします。Shiftを押しながらで${resolvedLargeStep}ずつ増やします`;
  const resolvedDecrementTooltip =
    decrementTooltip ??
    `${fieldLabel}を${baseStep}ずつ減らします。Shiftを押しながらで${resolvedLargeStep}ずつ減らします`;
  const controlsDisabled = disabled || readOnly;

  const setInputRef = useCallback(
    (node: HTMLInputElement | null) => {
      inputRef.current = node;
      assignRef(forwardedRef, node);
    },
    [forwardedRef],
  );

  const changeBy = (direction: -1 | 1, useLargeStep: boolean) => {
    const input = inputRef.current;
    if (!input || controlsDisabled) return;

    const current = finiteNumber(input.value) ?? 0;
    const amount = useLargeStep ? resolvedLargeStep : baseStep;
    const minimum = finiteNumber(min);
    const maximum = finiteNumber(max);
    const next = clamp(
      addWithoutFloatingPointNoise(current, direction * amount),
      minimum,
      maximum,
    );
    if (next === current) return;
    publishInputValue(input, next);
    onStepComplete?.();
  };

  const handleInputKeyDown = (event: KeyboardEvent<HTMLInputElement>) => {
    onKeyDown?.(event);
    if (event.defaultPrevented) return;
    if (event.key !== "ArrowUp" && event.key !== "ArrowDown") return;

    // ブラウザ標準の増減と重なって2回変わらないよう、同じ経路へ一本化する。
    event.preventDefault();
    if (controlsDisabled) return;
    changeBy(event.key === "ArrowUp" ? 1 : -1, event.shiftKey);
  };

  const wrapperClassName = ["number-stepper", containerClassName]
    .filter(Boolean)
    .join(" ");
  const inputClassName = ["number-stepper-input", className]
    .filter(Boolean)
    .join(" ");

  return (
    <span
      className={wrapperClassName}
      data-number-stepper=""
      data-disabled={disabled || undefined}
      data-read-only={readOnly || undefined}
    >
      <input
        {...inputProps}
        ref={setInputRef}
        id={inputId}
        type="number"
        className={inputClassName}
        aria-label={ariaLabel}
        disabled={disabled}
        readOnly={readOnly}
        min={min}
        max={max}
        step={step}
        onKeyDown={handleInputKeyDown}
      />
      <span className="number-stepper-controls">
        <button
          type="button"
          className="number-stepper-button number-stepper-button-increment"
          aria-label={resolvedIncrementLabel}
          aria-controls={inputId}
          data-tooltip={resolvedIncrementTooltip}
          disabled={controlsDisabled}
          tabIndex={0}
          onClick={(event) => changeBy(1, event.shiftKey)}
        >
          <span aria-hidden="true">▲</span>
        </button>
        <button
          type="button"
          className="number-stepper-button number-stepper-button-decrement"
          aria-label={resolvedDecrementLabel}
          aria-controls={inputId}
          data-tooltip={resolvedDecrementTooltip}
          disabled={controlsDisabled}
          tabIndex={0}
          onClick={(event) => changeBy(-1, event.shiftKey)}
        >
          <span aria-hidden="true">▼</span>
        </button>
      </span>
    </span>
  );
}

export const NumberStepper = forwardRef<
  HTMLInputElement,
  NumberStepperProps
>(NumberStepperComponent);

NumberStepper.displayName = "NumberStepper";
