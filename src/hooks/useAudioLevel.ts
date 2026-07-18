import { useEffect, useMemo, useState } from "react";
import { hasNativeRuntime } from "../lib/nativeDictation";
import {
  subscribeToAudioLevel,
  type AudioLevelFrame,
} from "../lib/nativeAudio";

export function useAudioLevel(active: boolean) {
  const native = useMemo(() => hasNativeRuntime(), []);
  const [frame, setFrame] = useState<AudioLevelFrame | null>(null);

  useEffect(() => {
    if (!native) return;

    let disposed = false;
    let unsubscribe: (() => void) | undefined;

    const connect = async () => {
      try {
        const stopListening = await subscribeToAudioLevel((nextFrame) => {
          if (!disposed) setFrame(nextFrame);
        });
        if (disposed) {
          stopListening();
          return;
        }
        unsubscribe = stopListening;
      } catch {
        if (!disposed) setFrame(null);
      }
    };

    void connect();
    return () => {
      disposed = true;
      unsubscribe?.();
    };
  }, [native]);

  return active ? frame : null;
}
