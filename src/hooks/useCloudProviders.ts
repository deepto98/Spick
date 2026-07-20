import { useCallback, useEffect, useRef, useState } from "react";
import {
  activateCloudProvider,
  deleteCloudApiKey,
  listCloudProviders,
  setCloudApiKey,
  type CloudProviderId,
  type CloudProviderStatus,
} from "../lib/nativeCloud";
import type { NativeAppSettings } from "../lib/nativeSettings";

export type CloudProviderAction = "configure" | "delete" | "activate";

export interface CloudProviderPendingAction {
  provider: CloudProviderId;
  action: CloudProviderAction;
}

const providerOrder: Record<CloudProviderId, number> = {
  openAi: 0,
  xAi: 1,
  gemini: 2,
};

function sortProviders(providers: readonly CloudProviderStatus[]) {
  return [...providers].sort(
    (left, right) =>
      providerOrder[left.provider] - providerOrder[right.provider],
  );
}

function mergeProvider(
  providers: readonly CloudProviderStatus[],
  saved: CloudProviderStatus,
) {
  return sortProviders([
    ...providers.filter((provider) => provider.provider !== saved.provider),
    saved,
  ]);
}

function readableError(reason: unknown) {
  return reason instanceof Error ? reason.message : String(reason);
}

function sanitizedActivationError(reason: unknown) {
  const printable = Array.from(readableError(reason), (character) => {
    const code = character.charCodeAt(0);
    return code <= 31 || code === 127 ? " " : character;
  }).join("");
  const message = printable
    .replace(/(?:sk-|xai-)[A-Za-z0-9._-]{8,}/g, "[redacted]")
    .replace(/AIza[A-Za-z0-9_-]{8,}/g, "[redacted]")
    .replace(/Bearer\s+\S+/gi, "Bearer [redacted]")
    .replace(/\s+/g, " ")
    .trim();
  return (message || "The native app did not provide a reason.").slice(0, 240);
}

export function useCloudProviders(enabled: boolean) {
  const [providers, setProviders] = useState<CloudProviderStatus[]>([]);
  const [loading, setLoading] = useState(enabled);
  const [error, setError] = useState<string | null>(null);
  const [pending, setPending] = useState<CloudProviderPendingAction | null>(
    null,
  );
  const aliveRef = useRef(true);
  const loadRequestRef = useRef(0);
  const dataGenerationRef = useRef(0);
  const operationPendingRef = useRef(false);

  useEffect(() => {
    aliveRef.current = true;
    return () => {
      aliveRef.current = false;
    };
  }, []);

  const refresh = useCallback(async () => {
    if (!enabled) return false;
    const request = ++loadRequestRef.current;
    const generation = dataGenerationRef.current;
    setLoading(true);
    setError(null);
    try {
      const statuses = await listCloudProviders();
      if (
        aliveRef.current &&
        request === loadRequestRef.current &&
        generation === dataGenerationRef.current
      ) {
        setProviders(sortProviders(statuses));
        return true;
      }
      return false;
    } catch (reason) {
      if (
        aliveRef.current &&
        request === loadRequestRef.current &&
        generation === dataGenerationRef.current
      ) {
        setError(`Couldn’t load cloud providers: ${readableError(reason)}`);
      }
      return false;
    } finally {
      if (
        aliveRef.current &&
        request === loadRequestRef.current &&
        generation === dataGenerationRef.current
      ) {
        setLoading(false);
      }
    }
  }, [enabled]);

  useEffect(() => {
    if (!enabled) return;
    const timeout = window.setTimeout(() => void refresh(), 0);
    return () => window.clearTimeout(timeout);
  }, [enabled, refresh]);

  const beginOperation = useCallback(
    (provider: CloudProviderId, action: CloudProviderAction) => {
      if (!enabled || operationPendingRef.current) return false;
      operationPendingRef.current = true;
      ++dataGenerationRef.current;
      setLoading(false);
      setPending({ provider, action });
      setError(null);
      return true;
    },
    [enabled],
  );

  const finishOperation = useCallback(() => {
    operationPendingRef.current = false;
    if (aliveRef.current) setPending(null);
  }, []);

  const configure = useCallback(
    async (provider: CloudProviderId, apiKey: string) => {
      if (!apiKey.trim() || !beginOperation(provider, "configure"))
        return false;
      try {
        const saved = await setCloudApiKey(provider, apiKey);
        if (!aliveRef.current) return false;
        ++dataGenerationRef.current;
        setLoading(false);
        setProviders((current) => mergeProvider(current, saved));
        // The acknowledged status is already visible. Reconcile the full list
        // without extending the lifetime of the credential field value.
        void refresh();
        return true;
      } catch {
        ++dataGenerationRef.current;
        if (aliveRef.current) {
          setLoading(false);
          setError("Couldn’t save that API key. Check the key and try again.");
        }
        return false;
      } finally {
        finishOperation();
      }
    },
    [beginOperation, finishOperation, refresh],
  );

  const removeCredential = useCallback(
    async (provider: CloudProviderId) => {
      if (!beginOperation(provider, "delete")) return false;
      try {
        const saved = await deleteCloudApiKey(provider);
        if (!aliveRef.current) return false;
        ++dataGenerationRef.current;
        setLoading(false);
        setProviders((current) => mergeProvider(current, saved));
        void refresh();
        return true;
      } catch {
        ++dataGenerationRef.current;
        if (aliveRef.current) {
          setLoading(false);
          setError(
            "Couldn’t remove that saved key. Make sure the provider isn’t active, then try again.",
          );
        }
        return false;
      } finally {
        finishOperation();
      }
    },
    [beginOperation, finishOperation, refresh],
  );

  const activate = useCallback(
    async (provider: CloudProviderId): Promise<NativeAppSettings | null> => {
      if (!beginOperation(provider, "activate")) return null;
      try {
        const settings = await activateCloudProvider(provider);
        if (!aliveRef.current) return null;
        ++dataGenerationRef.current;
        setLoading(false);
        setProviders((current) =>
          current.map((status) => ({
            ...status,
            selected: status.provider === provider,
          })),
        );
        void refresh();
        return settings;
      } catch (reason) {
        ++dataGenerationRef.current;
        if (aliveRef.current) {
          setLoading(false);
          setError(
            `Couldn’t activate that cloud provider: ${sanitizedActivationError(reason)}`,
          );
        }
        return null;
      } finally {
        finishOperation();
      }
    },
    [beginOperation, finishOperation, refresh],
  );

  const clearSelectedProvider = useCallback(() => {
    setProviders((current) =>
      current.map((provider) =>
        provider.selected ? { ...provider, selected: false } : provider,
      ),
    );
  }, []);

  return {
    activate,
    clearSelectedProvider,
    configure,
    error,
    loading,
    pending,
    providers,
    refresh,
    removeCredential,
  };
}
