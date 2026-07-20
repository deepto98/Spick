import { useCallback, useEffect, useRef, useState } from "react";
import {
  getMicrophonePermissionStatus,
  requestMicrophonePermission,
  type MicrophonePermissionStatus,
} from "../lib/nativeMicrophone";

const unsupportedStatus: MicrophonePermissionStatus = {
  state: "unsupported",
  canRequest: false,
};

export function useMicrophonePermission(enabled: boolean) {
  const [status, setStatus] = useState<MicrophonePermissionStatus | null>(
    enabled ? null : unsupportedStatus,
  );
  const [pending, setPending] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const requestRevision = useRef(0);

  const refresh = useCallback(async () => {
    if (!enabled) {
      requestRevision.current += 1;
      setStatus(unsupportedStatus);
      setError(null);
      return unsupportedStatus;
    }

    const revision = ++requestRevision.current;
    setPending(true);
    setError(null);
    try {
      const next = await getMicrophonePermissionStatus();
      if (revision === requestRevision.current) setStatus(next);
      return next;
    } catch (reason) {
      if (revision === requestRevision.current) {
        setError(`Couldn’t check microphone access: ${String(reason)}`);
      }
      return null;
    } finally {
      if (revision === requestRevision.current) setPending(false);
    }
  }, [enabled]);

  const request = useCallback(async () => {
    if (!enabled) {
      requestRevision.current += 1;
      setStatus(unsupportedStatus);
      return unsupportedStatus;
    }

    const revision = ++requestRevision.current;
    setPending(true);
    setError(null);
    try {
      const next = await requestMicrophonePermission();
      if (revision === requestRevision.current) setStatus(next);
      return next;
    } catch (reason) {
      if (revision === requestRevision.current) {
        setError(`Couldn’t request microphone access: ${String(reason)}`);
      }
      return null;
    } finally {
      if (revision === requestRevision.current) setPending(false);
    }
  }, [enabled]);

  useEffect(() => {
    const timeout = window.setTimeout(() => void refresh(), 0);
    return () => window.clearTimeout(timeout);
  }, [refresh]);

  useEffect(() => {
    if (!enabled) return;
    const checkWhenVisible = () => {
      if (document.visibilityState === "visible") void refresh();
    };
    window.addEventListener("focus", checkWhenVisible);
    document.addEventListener("visibilitychange", checkWhenVisible);
    return () => {
      window.removeEventListener("focus", checkWhenVisible);
      document.removeEventListener("visibilitychange", checkWhenVisible);
    };
  }, [enabled, refresh]);

  return { error, pending, refresh, request, status };
}
