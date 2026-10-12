interface SliderProps {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  valueText?: string;
  onChange(value: number): void;
}

/**
 * 原生 range 输入。用浏览器控件而不是自绘拖拽：
 * 键盘步进、屏幕阅读器数值播报、触控全都免费且正确。
 * 外观只改轨道与拇指的视觉，不改交互语义。
 */
export function Slider({ label, value, min, max, step = 1, valueText, onChange }: SliderProps) {
  const percent = ((value - min) / (max - min)) * 100;

  return (
    <label className="inline-flex items-center gap-2">
      <span className="text-xs text-ink-3">{label}</span>
      <input
        type="range"
        aria-label={label}
        min={min}
        max={max}
        step={step}
        value={value}
        onChange={(event) => onChange(Number(event.currentTarget.value))}
        className="lf-range h-1 w-28 cursor-pointer appearance-none rounded-full outline-offset-4"
        style={{
          background: `linear-gradient(to right, var(--lf-accent) ${percent}%, var(--lf-surface-3) ${percent}%)`,
        }}
      />
      {valueText ? (
        <span className="tabular w-10 text-right text-xs text-ink-2">{valueText}</span>
      ) : null}
    </label>
  );
}