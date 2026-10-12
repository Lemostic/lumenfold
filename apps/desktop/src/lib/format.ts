/** 展示层格式化。全部返回中文界面直接可用的字符串。 */

const UNITS = ["B", "KB", "MB", "GB", "TB"];

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value >= 100 || unit <= 1 ? Math.round(value) : value.toFixed(1)} ${UNITS[unit]}`;
}

export function formatCount(value: number): string {
  return value.toLocaleString("zh-CN");
}

export function formatMegapixels(width: number, height: number): string {
  return `${((width * height) / 1_000_000).toFixed(1)} MP`;
}

const DATE_TIME = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
  hour: "2-digit",
  minute: "2-digit",
  hour12: false,
});

const DATE_ONLY = new Intl.DateTimeFormat("zh-CN", {
  year: "numeric",
  month: "2-digit",
  day: "2-digit",
});

export function formatDateTime(unixSeconds: number | null): string {
  if (unixSeconds === null) return "未知";
  return DATE_TIME.format(new Date(unixSeconds * 1000)).replace(/\//g, "-");
}

export function formatDate(unixSeconds: number): string {
  return DATE_ONLY.format(new Date(unixSeconds * 1000)).replace(/\//g, "-");
}

const WEEKDAYS = ["周日", "周一", "周二", "周三", "周四", "周五", "周六"];

export function formatDayHeading(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  return `${formatDate(unixSeconds)} ${WEEKDAYS[date.getDay()]}`;
}

const MONTHS = ["1 月", "2 月", "3 月", "4 月", "5 月", "6 月", "7 月", "8 月", "9 月", "10 月", "11 月", "12 月"];

export function formatMonthHeading(unixSeconds: number): string {
  const date = new Date(unixSeconds * 1000);
  return `${date.getFullYear()} 年 ${MONTHS[date.getMonth()]}`;
}

/** 时间线分组键：同一天的照片必须落进同一个桶 */
export function dayKey(unixSeconds: number | null): string {
  if (unixSeconds === null) return "unknown";
  const date = new Date(unixSeconds * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}-${String(date.getDate()).padStart(2, "0")}`;
}

export function monthKey(unixSeconds: number | null): string {
  if (unixSeconds === null) return "unknown";
  const date = new Date(unixSeconds * 1000);
  return `${date.getFullYear()}-${String(date.getMonth() + 1).padStart(2, "0")}`;
}

export function formatAperture(value: number | null): string {
  return value === null ? "未知" : `f/${value}`;
}

export function formatFocalLength(value: number | null): string {
  return value === null ? "未知" : `${value} mm`;
}

export function formatGps(lat: number | null, lon: number | null): string {
  if (lat === null || lon === null) return "无位置信息";
  return `${lat.toFixed(4)}, ${lon.toFixed(4)}`;
}