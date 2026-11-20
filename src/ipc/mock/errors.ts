// Error rejections in the mock backend (T-141).
//
// The wire contract (§7.10): commands return `Result<T, AppError>`; the
// `Err` arm crosses `invoke` as a REJECTED promise whose rejection value is
// the serialized `AppError` — a plain object, not an `Error` instance. The
// mocks model exactly that: UI code branches on `code` and `params`, and
// the tests assert that shape.

import type { AppErrorWire, ErrorCode } from '../contract'

/**
 * Builds a wire-shaped `AppError`. `message_key` values are the mock's own
 * approximations (the real i18n keys are the backend's contract and arrive
 * with it); tests should assert `code` and `params`, not the message key.
 */
export function mockAppError(
  code: ErrorCode,
  messageKey: string,
  params: Record<string, string> = {},
): AppErrorWire {
  return { code, message_key: messageKey, params, details: null }
}

/** Rejects with a wire-shaped `AppError` — the mock's `Err` arm. */
export function rejectWith(
  code: ErrorCode,
  messageKey: string,
  params: Record<string, string> = {},
): Promise<never> {
  return Promise.reject(mockAppError(code, messageKey, params))
}
