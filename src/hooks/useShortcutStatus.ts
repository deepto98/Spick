import { useCallback, useEffect, useRef, useState } from "react";
import {
  getShortcutStatus,
  requestInputMonitoringPermission,
  type NativeShortcutStatus,
} from "../lib/nativeShortcut";

const unsupportedStatus: NativeShortcutStatus = {
  optionSelected: false,
  optionListenerActive: false,
  inputMonitoringGranted: false,
  fallbackShortcut: null,
};

export function useShortcutStatus(enabled: boolean) {
  const [status, setStatus] = useState<NativeShortcutStatus | null>(
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
      let next = await getShortcutStatus();
      if (
        next.optionSelected &&
        next.inputMonitoringGranted &&
        !next.optionListenerActive
      ) {
        await requestInputMonitoringPermission();
        next = await getShortcutStatus();
      }
      if (revision === requestRevision.current) setStatus(next);
      return next;
    } catch (reason) {
      if (revision === requestRevision.current) {
        setError(`Couldn’t check the Option shortcut: ${String(reason)}`);
      }
      return null;
    } finally {
      if (revision === requestRevision.current) setPending(false);
    }
  }, [enabled]);

  const request = useCallback(async () => {
    if (!enabled) return unsupportedStatus;

    const revision = ++requestRevision.current;
    setPending(true);
    setError(null);
    try {
      await requestInputMonitoringPermission();
      const next = await getShortcutStatus();
      if (revision === requestRevision.current) setStatus(next);
      return next;
    } catch (reason) {
      if (revision === requestRevision.current) {
        setError(`Couldn’t open Input Monitoring: ${String(reason)}`);
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
