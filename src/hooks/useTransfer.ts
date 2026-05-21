import { useEffect, useState, useCallback, useRef } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type {
  Device,
  IncomingRequest,
  TransferComplete,
  TransferFailed,
  TransferProgress,
} from '../types';

export interface ActiveTransfer {
  transferId: string;
  fileName: string;
  fileSize: number;
  bytesDone: number;
  direction: 'sending' | 'receiving';
}

export function useTransfer() {
  const [incoming, setIncoming] = useState<IncomingRequest | null>(null);
  const [activeTransfer, setActiveTransfer] = useState<ActiveTransfer | null>(null);
  const [lastComplete, setLastComplete] = useState<TransferComplete | null>(null);
  const [lastError, setLastError] = useState<string | null>(null);
  const currentFileNameRef = useRef<string>('');

  useEffect(() => {
    const unlistenIncoming = listen<IncomingRequest>('incoming-transfer', (e) => {
      currentFileNameRef.current = e.payload.file_name;
      setIncoming(e.payload);
    });

    const unlistenProgress = listen<TransferProgress>('transfer-progress', (e) => {
      const p = e.payload;
      setActiveTransfer({
        transferId: p.transfer_id,
        fileName: currentFileNameRef.current,
        fileSize: p.total,
        bytesDone: p.bytes_done,
        direction: p.direction,
      });
    });

    const unlistenComplete = listen<TransferComplete>('transfer-complete', (e) => {
      setLastComplete(e.payload);
      setLastError(null);
      setActiveTransfer(null);
      setIncoming(null);
      currentFileNameRef.current = '';
    });

    const unlistenFailed = listen<TransferFailed>('transfer-failed', (e) => {
      setLastError(e.payload.reason);
      setActiveTransfer(null);
      setIncoming(null);
    });

    return () => {
      unlistenIncoming.then((f) => f());
      unlistenProgress.then((f) => f());
      unlistenComplete.then((f) => f());
      unlistenFailed.then((f) => f());
    };
  }, []);

  const sendFile = useCallback(async (device: Device, filePath: string) => {
    setLastError(null);
    currentFileNameRef.current = filePath.split('/').pop() ?? filePath;
    try {
      await invoke('send_file_to_device', { device, filePath });
    } catch (e) {
      setLastError(String(e));
    }
  }, []);

  const respondTransfer = useCallback(
    async (transferId: string, accepted: boolean) => {
      await invoke('respond_transfer', { transferId, accepted });
      if (!accepted) setIncoming(null);
    },
    []
  );

  return { incoming, activeTransfer, lastComplete, lastError, sendFile, respondTransfer };
}
