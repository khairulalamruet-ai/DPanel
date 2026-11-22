import { useState, useEffect, useCallback, useRef } from "react";
import { apiFetch } from "@builtpanel/api";
import { retryOperation, toOperation } from "./api";
import type { OperationItem } from "./types";

export interface UseOperationOptions {
  enabled?: boolean;
  pollInterval?: number; // milliseconds, default 2000
  onSuccess?: (op: OperationItem) => void;
  onError?: (err: unknown) => void;
}

export function useOperation(opId: string | null | undefined, options: UseOperationOptions = {}) {
  const { enabled = true, pollInterval = 2000, onSuccess, onError } = options;

  const onSuccessRef = useRef(onSuccess);
  onSuccessRef.current = onSuccess;
  const onErrorRef = useRef(onError);
  onErrorRef.current = onError;

  const [operation, setOperation] = useState<OperationItem | null>(null);
  const [isLoading, setIsLoading] = useState<boolean>(false);
  const [isPolling, setIsPolling] = useState<boolean>(false);
  const [error, setError] = useState<Error | null>(null);

  const timerRef = useRef<NodeJS.Timeout | null>(null);

  const fetchOperation = useCallback(async () => {
    if (!opId) return null;
    try {
      const data = toOperation(await apiFetch<Parameters<typeof toOperation>[0]>(`/api/v1/admin/operations/${opId}`));
      setOperation(data);
      return data;
    } catch (err) {
      const e = err instanceof Error ? err : new Error(String(err));
      setError(e);
      onErrorRef.current?.(e);
      return null;
    }
  }, [opId]);

  const retry = useCallback(async () => {
    if (!opId) return null;
    try {
      setIsLoading(true);
      const operation = await retryOperation(opId);
      setOperation(operation);
      return operation;
    } catch (err) {
      const e = err instanceof Error ? err : new Error(String(err));
      setError(e);
      throw e;
    } finally {
      setIsLoading(false);
    }
  }, [opId]);

  useEffect(() => {
    if (!opId || !enabled) {
      if (timerRef.current) clearTimeout(timerRef.current);
      setIsPolling(false);
      return;
    }

    let isMounted = true;
    setIsLoading(true);

    const poll = async () => {
      const data = await fetchOperation();
      if (!isMounted) return;

      setIsLoading(false);

      if (data) {
        if (data.status === "pending" || data.status === "dispatched") {
          setIsPolling(true);
          timerRef.current = setTimeout(poll, pollInterval);
        } else {
          setIsPolling(false);
          if (data.status === "succeeded") {
            onSuccessRef.current?.(data);
          }
        }
      } else {
        setIsPolling(false);
      }
    };

    poll();

    return () => {
      isMounted = false;
      if (timerRef.current) clearTimeout(timerRef.current);
    };
  }, [opId, enabled, pollInterval, fetchOperation]);

  return {
    operation,
    isLoading,
    isPolling,
    error,
    refetch: fetchOperation,
    retry,
  };
}
