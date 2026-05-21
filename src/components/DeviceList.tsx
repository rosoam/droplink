import type { Device } from '../types';

interface Props {
  devices: Device[];
  onDeviceSelect: (device: Device) => void;
  selectedDevice: Device | null;
}

export function DeviceList({ devices, onDeviceSelect, selectedDevice }: Props) {
  if (devices.length === 0) {
    return (
      <div className="flex flex-col items-center justify-center py-8 text-gray-500 text-sm">
        <div className="w-8 h-8 border-2 border-gray-600 border-t-transparent rounded-full animate-spin mb-3" />
        Recherche d'appareils à proximité…
      </div>
    );
  }

  return (
    <ul className="space-y-1">
      {devices.map((device) => {
        const isSelected = selectedDevice?.name === device.name;
        return (
          <li key={device.name}>
            <button
              onClick={() => onDeviceSelect(device)}
              className={`w-full flex items-center gap-3 px-3 py-2 rounded-lg text-left transition-colors ${
                isSelected
                  ? 'bg-blue-600 text-white'
                  : 'hover:bg-gray-700 text-gray-200'
              }`}
            >
              <span className="w-2 h-2 rounded-full bg-green-400 shrink-0" />
              <span className="text-sm font-medium truncate">{device.name}</span>
            </button>
          </li>
        );
      })}
    </ul>
  );
}
