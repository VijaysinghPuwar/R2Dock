import { isAppError } from "../ipc/commands";
import type { AppError } from "../ipc/bindings/AppError";
import type { ErrorCode } from "../ipc/bindings/ErrorCode";

export function errorCode(value: unknown): ErrorCode | null {
  return isAppError(value) ? value.code : null;
}

export function asAppError(value: unknown): AppError | null {
  return isAppError(value) ? value : null;
}

export function isCredentialError(value: unknown): boolean {
  const code = errorCode(value);
  return code === "invalidAccessKey" || code === "signatureMismatch";
}

export function isOfflineError(value: unknown): boolean {
  const code = errorCode(value);
  return code === "network";
}

export function isCancelled(value: unknown): boolean {
  return errorCode(value) === "cancelled";
}
