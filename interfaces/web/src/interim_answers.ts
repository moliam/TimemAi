export type OrderedInterimAnswer<T> = {
  item: T;
  ordinal: number;
};

export function newestInterimAnswersFirst<T>(items: readonly T[]): OrderedInterimAnswer<T>[] {
  return items.map((item, index) => ({ item, ordinal: index + 1 })).reverse();
}
