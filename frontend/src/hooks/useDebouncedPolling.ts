import { useEffect, useRef, useCallback } from 'react';

export const useDebouncedPolling = (
  callback: () => Promise<void> | void,
  interval: number,
  enabled: boolean = true,
  immediate: boolean = true
) => {
  const timeoutRef = useRef<ReturnType<typeof setTimeout>>();
  const callbackRef = useRef(callback);

  useEffect(() => {
    callbackRef.current = callback;
  }, [callback]);

  const debouncedCallback = useCallback(async () => {
    try {
      await callbackRef.current();
    } catch (error) {
      console.error('Polling error:', error);
    }
    timeoutRef.current = setTimeout(debouncedCallback, interval);
  }, [interval]);

  useEffect(() => {
    if (!enabled) {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
      return;
    }
    if (immediate) {
      debouncedCallback();
    } else {
      timeoutRef.current = setTimeout(debouncedCallback, interval);
    }
    return () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, [debouncedCallback, enabled, immediate, interval]);

  useEffect(() => {
    return () => {
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, []);
};
