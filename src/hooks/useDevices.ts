import { useEffect, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { listen } from '@tauri-apps/api/event';
import type { Device } from '../types';

export function useDevices() {
  const [devices, setDevices] = useState<Device[]>([]);

  useEffect(() => {
    // Charger la liste initiale
    invoke<Device[]>('get_devices').then(setDevices).catch(console.error);

    // Écouter les updates
    const unlistenFound = listen<Device>('device-discovered', (event) => {
      setDevices((prev) => {
        if (prev.some((d) => d.name === event.payload.name)) return prev;
        return [...prev, event.payload];
      });
    });

    const unlistenLost = listen<string>('device-lost', (event) => {
      setDevices((prev) =>
        prev.filter((d) => event.payload !== d.name)
      );
    });

    return () => {
      unlistenFound.then((f) => f());
      unlistenLost.then((f) => f());
    };
  }, []);

  return devices;
}
