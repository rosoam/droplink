import type { IncomingRequest } from '../types';

interface Props {
  request: IncomingRequest;
  onAccept: () => void;
  onRefuse: () => void;
}

function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
}

export function AcceptPopup({ request, onAccept, onRefuse }: Props) {
  return (
    <div className="fixed inset-0 bg-black/60 flex items-center justify-center z-50">
      <div className="bg-gray-800 rounded-2xl p-6 w-72 shadow-2xl">
        <p className="text-sm font-semibold text-white mb-1">
          {request.sender_name}
        </p>
        <p className="text-xs text-gray-400 mb-4">
          veut t'envoyer un fichier
        </p>

        <div className="bg-gray-700 rounded-lg px-3 py-2 mb-5">
          <p className="text-sm text-white truncate">{request.file_name}</p>
          <p className="text-xs text-gray-400 mt-0.5">
            {formatBytes(request.file_size)}
          </p>
        </div>

        <div className="flex gap-3">
          <button
            onClick={onRefuse}
            className="flex-1 py-2 rounded-lg bg-gray-700 hover:bg-gray-600 text-sm text-gray-300 transition-colors"
          >
            Refuser
          </button>
          <button
            onClick={onAccept}
            className="flex-1 py-2 rounded-lg bg-blue-600 hover:bg-blue-500 text-sm text-white font-medium transition-colors"
          >
            Accepter
          </button>
        </div>
      </div>
    </div>
  );
}
