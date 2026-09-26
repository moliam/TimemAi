export type OrderedInterimAnswer<T> = {
  item: T;
  ordinal: number;
};

export function newestInterimAnswersFirst<T>(items: readonly T[]): OrderedInterimAnswer<T>[] {
  return items.map((item, index) => ({ item, ordinal: index + 1 })).reverse();
}

import type { ResponsePreview, WebSubAnswer } from "./protocol";

/** Preview identity, not text equality, links provisional and accepted answers. */
export function interimAnswerPresentation(answers: readonly WebSubAnswer[], preview?: ResponsePreview | null) {
  const accepted = answers.map((item) => ({
    key: item.preview_attempt !== undefined && item.preview_index !== undefined
      ? `preview-${item.preview_attempt}-${item.preview_index}` : item.sub_answer_id,
    task: item.task, answer: item.answer, provisional: false,
    index: item.preview_index, ordinal: item.ordinal, createdAt: item.created_at_ms,
  })).reverse();
  const keys = new Set(accepted.map(item => item.key));
  const pending = (preview?.chat ?? []).map(item => ({
    key: `preview-${preview?.attempt}-${item.index}`, task: item.task, answer: item.answer,
    provisional: true, index: item.index, ordinal: undefined as number | undefined,
    createdAt: Infinity,
  })).reverse().filter(item => !keys.has(item.key));
  return [...pending, ...accepted];
}
