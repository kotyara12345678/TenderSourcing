import { useEffect, useRef, useState } from "react";

import type { TenderStatus } from "@/api/types";

export interface ProgressMessage {
  type: "snapshot" | "progress";
  tender_id?: string;
  status?: TenderStatus;
  components?: number;
  matched?: number;
  event_type?: string;
  stage?: string;
  progress?: number;
  message?: string;
  level?: string;
  task_id?: string;
  run_id?: string;
  timestamp_unix_ms?: number;
}

/**
 * Subscribes to the tender progress feed. The socket authenticates with the
 * same HttpOnly session cookie as the REST API; reconnects with exponential
 * backoff capped at 30 seconds.
 */
export function useProgressSocket(
  tenderId: string | null,
  onMessage: (message: ProgressMessage) => void,
): { connected: boolean } {
  const [connected, setConnected] = useState(false);
  const handlerRef = useRef(onMessage);
  handlerRef.current = onMessage;

  useEffect(() => {
    if (!tenderId) {
      return undefined;
    }

    let socket: WebSocket | null = null;
    let closed = false;
    let retries = 0;
    let timer: number | undefined;

    const connect = (): void => {
      const protocol = window.location.protocol === "https:" ? "wss" : "ws";
      socket = new WebSocket(
        `${protocol}://${window.location.host}/api/v1/ws/tenders/${tenderId}`,
      );

      socket.onopen = () => {
        retries = 0;
        setConnected(true);
      };

      socket.onmessage = (event: MessageEvent<string>) => {
        try {
          const parsed = JSON.parse(event.data) as ProgressMessage;
          handlerRef.current(parsed);
        } catch {
          // Malformed frame: ignore rather than crash the feed.
        }
      };

      socket.onclose = () => {
        setConnected(false);
        if (closed) return;
        retries += 1;
        const delay = Math.min(30_000, 1_000 * 2 ** Math.min(retries, 5));
        timer = window.setTimeout(connect, delay);
      };

      socket.onerror = () => {
        socket?.close();
      };
    };

    connect();

    return () => {
      closed = true;
      if (timer !== undefined) {
        window.clearTimeout(timer);
      }
      socket?.close();
    };
  }, [tenderId]);

  return { connected };
}
