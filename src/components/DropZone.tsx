import { useState, useCallback } from 'react';
import type { Device } from '../types';

interface Props {
  selectedDevice: Device | null;
  onFilesDropped: (device: Device, filePaths: string[]) => void;
  disabled?: boolean;
}

export function DropZone({ selectedDevice, onFilesDropped, disabled }: Props) {
  const [isDragging, setIsDragging] = useState(false);

  const handleDragOver = useCallback((e: React.DragEvent) => {
    e.preventDefault();
    if (!disabled) setIsDragging(true);
  }, [disabled]);

  const handleDragLeave = useCallback(() => {
    setIsDragging(false);
  }, []);

  const handleDrop = useCallback(
    (e: React.DragEvent) => {
      e.preventDefault();
      setIsDragging(false);
      if (!selectedDevice || disabled) return;

      // Tauri expose les chemins absolus via dataTransfer
      const paths: string[] = [];
      for (const file of Array.from(e.dataTransfer.files)) {
        // @ts-expect-error — Tauri étend File avec path
        if (file.path) paths.push(file.path);
      }
      if (paths.length > 0) {
        onFilesDropped(selectedDevice, paths);
      }
    },
    [selectedDevice, onFilesDropped, disabled]
  );

  const active = !disabled && selectedDevice;

  return (
    <div
      onDragOver={handleDragOver}
      onDragLeave={handleDragLeave}
      onDrop={handleDrop}
      className={`
        flex flex-col items-center justify-center rounded-xl border-2 border-dashed
        py-8 px-4 transition-colors text-center select-none
        ${isDragging ? 'border-blue-400 bg-blue-950/30' : 'border-gray-600'}
        ${active ? 'cursor-copy' : 'opacity-40 cursor-not-allowed'}
      `}
    >
      <div className="text-3xl mb-2">📂</div>
      {selectedDevice ? (
        <>
          <p className="text-sm text-gray-300">
            Glisse ici pour envoyer à
          </p>
          <p className="text-sm font-semibold text-white mt-1">
            {selectedDevice.name}
          </p>
        </>
      ) : (
        <p className="text-sm text-gray-500">
          Sélectionne un appareil ci-dessus
        </p>
      )}
    </div>
  );
}
