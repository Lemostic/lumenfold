import { Star } from "@phosphor-icons/react";

interface RatingProps {
  value: 0 | 1 | 2 | 3 | 4 | 5;
  size?: 12 | 14;
}

/**
 * 评分只读展示。写入评分属于 M2,在那之前不放一个点了没反应的控件。
 * 实心与空心星形本身就承载信息,不依赖颜色单独表达,
 * 浅色主题与色觉差异下都成立。
 */
export function Rating({ value, size = 12 }: RatingProps) {
  const stars = [1, 2, 3, 4, 5] as const;

  return (
    <span className="inline-flex items-center gap-px" aria-label={`评分 ${value} 星`}>
      {stars.map((star) => (
        <Star
          key={star}
          size={size}
          weight={star <= value ? "fill" : "regular"}
          className={star <= value ? "text-accent" : "text-ink-3"}
        />
      ))}
    </span>
  );
}
