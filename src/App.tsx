import { useState, useCallback, useEffect } from 'react';
import { useDevices } from './hooks/useDevices';
import { useTransfer } from './hooks/useTransfer';
import { DeviceList } from './components/DeviceList';
import { DropZone } from './components/DropZone';
import { TransferModal } from './components/TransferModal';
import { AcceptPopup } from './components/AcceptPopup';
import type { Device } from './types';

export default function App() {
  const devices = useDevices();
  const { incoming, activeTransfer, lastComplete, lastError, sendFile, respondTransfer } =
    useTransfer();
  const [selectedDevice, setSelectedDevice] = useState<Device | null>(null);

  const handleFilesDropped = useCallback(
    (device: Device, filePaths: string[]) => {
      for (const path of filePaths) {
        sendFile(device, path).catch(console.error);
      }
    },
    [sendFile]
  );

  useEffect(() => {
    if (selectedDevice && !devices.some((d) => d.name === selectedDevice.name)) {
      setSelectedDevice(null);
    }
  }, [devices, selectedDevice]);

  return (
    <div className="bg-gray-900 text-white min-h-screen flex flex-col select-none">
      {/* Header */}
      <header className="flex items-center gap-2 px-4 py-3 border-b border-gray-800">
        <span className="text-lg font-bold tracking-tight">DropLink</span>
        <span className="ml-auto text-xs text-gray-500">
          {devices.length} appareil{devices.length !== 1 ? 's' : ''}
        </span>
      </header>

      {/* Body */}
      <main className="flex-1 flex flex-col gap-4 p-4 overflow-y-auto">
        <section>
          <p className="text-xs text-gray-500 uppercase tracking-wider mb-2">
            À proximité
          </p>
          <DeviceList
            devices={devices}
            selectedDevice={selectedDevice}
            onDeviceSelect={setSelectedDevice}
          />
        </section>

        <DropZone
          selectedDevice={selectedDevice}
          onFilesDropped={handleFilesDropped}
          disabled={!!activeTransfer}
        />

        {lastComplete && (
          <div className="rounded-lg bg-green-900/40 border border-green-700 px-3 py-2 text-sm text-green-300">
            ✓ {lastComplete.file_name} — transfert terminé
          </div>
        )}

        {lastError && (
          <div className="rounded-lg bg-red-900/40 border border-red-700 px-3 py-2 text-sm text-red-300">
            ✗ {lastError}
          </div>
        )}
      </main>

      {/* Modals */}
      {activeTransfer && <TransferModal transfer={activeTransfer} />}

      {incoming && !activeTransfer && (
        <AcceptPopup
          request={incoming}
          onAccept={() => respondTransfer(incoming.transfer_id, true)}
          onRefuse={() => respondTransfer(incoming.transfer_id, false)}
        />
      )}
    </div>
  );
}
