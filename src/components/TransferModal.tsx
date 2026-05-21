import type { ActiveTransfer } from '../hooks/useTransfer';

interface Props {
  transfer: ActiveTransfer;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function TransferModal({ transfer }: Props) {
  const pct = transfer.fileSize > 0
    ? Math.round((transfer.bytesDone / transfer.fileSize) * 100)
    : 0;

  const label = transfer.direction === 'sending' ? 'Envoi en cours' : 'Réception en cours';

  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-gray-800 rounded-2xl p-6 w-72 shadow-2xl">
        <p className="text-xs text-gray-400 uppercase tracking-wider mb-1">{label}</p>
        <p className="text-sm font-medium text-white mb-4 truncate">
          {transfer.fileName || 'Fichier en cours…'}
        </p>

        <div className="w-full bg-gray-700 rounded-full h-2 mb-2">
          <div
            className="bg-blue-500 h-2 rounded-full transition-all duration-100"
            style={{ width: `${pct}%` }}
          />
        </div>

        <div className="flex justify-between text-xs text-gray-400">
          <span>{formatBytes(transfer.bytesDone)}</span>
          <span>{pct}%</span>
          <span>{formatBytes(transfer.fileSize)}</span>
        </div>
      </div>
    </div>
  );
}
