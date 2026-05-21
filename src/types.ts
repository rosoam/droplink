export interface Device {
  name: string;
  ip: string;
  port: number;
}

export interface TransferProgress {
  transfer_id: string;
  bytes_done: number;
  total: number;
  direction: 'sending' | 'receiving';
}

export interface IncomingRequest {
  transfer_id: string;
  sender_name: string;
  file_name: string;
  file_size: number;
}

export interface TransferComplete {
  transfer_id: string;
  file_name: string;
  saved_path: string;
}

export interface TransferFailed {
  transfer_id: string;
  reason: string;
}
