export interface ServerArticleStats {
  server_id: string;
  server_name: string;
  articles_downloaded: number;
  articles_failed: number;
  bytes_downloaded: number;
}

export interface NzbJob {
  id: string;
  name: string;
  category: string;
  status: string;
  priority: number;
  total_bytes: number;
  downloaded_bytes: number;
  file_count: number;
  files_completed: number;
  article_count: number;
  articles_downloaded: number;
  articles_failed: number;
  added_at: string;
  completed_at: string | null;
  speed_bps: number;
  error_message: string | null;
  server_stats: ServerArticleStats[];
}

export interface QueueResponse {
  jobs: NzbJob[];
  total: number;
  speed_bps: number;
  paused: boolean;
}

export interface StatusResponse {
  speed_bps: number;
  queue_size: number;
  queue_remaining_bytes: number;
  disk_free_bytes: number;
  paused: boolean;
  uptime_secs: number;
  webdav_enabled: boolean;
}

export interface HistoryEntry {
  id: string;
  name: string;
  category: string;
  status: string;
  total_bytes: number;
  downloaded_bytes: number;
  added_at: string;
  completed_at: string;
  output_dir: string;
  stages: StageResult[];
  error_message: string | null;
  server_stats: ServerArticleStats[];
  has_nzb_data: boolean;
}

export interface LogEntry {
  seq: number;
  timestamp: string;
  level: string;
  message: string;
}

export interface LogsResponse {
  entries: LogEntry[];
  latest_seq: number;
}

export interface StageResult {
  name: string;
  status: string;
  message: string | null;
  duration_secs: number;
}
